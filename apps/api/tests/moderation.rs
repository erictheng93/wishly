mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// 回傳 (user_id, session token)
use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_guard_report_and_moderation(pool: PgPool) {
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "owner").await;
    let TestUser {
        id: _,
        token: normal,
    } = user(&pool, "n").await;
    let TestUser {
        id: staff_id,
        token: staff,
    } = staff(&pool, "s").await;
    let (wid, slug, _) = claimed_wishlist(&pool, owner, false, 30, 2, 1).await;

    for uri in [
        "/admin/reports",
        "/admin/wishlists",
        "/admin/users",
        "/admin/stats",
    ] {
        let (s, _, v) = call(&pool, "GET", uri, &ck(&normal), None).await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("STAFF_ONLY")),
            "{uri}"
        );
        assert_eq!(
            call(&pool, "GET", uri, &[], None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&normal),
            Some(json!({"value": true}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // 公開檢舉
    let (s, _, r) = call(
        &pool,
        "POST",
        &format!("/public/wishlists/{slug}/reports"),
        &[],
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
            &[],
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
            &[],
            Some(json!({"reason": "scam"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let (s, _, q) = call(&pool, "GET", "/admin/reports", &ck(&staff), None).await;
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
            &ck(&staff),
            Some(json!({"moderation_status": "hidden"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&staff),
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
            &[],
            Some(json!({"reason": "scam"}))
        )
        .await
        .0,
        StatusCode::GONE
    );
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/admin/reports/{rid}"),
        &ck(&staff),
        Some(json!({"status": "dismissed"})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("INVALID_STATE_TRANSITION"))
    );
    // 復原
    let (_, _, v) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&staff),
        Some(json!({"moderation_status": "ok"})),
    )
    .await;
    assert_eq!(v["moderation_status"], "ok");
    assert!(v["moderation_reason"].is_null());

    // 搜尋 + system flag + stats
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/admin/wishlists?q={slug}"),
        &ck(&staff),
        None,
    )
    .await;
    assert_eq!(v["data"][0]["id"], wid.to_string());
    let (s, _, v) = call(
        &pool,
        "PUT",
        "/admin/system-flags/read_only",
        &ck(&staff),
        Some(json!({"value": true})),
    )
    .await;
    assert_eq!((s, &v["value"]), (StatusCode::OK, &json!(true)));
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/nope",
            &ck(&staff),
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
            &ck(&staff),
            Some(json!({"value": "x"}))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, _, v) = call(&pool, "GET", "/admin/stats", &ck(&staff), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["flags"]["read_only"], true);
    assert_eq!(v["claims_by_status"]["reserved"], 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn report_resolution_notes_and_lifetime_counts(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let staff = staff(&pool, "staff").await;
    let w = wishlist(&pool, owner.id, ListOpts::default()).await;
    let ids: Vec<Uuid> = sqlx::query_scalar("INSERT INTO content_reports(wishlist_id, reason) SELECT $1, 'other'::report_reason FROM generate_series(1,3) RETURNING id")
        .bind(w.id).fetch_all(&pool).await.unwrap();
    for (id, status) in [(ids[0], "dismissed"), (ids[1], "actioned")] {
        let uri = format!("/admin/reports/{id}");
        let (s, _, _) = call(
            &pool,
            "PATCH",
            &uri,
            &ck(&owner.token),
            Some(json!({"status":status,"note":"不允許"})),
        )
        .await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _, b) = call(
            &pool,
            "PATCH",
            &uri,
            &ck(&staff.token),
            Some(json!({"status":status,"note":"  查證後結案  "})),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert_eq!(b["resolution_note"], "查證後結案");
        let (_, _, list) = call(
            &pool,
            "GET",
            &format!("/admin/reports?status={status}"),
            &ck(&staff.token),
            None,
        )
        .await;
        assert_eq!(list["data"][0]["resolution_note"], "查證後結案");
        let (s, _, _) = call(
            &pool,
            "PATCH",
            &uri,
            &ck(&staff.token),
            Some(json!({"status":status,"note":"覆寫"})),
        )
        .await;
        assert_eq!(s, StatusCode::CONFLICT);
    }
    let uri = format!("/admin/reports/{}", ids[2]);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&staff.token),
        Some(json!({"status":"actioned","note":"字".repeat(1001)})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, _, before) = call(&pool, "GET", "/admin/wishlists", &ck(&staff.token), None).await;
    assert_eq!(before["data"][0]["open_report_count"], 1);
    assert_eq!(before["data"][0]["report_count"], 3);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/admin/wishlists/{}/moderation", w.id),
        &ck(&staff.token),
        Some(json!({"moderation_status":"hidden","reason":"違反規範"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, _, after) = call(&pool, "GET", "/admin/wishlists", &ck(&staff.token), None).await;
    assert_eq!(after["data"][0]["open_report_count"], 0);
    assert_eq!(after["data"][0]["report_count"], 3);
    let notes: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, resolution_note FROM content_reports WHERE wishlist_id=$1")
            .bind(w.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(notes.len(), 3);
    for (id, note) in notes {
        assert_eq!(
            note,
            if id == ids[2] {
                "違反規範"
            } else {
                "查證後結案"
            }
        );
    }
}
