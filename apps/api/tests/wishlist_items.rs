//! 擁有者權限、品項與庫存規則。
mod common;
use axum::http::{header, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

/// 固定測試碼，不依賴 SMTP 或暴力枚舉。

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_only_and_item_rules(pool: PgPool) {
    let app = test_app(&pool);
    let TestUser { id: _, token: a } = user(&pool, "t").await;
    let TestUser { id: _, token: b } = user(&pool, "t").await;
    assert_eq!(call_app(&app, "GET", "/wishlists", &[], None).await.0, 401);

    let (st, h, w) = call_app(
        &app,
        "POST",
        "/wishlists",
        &ck(&a),
        Some(json!({ "type": "registry", "title": "寶寶清單" })),
    )
    .await;
    assert_eq!(st, 201, "{w}");
    let slug = w["slug"].as_str().unwrap();
    assert!(slug.len() == 10 && slug.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_eq!(w["status"], "draft");
    let wid = w["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.get(header::LOCATION).unwrap().to_str().unwrap(),
        format!("/api/v1/wishlists/{wid}")
    );
    assert_eq!(
        call_app(
            &app,
            "POST",
            "/wishlists",
            &ck(&a),
            Some(json!({ "type": "relief", "title": "x" }))
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call_app(
            &app,
            "POST",
            "/wishlists",
            &ck(&a),
            Some(json!({ "type": "personal", "title": "x", "surprise_mode": true }))
        )
        .await
        .0,
        422
    );

    // 空清單不可發佈
    let (st, _, e) = call_app(
        &app,
        "PATCH",
        &format!("/wishlists/{wid}"),
        &ck(&a),
        Some(json!({ "status": "active" })),
    )
    .await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("WISHLIST_NOT_PUBLISHABLE"))
    );

    let (st, _, it) = call_app(
        &app,
        "POST",
        &format!("/wishlists/{wid}/items"),
        &ck(&a),
        Some(json!({ "title": "奶瓶", "qty_needed": 3 })),
    )
    .await;
    assert_eq!(st, 201, "{it}");
    let iid = it["id"].as_str().unwrap().to_string();
    assert_eq!(it["sort_order"], 10);
    assert_eq!(
        call_app(
            &app,
            "POST",
            &format!("/wishlists/{wid}/items"),
            &ck(&a),
            Some(json!({ "title": "x", "funding_mode": "crowdfund" }))
        )
        .await
        .0,
        422
    );

    // 他人一律 404
    let wu = format!("/wishlists/{wid}");
    let iu = format!("/items/{iid}");
    for (m, u, body) in [
        ("GET", wu.clone(), None),
        ("PATCH", wu.clone(), Some(json!({ "title": "hack" }))),
        ("DELETE", wu.clone(), None),
        ("GET", format!("{wu}/dashboard"), None),
        ("POST", format!("{wu}/items"), Some(json!({ "title": "x" }))),
        ("PATCH", iu.clone(), Some(json!({ "title": "hack" }))),
        ("DELETE", iu.clone(), None),
        (
            "POST",
            format!("{wu}/items/reorder"),
            Some(json!({ "item_ids": [iid] })),
        ),
    ] {
        let (st, _, e) = call_app(&app, m, &u, &ck(&b), body).await;
        assert_eq!(
            (st, e["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("NOT_FOUND")),
            "{m} {u}"
        );
    }
    let (_, _, l) = call_app(&app, "GET", "/wishlists", &ck(&b), None).await;
    assert!(l["data"].as_array().unwrap().is_empty());
    // 擁有者標題未被改動
    let (_, _, got) = call_app(&app, "GET", &wu, &ck(&a), None).await;
    assert_eq!(got["wishlist"]["title"], "寶寶清單");
    assert_eq!(got["items"][0]["qty_claimed"], 0);

    // qty_needed 不得低於已認領
    sqlx::query("UPDATE wishlist_items SET qty_claimed=2 WHERE id=$1")
        .bind(Uuid::parse_str(&iid).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO claims (item_id, user_id, claimer_name, qty) VALUES ($1, NULL, 'g', 2)",
    )
    .bind(Uuid::parse_str(&iid).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    let (st, _, e) = call_app(
        &app,
        "PATCH",
        &iu,
        &ck(&a),
        Some(json!({ "qty_needed": 1 })),
    )
    .await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("QTY_BELOW_CLAIMED"))
    );
    assert_eq!(
        call_app(
            &app,
            "PATCH",
            &iu,
            &ck(&a),
            Some(json!({ "qty_needed": 2, "priority": "high" }))
        )
        .await
        .0,
        200
    );

    // 有認領的品項刪除需 force
    let (st, _, e) = call_app(&app, "DELETE", &iu, &ck(&a), None).await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("ITEM_HAS_CLAIMS"))
    );
    // 重排：集合不符 → 422；相符 → 204
    let (_, _, it2) = call_app(
        &app,
        "POST",
        &format!("{wu}/items"),
        &ck(&a),
        Some(json!({ "title": "第二個" })),
    )
    .await;
    let i2 = it2["id"].as_str().unwrap();
    assert_eq!(
        call_app(
            &app,
            "POST",
            &format!("{wu}/items/reorder"),
            &ck(&a),
            Some(json!({ "item_ids": [iid] }))
        )
        .await
        .0,
        422
    );
    assert_eq!(
        call_app(
            &app,
            "POST",
            &format!("{wu}/items/reorder"),
            &ck(&a),
            Some(json!({ "item_ids": [i2, iid] }))
        )
        .await
        .0,
        204
    );
    let (_, _, got) = call_app(&app, "GET", &wu, &ck(&a), None).await;
    assert_eq!(got["items"][0]["id"], i2);
    assert_eq!(
        call_app(&app, "DELETE", &format!("{iu}?force=true"), &ck(&a), None)
            .await
            .0,
        204
    );

    // 發佈 → 關閉 → 修改品項被拒；DELETE 清單 = 封存且冪等
    assert!(!call_app(
        &app,
        "PATCH",
        &wu,
        &ck(&a),
        Some(json!({ "status": "active" }))
    )
    .await
    .1
    .is_empty());
    let (_, _, w) = call_app(&app, "GET", &wu, &ck(&a), None).await;
    assert_eq!(w["wishlist"]["status"], "active");
    assert_eq!(
        call_app(
            &app,
            "PATCH",
            &wu,
            &ck(&a),
            Some(json!({ "status": "closed" }))
        )
        .await
        .0,
        200
    );
    let (st, _, e) = call_app(
        &app,
        "PATCH",
        &format!("/items/{i2}"),
        &ck(&a),
        Some(json!({ "title": "x" })),
    )
    .await;
    assert_eq!(
        (st, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("WISHLIST_CLOSED"))
    );
    assert_eq!(call_app(&app, "DELETE", &wu, &ck(&a), None).await.0, 204);
    assert_eq!(call_app(&app, "DELETE", &wu, &ck(&a), None).await.0, 204);
    let (_, _, l) = call_app(
        &app,
        "GET",
        "/wishlists?status=archived&limit=5",
        &ck(&a),
        None,
    )
    .await;
    assert_eq!(l["data"][0]["status"], "archived");
    assert_all_reconciled(&pool).await;
}
