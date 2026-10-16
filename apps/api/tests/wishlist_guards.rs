mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use common::call;
use common::ck;

/// 清單 + 一個 qty_needed=3 的品項；event_offset_days>0 且 surprise → 鎖定
async fn list(pool: &PgPool, owner: Uuid, surprise: bool, off: i32) -> (Uuid, Uuid) {
    let wid = common::wishlist(
        pool,
        owner,
        common::ListOpts {
            surprise,
            event_offset_days: Some(off),
            ..Default::default()
        },
    )
    .await
    .id;
    let it = common::item(pool, wid, "奶瓶", 3).await;
    (wid, it)
}

/// 訪客認領；回傳 (guest_token, claim_id)
async fn claim(pool: &PgPool, item: Uuid, email: Option<&str>) -> (String, Uuid) {
    let (s, _, b) = call(
        pool,
        "POST",
        &format!("/items/{item}/claims"),
        &[("idempotency-key", Uuid::new_v4().to_string())],
        Some(json!({ "qty": 1, "display_name": "小明", "email": email })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    (
        b["guest_token"].as_str().unwrap().into(),
        b["claim"]["id"].as_str().unwrap().parse().unwrap(),
    )
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_lock_errors_do_not_reveal_claims(pool: PgPool) {
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let (wid, claimed) = list(&pool, o, true, 30).await;
    let free: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed) VALUES ($1,'x',3) RETURNING id").bind(wid).fetch_one(&pool).await.unwrap();
    claim(&pool, claimed, None).await;
    for it in [claimed, free] {
        let (s, _, b) = call(
            &pool,
            "DELETE",
            &format!("/items/{it}?force=true"),
            &ck(&t),
            None,
        )
        .await;
        assert_eq!(
            (s.as_u16(), b["code"].as_str()),
            (403, Some("FORBIDDEN")),
            "{b}"
        );
        let (s, _, b) = call(&pool, "DELETE", &format!("/items/{it}"), &ck(&t), None).await;
        assert_eq!(
            (s.as_u16(), b["code"].as_str()),
            (403, Some("FORBIDDEN")),
            "{b}"
        );
        // 調降（無論低於或高於已認領）同樣回通用錯誤，回應完全相同
        for q in [1, 2] {
            let (s, _, b) = call(
                &pool,
                "PATCH",
                &format!("/items/{it}"),
                &ck(&t),
                Some(json!({ "qty_needed": q })),
            )
            .await;
            assert_eq!(
                (s.as_u16(), b["code"].as_str()),
                (403, Some("FORBIDDEN")),
                "{b}"
            );
        }
        // 調升與其他欄位仍可改，且不含 qty_claimed
        let (s, _, b) = call(
            &pool,
            "PATCH",
            &format!("/items/{it}"),
            &ck(&t),
            Some(json!({ "qty_needed": 5, "title": "新" })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        assert!(b["qty_claimed"].is_null());
    }
    // GET / reorder 也不洩漏
    let (_, _, g) = call(&pool, "GET", &format!("/wishlists/{wid}"), &ck(&t), None).await;
    assert!(g["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|i| i["qty_claimed"].is_null() && i["updated_at"] == i["created_at"]));
    let ids: Vec<Value> = g["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].clone())
        .collect();
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/wishlists/{wid}/items/reorder"),
        &ck(&t),
        Some(json!({ "item_ids": ids })),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unlocked_keeps_specific_errors(pool: PgPool) {
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let (_, it) = list(&pool, o, false, 30).await;
    claim(&pool, it, None).await;
    let (_, _, b) = call(
        &pool,
        "PATCH",
        &format!("/items/{it}"),
        &ck(&t),
        Some(json!({ "qty_needed": 0 })),
    )
    .await;
    assert_eq!(b["code"], "VALIDATION_FAILED");
    claim(&pool, it, None).await;
    claim(&pool, it, None).await;
    let (_, _, b) = call(
        &pool,
        "PATCH",
        &format!("/items/{it}"),
        &ck(&t),
        Some(json!({ "qty_needed": 2 })),
    )
    .await;
    assert_eq!(b["code"], "QTY_BELOW_CLAIMED");
    let (s, _, b) = call(&pool, "DELETE", &format!("/items/{it}"), &ck(&t), None).await;
    assert_eq!(
        (s.as_u16(), b["code"].as_str()),
        (409, Some("ITEM_HAS_CLAIMS"))
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn archive_is_idempotent(pool: PgPool) {
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let (wid, _) = list(&pool, o, false, 30).await;
    let uri = format!("/wishlists/{wid}");
    assert_eq!(
        call(&pool, "DELETE", &uri, &ck(&t), None).await.0,
        StatusCode::NO_CONTENT
    );
    let (_, _, a) = call(&pool, "GET", &uri, &ck(&t), None).await;
    assert_eq!(
        call(&pool, "DELETE", &uri, &ck(&t), None).await.0,
        StatusCode::NO_CONTENT
    );
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "status": "archived" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["status"], "archived");
    assert_eq!(
        b["updated_at"], a["wishlist"]["updated_at"],
        "重複封存不應推進 updated_at"
    );
    // 其他修改仍被拒
    let (s, _, b) = call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "x" }))).await;
    assert_eq!(
        (s.as_u16(), b["code"].as_str()),
        (409, Some("WISHLIST_CLOSED"))
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn optimistic_concurrency(pool: PgPool) {
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let (wid, it) = list(&pool, o, false, 30).await;
    let uri = format!("/wishlists/{wid}");
    let (_, _, g) = call(&pool, "GET", &uri, &ck(&t), None).await;
    let v0 = g["wishlist"]["updated_at"].clone();
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "title": "A", "expected_updated_at": v0 })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_ne!(b["updated_at"], v0);
    // 舊版本寫入 → 衝突且不覆蓋
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "title": "B", "expected_updated_at": v0 })),
    )
    .await;
    assert_eq!(
        (s.as_u16(), b["code"].as_str()),
        (409, Some("STALE_VERSION")),
        "{b}"
    );
    let (_, _, g) = call(&pool, "GET", &uri, &ck(&t), None).await;
    assert_eq!(g["wishlist"]["title"], "A");
    // 用新版本可成功；未帶則照舊
    let v1 = g["wishlist"]["updated_at"].clone();
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &uri,
            &ck(&t),
            Some(json!({ "title": "C", "expected_updated_at": v1 }))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&pool, "PATCH", &uri, &ck(&t), Some(json!({ "title": "D" })))
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &uri,
            &ck(&t),
            Some(json!({ "title": "E", "expected_updated_at": "bad" }))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 品項
    let (_, _, g) = call(&pool, "GET", &uri, &ck(&t), None).await;
    let iv = g["items"][0]["updated_at"].clone();
    let iu = format!("/items/{it}");
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &iu,
            &ck(&t),
            Some(json!({ "title": "1", "expected_updated_at": iv }))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &iu,
        &ck(&t),
        Some(json!({ "title": "2", "expected_updated_at": iv })),
    )
    .await;
    assert_eq!(
        (s.as_u16(), b["code"].as_str()),
        (409, Some("STALE_VERSION")),
        "{b}"
    );
    common::assert_all_reconciled(&pool).await;
}

