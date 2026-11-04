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

async fn admin_reports(pool: &PgPool, tok: &str, q: &str) -> Vec<Value> {
    let (_, _, v) = call(pool, "GET", &format!("/admin/reports{q}"), &ck(tok), None).await;
    v["data"].as_array().unwrap().clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_report_snapshot_item_and_reporter(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let owner = user(&pool, "owner").await;
    let rep = user(&pool, "rep").await;
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    let other = common::wishlist(&pool, owner.id, Default::default()).await;
    let it = common::item(&pool, l.id, "原品項", 1).await;
    let foreign = common::item(&pool, other.id, "別份", 1).await;
    let url = format!("/public/wishlists/{}/reports", l.slug);

    // 匿名、不帶 item_id
    let (s, ..) = call(&pool, "POST", &url, &[], Some(json!({"reason":"scam"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    // 登入使用者 + 本清單品項
    let (_, _, a) = call(
        &pool,
        "POST",
        &url,
        &ck(&rep.token),
        Some(json!({"reason":"scam","item_id":it})),
    )
    .await;
    // 訪客 + 他清單品項
    let g: Uuid = sqlx::query_scalar(
        "INSERT INTO guests (guest_token_hash, display_name) VALUES ($1,'g') RETURNING id",
    )
    .bind(wishly_api::session::hash_token("gtok"))
    .fetch_one(&pool)
    .await
    .unwrap();
    let (s, ..) = call(
        &pool,
        "POST",
        &url,
        &common::gt("gtok"),
        Some(json!({"reason":"scam","item_id":foreign})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let (rid, ..) = (a["id"].as_str().unwrap().to_string(), ());

    // 事後編輯清單與品項、刪除品項：快照不變，item 反映現況
    sqlx::query("UPDATE wishlists SET title='改過' WHERE id=$1")
        .bind(l.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE wishlist_items SET title='改過', deleted_at=now() WHERE id=$1")
        .bind(it)
        .execute(&pool)
        .await
        .unwrap();

    let all = admin_reports(&pool, &st.token, "").await;
    assert_eq!(all.len(), 3);
    for r in &all {
        assert_eq!(r["snapshot"]["title"], "清單");
        assert_eq!(r["snapshot"]["items"][0]["title"], "原品項");
        assert_eq!(r["snapshot"]["items"].as_array().unwrap().len(), 1);
    }
    let by = |id: &str| all.iter().find(|r| r["id"] == id).unwrap().clone();
    let r = by(&rid);
    assert_eq!(r["snapshot"]["item"]["title"], "原品項");
    assert_eq!(
        r["item"],
        json!({"id": it, "title": "改過", "deleted": true})
    );
    assert_eq!(r["reporter"], "user");
    assert_eq!(r["reporter_user"]["id"], json!(rep.id));
    assert_eq!(r["reporter_stats"], json!({"total": 1, "dismissed": 0}));
    // 匿名：無 item、無檢舉者資訊
    let anon = all.iter().find(|r| r["reporter"] == "anonymous").unwrap();
    assert!(anon["item"].is_null() && anon["snapshot"]["item"].is_null());
    assert!(anon["reporter_user"].is_null() && anon["reporter_stats"].is_null());
    // 訪客 + 他清單品項：item 與快照 item 都是 null
    let gr = all.iter().find(|r| r["reporter"] == "guest").unwrap();
    assert!(
        gr["item"].is_null() && gr["snapshot"]["item"].is_null() && gr["reporter_user"].is_null()
    );
    assert_eq!(gr["reporter_stats"], json!({"total": 1, "dismissed": 0}));
    // 駁回該訪客先前的檢舉會計入 dismissed
    sqlx::query("UPDATE content_reports SET status='dismissed', handled_at=now() WHERE reporter_guest_id=$1").bind(g).execute(&pool).await.unwrap();
    let gr = &admin_reports(&pool, &st.token, "?status=dismissed").await[0];
    assert_eq!(gr["reporter_stats"], json!({"total": 1, "dismissed": 1}));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn user_report_snapshot_and_undo(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let bad = user(&pool, "bad").await;
    let rep = user(&pool, "rep").await;
    sqlx::query("UPDATE users SET handle='badguy', bio='原簡介' WHERE id=$1")
        .bind(bad.id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, ..) = call(
        &pool,
        "POST",
        &format!("/users/{}/reports", bad.id),
        &ck(&rep.token),
        Some(json!({"reason":"scam"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    sqlx::query("UPDATE users SET display_name='改名', bio='新簡介' WHERE id=$1")
        .bind(bad.id)
        .execute(&pool)
        .await
        .unwrap();

    let r = &admin_reports(&pool, &st.token, "?kind=user").await[0];
    assert_eq!(r["snapshot"]["handle"], "badguy");
    assert_eq!(r["snapshot"]["bio"], "原簡介");
    assert_ne!(r["snapshot"]["display_name"], "改名");
    assert_eq!(r["reporter_user"]["id"], json!(rep.id));
    assert_eq!(r["user"]["suspended"], false);

    // 停權後復原：同一份輸出反映回復後狀態
    let u = format!("/admin/users/{}", bad.id);
    call(
        &pool,
        "PATCH",
        &u,
        &ck(&st.token),
        Some(json!({"blocked":true,"reason":"scam"})),
    )
    .await;
    let r = &admin_reports(&pool, &st.token, "?kind=user&status=actioned").await[0];
    assert_eq!(r["user"]["suspended"], true);
    let (s, ..) = call(
        &pool,
        "PATCH",
        &u,
        &ck(&st.token),
        Some(json!({"blocked":false,"reason":"誤判"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let r = &admin_reports(&pool, &st.token, "?kind=user&status=actioned").await[0];
    assert_eq!(r["user"]["suspended"], false);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_moderation_undo_reflected(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let owner = user(&pool, "owner").await;
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    report_list(&pool, l.id).await;
    let m = format!("/admin/wishlists/{}/moderation", l.id);
    call(
        &pool,
        "PATCH",
        &m,
        &ck(&st.token),
        Some(json!({"moderation_status":"hidden","reason":"違規"})),
    )
    .await;
    let r = &admin_reports(&pool, &st.token, "?status=actioned").await[0];
    assert_eq!(r["wishlist"]["moderation_status"], "hidden");
    let (s, ..) = call(
        &pool,
        "PATCH",
        &m,
        &ck(&st.token),
        Some(json!({"moderation_status":"ok"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let r = &admin_reports(&pool, &st.token, "?status=actioned").await[0];
    assert_eq!(r["wishlist"]["moderation_status"], "ok");
    // 手動插入的舊資料沒有快照
    assert!(r["snapshot"].is_null());
}
