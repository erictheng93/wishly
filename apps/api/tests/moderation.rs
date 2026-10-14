mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn call(
    pool: &PgPool,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let h = cookie.map(common::ck).unwrap_or_default();
    let (s, _, b) = common::api(pool, method, uri, &h, body).await;
    (s, b)
}

/// 回傳 (user_id, session token)
use common::user_with_staff as user;

use common::claimed_wishlist as wishlist;

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_guard_report_and_moderation(pool: PgPool) {
    let (owner, _) = user(&pool, "owner", false).await;
    let (_, normal) = user(&pool, "n", false).await;
    let (staff_id, staff) = user(&pool, "s", true).await;
    let (wid, slug, _) = wishlist(&pool, owner, false, 30).await;

    for uri in [
        "/admin/reports",
        "/admin/wishlists",
        "/admin/users",
        "/admin/stats",
    ] {
        let (s, v) = call(&pool, "GET", uri, Some(&normal), None).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("STAFF_ONLY")),
            "{uri}"
        );
        assert_eq!(
            call(&pool, "GET", uri, None, None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            Some(&normal),
            Some(json!({"value": true}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // 公開檢舉
    let (s, r) = call(
        &pool,
        "POST",
        &format!("/public/wishlists/{slug}/reports"),
        None,
        Some(json!({"reason": "scam", "detail": "私下轉帳"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let rid = r["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/public/wishlists/{slug}/reports"),
            None,
            Some(json!({"reason": "bogus"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/public/wishlists/nonexistent/reports",
            None,
            Some(json!({"reason": "scam"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let (s, q) = call(&pool, "GET", "/admin/reports", Some(&staff), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(q["data"][0]["wishlist"]["slug"], slug.as_str());
    assert_eq!(q["data"][0]["reporter"], "anonymous");

    // 下架：reason 必填
    let uri = format!("/admin/wishlists/{wid}/moderation");
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &uri,
            Some(&staff),
            Some(json!({"moderation_status": "hidden"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, v) = call(
        &pool,
        "PATCH",
        &uri,
        Some(&staff),
        Some(json!({"moderation_status": "hidden", "reason": "疑似詐騙"})),
    )
    .await;
    assert_eq!(
        (s, v["moderation_status"].as_str()),
        (StatusCode::OK, Some("hidden"))
    );
    let (ms, rs): (String, String) = sqlx::query_as("SELECT w.moderation_status::text, r.status::text FROM wishlists w, content_reports r WHERE w.id = $1 AND r.id = $2")
        .bind(wid).bind(rid.parse::<Uuid>().unwrap()).fetch_one(&pool).await.unwrap();
    assert_eq!((ms.as_str(), rs.as_str()), ("hidden", "actioned"));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type = 'staff' AND actor_id = $1 AND action = 'wishlist.moderate' AND entity_id = $2")
        .bind(staff_id).bind(wid).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    // 下架後再檢舉 → 410；已處理檢舉再改 → 409
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/public/wishlists/{slug}/reports"),
            None,
            Some(json!({"reason": "scam"}))
        )
        .await
        .0,
        StatusCode::GONE
    );
    let (s, v) = call(
        &pool,
        "PATCH",
        &format!("/admin/reports/{rid}"),
        Some(&staff),
        Some(json!({"status": "dismissed"})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );
    // 復原
    let (_, v) = call(
        &pool,
        "PATCH",
        &uri,
        Some(&staff),
        Some(json!({"moderation_status": "ok"})),
    )
    .await;
    assert_eq!(v["moderation_status"], "ok");
    assert!(v["moderation_reason"].is_null());

    // 搜尋 + system flag + stats
    let (_, v) = call(
        &pool,
        "GET",
        &format!("/admin/wishlists?q={slug}"),
        Some(&staff),
        None,
    )
    .await;
    assert_eq!(v["data"][0]["id"], wid.to_string());
    let (s, v) = call(
        &pool,
        "PUT",
        "/admin/system-flags/read_only",
        Some(&staff),
        Some(json!({"value": true})),
    )
    .await;
    assert_eq!((s, &v["value"]), (StatusCode::OK, &json!(true)));
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/nope",
            Some(&staff),
            Some(json!({"value": true}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            Some(&staff),
            Some(json!({"value": "x"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, v) = call(&pool, "GET", "/admin/stats", Some(&staff), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["flags"]["read_only"], true);
    assert_eq!(v["claims_by_status"]["reserved"], 1);
}
