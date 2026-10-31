//! 平台審核組織 / 許可 / 撤銷。
mod common;
use axum::http::StatusCode;
use common::{call, call_app, ck, staff, user, TestUser};
use serde_json::json;
use sqlx::PgPool;
use std::time::Duration;
use wishly_api::{app, AppState};

async fn org_status(pool: &PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT verification_status FROM organizations WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn approve_requires_approved_permit(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "pending").await;
    let p = common::permit(&pool, o, "pending", 30).await;
    let uri = format!("/admin/orgs/{o}/review");
    let (s, _, b) = call(
        &pool,
        "POST",
        &uri,
        &ck(&st),
        Some(json!({"decision":"approve"})),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("PERMIT_REQUIRED")),
        "{b}"
    );
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/permits/{p}/review"),
        &ck(&st),
        Some(json!({"decision":"approve"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, _, b) = call(
        &pool,
        "POST",
        &uri,
        &ck(&st),
        Some(json!({"decision":"approve"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["verification_status"], "verified");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action IN ('org.review','permit.review')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn needs_changes_sets_flagged_fields(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "pending").await;
    let uri = format!("/admin/orgs/{o}/review");
    for bad in [json!([]), json!(["bogus"])] {
        let (s, _, b) = call(
            &pool,
            "POST",
            &uri,
            &ck(&st),
            Some(json!({"decision":"needs_changes","flagged_fields":bad})),
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    }
    let (s, _, b) = call(&pool, "POST", &uri, &ck(&st), Some(json!({"decision":"needs_changes","note":"補件","flagged_fields":["intro","documents"]}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let f: Vec<String> = sqlx::query_scalar(
        "SELECT unnest(flagged_fields) FROM organizations WHERE id=$1 ORDER BY 1",
    )
    .bind(o)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(f, ["documents", "intro"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reject_sets_resubmit_after_30d(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "pending").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st),
        Some(json!({"decision":"reject","note":"不符"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let d: i32 = sqlx::query_scalar(
        "SELECT (resubmit_after::date - now()::date)::int FROM organizations WHERE id=$1",
    )
    .bind(o)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!((29..=30).contains(&d), "{d}");
    // 已退件者不可再審
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st),
        Some(json!({"decision":"reject"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn revoke_demotes_active_sites_and_purges(pool: PgPool) {
    let cdn = common::cdn::init();
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (ev, evs) = common::event(&pool, o).await;
    let site = common::site(&pool, o, owner, ev, p, "active").await;
    let os: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st),
        Some(json!({"decision":"reject","note":"撤銷"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let status: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id=$1")
        .bind(site.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "draft");
    cdn.wait_for(&site.slug).await;
    cdn.wait_for_url(&common::cdn::event_url(&evs)).await;
    cdn.wait_for_url(&common::cdn::org_url(&os)).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn review_notifies_owners(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { id: vol, .. } = user(&pool, "vol").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "pending").await;
    common::member(&pool, o, owner, "owner").await;
    common::member(&pool, o, vol, "volunteer").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st),
        Some(json!({"decision":"reject","note":"x"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let to: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT user_id FROM notifications WHERE kind='org.review'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(to, [owner]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn non_staff_forbidden(pool: PgPool) {
    let TestUser { id: owner, token } = user(&pool, "owner").await;
    let o = common::org(&pool, owner, "pending").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&token),
        Some(json!({"decision":"reject"})),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("STAFF_ONLY"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn detail_decrypts_phone(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "pending").await;
    sqlx::query("UPDATE organizations SET contact_phone_enc=$2 WHERE id=$1")
        .bind(o)
        .bind(wishly_api::sealed::seal(b"0912345678"))
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = call(&pool, "GET", &format!("/admin/orgs/{o}"), &ck(&st), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["org"]["contact_phone"], "0912345678");
    assert!(b["org"].get("contact_phone_enc").is_none());
    let (s, _, b) = call(&pool, "GET", "/admin/orgs?status=pending", &ck(&st), None).await;
    assert_eq!(
        (s, b["data"].as_array().map(Vec::len)),
        (StatusCode::OK, Some(1)),
        "{b}"
    );
}

/// 兩位 staff 對同一筆 pending 組織同時 approve：鎖住組織列後兩邊都排隊，只有一個轉移生效。
#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_reviews_apply_once(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: s1, .. } = staff(&pool, "s1").await;
    let TestUser { token: s2, .. } = staff(&pool, "s2").await;
    let o = common::org(&pool, owner, "pending").await;
    common::permit(&pool, o, "approved", 30).await;
    let app = app(AppState { pool: pool.clone() });
    let mut gate = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(o)
        .execute(&mut *gate)
        .await
        .unwrap();
    let uri = format!("/admin/orgs/{o}/review");
    let go = |tok: String| {
        let (app, uri) = (app.clone(), uri.clone());
        tokio::spawn(async move {
            call_app(
                &app,
                "POST",
                &uri,
                &ck(&tok),
                Some(json!({"decision":"approve"})),
            )
            .await
        })
    };
    let (a, b) = (go(s1), go(s2));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock'
                   AND query LIKE '%FROM organizations WHERE id = %FOR UPDATE'")
                .fetch_one(&pool).await.unwrap();
            if n >= 2 { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("expected 2 review statements blocked on the org row lock");
    gate.commit().await.unwrap();
    let mut codes = [a.await.unwrap().0, b.await.unwrap().0];
    codes.sort();
    assert_eq!(codes, [StatusCode::OK, StatusCode::CONFLICT]);
    assert_eq!(org_status(&pool, o).await, "verified");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='org.review'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

/// 送審中的許可被退件：owner 收到帶備註的通知（P-P3）
#[sqlx::test(migrations = "../../db/migrations")]
async fn reject_pending_permit_notifies_owner(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    common::member(&pool, o, owner, "owner").await;
    let p = common::permit(&pool, o, "pending", 30).await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/permits/{p}/review"),
        &ck(&st),
        Some(json!({"decision":"reject","note":"字號不清楚"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let note: String = sqlx::query_scalar(
        "SELECT payload->>'note' FROM notifications WHERE kind='org.permit_rejected' AND user_id=$1",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(note, "字號不清楚");
    let reviewed: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='org.permit_reviewed' AND user_id=$1")
        .bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(reviewed, 0, "退件只寄 org.permit_rejected 一封");
    let (subj, body) = wishly_api::notify::render_lang(
        wishly_api::notify::Lang::Zh,
        "org.permit_rejected",
        "",
        &json!({"display_name":"協會","permit_no":"P-1","note":"字號不清楚","org_id":o}),
    );
    assert!(
        subj.contains("P-1")
            && body.contains("審核備註：字號不清楚")
            && body.contains(&format!("/orgs/{o}")),
        "{subj} {body}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn revoke_approved_permit_pauses_sites(pool: PgPool) {
    let cdn = common::cdn::init();
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    common::member(&pool, o, owner, "owner").await;
    let p = common::permit(&pool, o, "approved", 30).await;
    let (ev, _) = common::event(&pool, o).await;
    let site = common::site(&pool, o, owner, ev, p, "active").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/permits/{p}/review"),
        &ck(&st),
        Some(json!({"decision":"reject","note":"過期"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let ss: String = sqlx::query_scalar("SELECT site_status FROM wishlists WHERE id=$1")
        .bind(site.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ss, "paused");
    cdn.wait_for(&site.slug).await;
    let os: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    cdn.wait_for_url(&common::cdn::org_url(&os)).await; // 組織頁只列上線據點
    assert_eq!(org_status(&pool, o).await, "verified");
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind='org.permit_revoked' AND user_id=$1",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn org_review_copy() {
    use wishly_api::notify::{render_lang, Lang};
    let cases = [
        (json!({"decision":"verified"}), "已通過審核", "approved"),
        (
            json!({"decision":"needs_changes"}),
            "需要補件",
            "needs changes",
        ),
        (json!({"decision":"rejected"}), "未通過審核", "not approved"),
        (
            json!({"decision":"rejected","revoked":true}),
            "驗證已撤銷",
            "revoked",
        ),
    ];
    for (mut p, zh, en) in cases {
        p["display_name"] = json!("協會");
        assert!(render_lang(Lang::Zh, "org.review", "", &p).0.contains(zh));
        let (s, b) = render_lang(Lang::En, "org.review", "", &p);
        assert!(s.contains(en) && b.contains("/org/apply"), "{s}");
        // 沒有備註就不印備註行（不再出現「審核備註：-」）
        assert!(!b.contains("Review note"), "{b}");
    }
    let p = json!({"decision":"needs_changes","display_name":"協會","note":"請補統編證明"});
    assert!(render_lang(Lang::Zh, "org.review", "", &p)
        .1
        .contains("審核備註：請補統編證明"));
}

/// 已通過的組織送了新許可 → pending_permits 偽狀態列得出來，每列帶待審數量；cursor 分頁照常。
#[sqlx::test(migrations = "../../db/migrations")]
async fn list_pending_permits_filter(pool: PgPool) {
    let TestUser { id: owner, .. } = user(&pool, "owner").await;
    let TestUser { token: st, .. } = staff(&pool, "staff").await;
    let o = common::org(&pool, owner, "verified").await;
    common::permit(&pool, o, "approved", 30).await;
    let (_, _, b) = call(
        &pool,
        "GET",
        "/admin/orgs?status=pending_permits",
        &ck(&st),
        None,
    )
    .await;
    assert_eq!(b["data"].as_array().map(Vec::len), Some(0), "{b}");
    common::permit(&pool, o, "pending", 90).await;
    common::permit(&pool, o, "pending", 120).await;
    let (s, _, b) = call(
        &pool,
        "GET",
        "/admin/orgs?status=pending_permits&limit=1",
        &ck(&st),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["data"].as_array().map(Vec::len), Some(1), "{b}");
    assert_eq!(b["data"][0]["pending_permits"], 2, "{b}");
    assert!(b["next_cursor"].is_null(), "{b}");
    let (_, _, b) = call(&pool, "GET", "/admin/orgs?status=verified", &ck(&st), None).await;
    assert_eq!(b["data"][0]["pending_permits"], 2, "{b}");
}
