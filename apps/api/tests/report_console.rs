//! 檢舉後台：kind 篩選、目標資訊與 target_open_count、停權使用者連動處置檢舉。
mod common;
use axum::http::StatusCode;
use common::{call, ck, staff, user};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn report_user(pool: &PgPool, target: Uuid, status: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO content_reports (reported_user_id, reason, status, handled_at) VALUES ($1,'scam',$2::report_status, CASE WHEN $2='open' THEN NULL ELSE now() END) RETURNING id")
        .bind(target).bind(status).fetch_one(pool).await.unwrap()
}

async fn report_list(pool: &PgPool, wid: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO content_reports (wishlist_id, reason) VALUES ($1,'scam') RETURNING id",
    )
    .bind(wid)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn status_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM content_reports WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_kind_filter_enrichment_and_open_count(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let owner = user(&pool, "owner").await;
    let bad = user(&pool, "bad").await;
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    report_list(&pool, l.id).await;
    report_list(&pool, l.id).await;
    report_user(&pool, bad.id, "open").await;
    report_user(&pool, bad.id, "open").await;
    report_user(&pool, bad.id, "dismissed").await; // 不計入 open

    let get = |q: &'static str| {
        let (pool, t) = (pool.clone(), st.token.clone());
        async move { call(&pool, "GET", &format!("/admin/reports{q}"), &ck(&t), None).await }
    };
    let (s, _, all) = get("").await;
    assert_eq!(
        (s, all["data"].as_array().unwrap().len()),
        (StatusCode::OK, 4)
    );

    let (_, _, w) = get("?kind=wishlist").await;
    let w = w["data"].as_array().unwrap();
    assert_eq!(w.len(), 2);
    for r in w {
        assert!(r["user"].is_null());
        assert_eq!(r["wishlist"]["moderation_status"], "ok");
        assert_eq!(r["wishlist"]["owner"]["id"], json!(owner.id));
        assert_eq!(r["target_open_count"], 2);
    }
    let (_, _, u) = get("?kind=user").await;
    let u = u["data"].as_array().unwrap();
    assert_eq!(u.len(), 2);
    for r in u {
        assert!(r["wishlist"].is_null());
        assert_eq!(r["user"]["suspended"], false);
        assert_eq!(r["target_open_count"], 2);
    }
    // 已駁回那筆：自身不是 open，但同目標仍有 2 筆 open
    let (_, _, d) = get("?kind=user&status=dismissed").await;
    assert_eq!(d["data"][0]["target_open_count"], 2);

    assert_eq!(get("?kind=x").await.0, StatusCode::UNPROCESSABLE_ENTITY);

    // 停權後 suspended 為 true（已處置的檢舉仍帶出）
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/users/{}", bad.id),
        &ck(&st.token),
        Some(json!({"blocked":true,"reason":"scam"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, a) = get("?kind=user&status=actioned").await;
    assert_eq!(a["data"][0]["user"]["suspended"], true);
    assert_eq!(a["data"][0]["target_open_count"], 0);

    // PATCH 回應沿用同一份 SQL，欄位齊全
    let rid = report_user(&pool, owner.id, "open").await;
    let (s, _, h): (_, _, Value) = call(
        &pool,
        "PATCH",
        &format!("/admin/reports/{rid}"),
        &ck(&st.token),
        Some(json!({"status":"dismissed"})),
    )
    .await;
    assert_eq!(
        (s, h["status"].as_str(), h["target_open_count"].as_i64()),
        (StatusCode::OK, Some("dismissed"), Some(0))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn suspending_user_actions_only_their_open_reports(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let bad = user(&pool, "bad").await;
    let other = user(&pool, "other").await;
    let open = report_user(&pool, bad.id, "open").await;
    let dismissed = report_user(&pool, bad.id, "dismissed").await;
    let others = report_user(&pool, other.id, "open").await;

    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/users/{}", bad.id),
        &ck(&st.token),
        Some(json!({"blocked":true,"reason":"違規"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(status_of(&pool, open).await, "actioned");
    let (by, note): (Option<Uuid>, Option<String>) =
        sqlx::query_as("SELECT handled_by, resolution_note FROM content_reports WHERE id=$1")
            .bind(open)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((by, note.as_deref()), (Some(st.id), Some("違規")));
    assert_eq!(
        status_of(&pool, dismissed).await,
        "dismissed",
        "已駁回的檢舉不可被停權覆寫"
    );
    assert_eq!(status_of(&pool, others).await, "open");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn hiding_list_still_actions_list_reports(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let owner = user(&pool, "owner").await;
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    let r = report_list(&pool, l.id).await;
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/wishlists/{}/moderation", l.id),
        &ck(&st.token),
        Some(json!({"moderation_status":"hidden","reason":"違規"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(status_of(&pool, r).await, "actioned");
}
