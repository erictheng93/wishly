//! MICRO_CACHE（程序內短暫快取）開啟時的行為：探索頁最多晚 10 秒、read_only 旗標最多晚 1 秒，
//! 但經 admin 改旗標要立即生效。config 與快取都是全域單例，所以獨立成一個 binary、只有一個測試函式。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "../../db/migrations")]
async fn micro_cache_bounds_staleness_and_admin_flag_is_immediate(pool: PgPool) {
    wishly_api::config::init(wishly_api::config::Config {
        micro_cache: true,
        ..wishly_api::config::Config::dev()
    });
    let o = user(&pool, "主人").await;
    let wl = wishlist(&pool, o.id, ListOpts::default()).await;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let donor = user(&pool, "donor").await;

    // 探索頁：同一頁 10 秒內回快取內容
    let titles = |v: serde_json::Value| -> Vec<String> {
        v["wishlists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["title"].as_str().unwrap().to_string())
            .collect()
    };
    let (s, h, v) = call(&pool, "GET", "/public/wishlists", &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["content-type"], "application/json");
    let before = titles(v);
    assert_eq!(before.len(), 1);
    sqlx::query("UPDATE wishlists SET title = '改名後' WHERE id = $1")
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, v) = call(&pool, "GET", "/public/wishlists", &[], None).await;
    assert_eq!(titles(v), before, "10 秒內應回快取");
    // 不同的頁 / type 是不同的鍵
    let (_, _, v) = call(&pool, "GET", "/public/wishlists?type=registry", &[], None).await;
    assert_eq!(titles(v), vec!["改名後".to_string()]);

    // 營運統計：守衛每次都先跑（非營運 403），通過後 60 秒內回快取
    let adm = staff(&pool, "營運統計").await;
    let (s, _, _) = call(&pool, "GET", "/admin/stats", &ck(&donor.token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, v1) = call(&pool, "GET", "/admin/stats", &ck(&adm.token), None).await;
    assert_eq!(s, StatusCode::OK);
    user(&pool, "新使用者").await;
    let (_, _, v2) = call(&pool, "GET", "/admin/stats", &ck(&adm.token), None).await;
    assert_eq!(v2, v1, "60 秒內應回快取");
    let (s, _, _) = call(&pool, "GET", "/admin/stats", &ck(&donor.token), None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "快取不可繞過守衛");

    // read_only：直接改 DB 最多晚 1 秒生效
    assert_eq!(
        call(&pool, "GET", "/public/status", &[], None).await.2["read_only"],
        false
    );
    sqlx::query("UPDATE system_flags SET value = 'true' WHERE key = 'read_only'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&pool, "GET", "/public/status", &[], None).await.2["read_only"],
        false,
        "1 秒內應回快取"
    );
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(
        call(&pool, "GET", "/public/status", &[], None).await.2["read_only"],
        true
    );

    // 經 admin 關閉：本程序的快取立即失效，寫入馬上可用
    let staff = staff(&pool, "營運").await;
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&staff.token),
            Some(json!({"value": false}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[ck(&donor.token), idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    // 經 admin 開啟：下一個寫入立即被擋
    assert_eq!(
        call(
            &pool,
            "PUT",
            "/admin/system-flags/read_only",
            &ck(&staff.token),
            Some(json!({"value": true}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/items/{it}/claims"),
        &[ck(&user(&pool, "donor2").await.token), idem(Uuid::new_v4())].concat(),
        Some(json!({"qty": 1})),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::SERVICE_UNAVAILABLE, Some("READ_ONLY_MODE"))
    );
}