/// closes_at：預設不設期限；設了之後到期即擋認領、sweep 轉 closed；過去時間不可設、不可帶著過期值重開
#[sqlx::test(migrations = "../../db/migrations")]
async fn closes_at_is_optional_and_enforced(pool: PgPool) {
    let common::TestUser { id: o, token: t } = common::user(&pool, "u").await;
    let (wid, item) = list(&pool, o, false, 30).await;
    let uri = format!("/wishlists/{wid}");
    let (_, _, b) = call(&pool, "GET", &uri, &ck(&t), None).await;
    assert!(b["wishlist"]["closes_at"].is_null(), "預設不設期限");

    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": "2000-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(
        (s.as_u16(), b["errors"][0]["pointer"].as_str()),
        (422, Some("/closes_at")),
        "{b}"
    );
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": "2999-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["closes_at"], "2999-01-01T00:00:00Z");

    // 繞過 API 把期限改成過去：認領立即被擋（不等 sweep），sweep 後轉 closed 並可由創建者重開
    sqlx::query("UPDATE wishlists SET closes_at = now() - interval '1 minute' WHERE id = $1")
        .bind(wid)
        .execute(&pool)
        .await
        .unwrap();
    // 已到期但尚未被 sweep：編輯其他欄位不應被擋
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "title": "改名" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "到期未掃到時改標題不該 422：{b}");
    // 前端儲存設定會原樣帶回 closes_at：值沒變就放行，改成另一個過去的時間仍擋
    let cur = b["closes_at"].clone();
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "title": "再改", "closes_at": cur })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "帶回未變動的過期 closes_at：{b}");
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "closes_at": "2001-01-01T00:00:00Z" })),
    )
    .await;
    assert_eq!(s.as_u16(), 422);
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/items/{item}/claims"),
        &[("idempotency-key", Uuid::new_v4().to_string())],
        Some(json!({ "qty": 1, "display_name": "小明" })),
    )
    .await;
    assert_eq!(
        (s.as_u16(), b["code"].as_str()),
        (409, Some("WISHLIST_CLOSED")),
        "{b}"
    );
    assert_eq!(wishly_api::wishlists::close_due(&pool).await.unwrap(), 1);
    let (_, _, b) = call(&pool, "GET", &uri, &ck(&t), None).await;
    assert_eq!(b["wishlist"]["status"], "closed");
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "status": "active" })),
    )
    .await;
    assert_eq!(s.as_u16(), 422, "過期的 closes_at 不可直接重開");
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&t),
        Some(json!({ "status": "active", "closes_at": null })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    common::assert_all_reconciled(&pool).await;
}
