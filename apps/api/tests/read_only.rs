//! 唯讀旗標的寫入門檻。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_mode_blocks_writes(pool: PgPool) {
    let TestUser {
        id: _,
        token: staff,
    } = staff(&pool, "u").await;
    let TestUser { id: o, token: ot } = user(&pool, "u").await;
    let TestUser { id: _, token: dt } = user(&pool, "donor").await; // 登出 ot 後仍可認捐的會員
    let wl = wishlist(
        &pool,
        o,
        ListOpts {
            surprise: false,
            event_offset_days: Some(0),
            ..Default::default()
        },
    )
    .await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    assert_eq!(
        call(&pool, "GET", "/public/status", &[], None).await.2["read_only"],
        false
    );
    let (_, _, v) = call(&pool, "GET", "/admin/system-flags", &ck(&staff), None).await;
    assert_eq!(v["data"][0]["key"], "read_only");
    assert_eq!(v["data"][0]["value"], false);
    assert_eq!(
        call(&pool, "GET", "/admin/system-flags", &ck(&ot), None)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&staff),
            Some(json!({"value": true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, h, v) = call(&pool, "GET", "/public/status", &[], None).await;
    assert_eq!((s, v["read_only"].as_bool()), (StatusCode::OK, Some(true)));
    assert_eq!(h["cache-control"], "public, max-age=30");
    let (s, h, v) = call(
        &pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[ck(&dt), idem(Uuid::new_v4())].concat(),
        Some(json!({"qty":1})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("READ_ONLY_MODE"))
    );
    assert!(h.contains_key("retry-after"));
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            &ck(&ot),
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(&pool, "GET", "/me/export", &ck(&ot), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&pool, "POST", "/auth/logout", &ck(&ot), None).await.0,
        StatusCode::NO_CONTENT
    );
    // 可關閉
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&staff),
            Some(json!({"value": false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(claim_status(&pool, it, &dt).await, StatusCode::CREATED);
    assert_all_reconciled(&pool).await;
}

async fn claim_status(pool: &PgPool, it: Uuid, token: &str) -> StatusCode {
    call(
        pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[ck(token), idem(Uuid::new_v4())].concat(),
        Some(json!({"qty":1})),
    )
    .await
    .0
}
