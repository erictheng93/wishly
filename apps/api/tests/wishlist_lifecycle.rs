//! F8 清單生命週期：closed / archived 後不可增刪改品項，只能改清單資訊（流程驗證 F8）。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn closed_and_archived_freeze_items(pool: PgPool) {
    let TestUser { id: o, token: t } = user(&pool, "o").await;
    for status in ["closed", "archived"] {
        let wl = wishlist(
            &pool,
            o,
            ListOpts {
                status: Some(status),
                ..Default::default()
            },
        )
        .await;
        let a = item(&pool, wl.id, "奶瓶", 2).await;
        let b = item(&pool, wl.id, "尿布", 1).await;
        let reqs = [
            (
                "POST",
                format!("/wishlists/{}/items", wl.id),
                Some(
                    json!({"title":"新","qty_needed":1,"priority":"medium","funding_mode":"quantity"}),
                ),
            ),
            (
                "PATCH",
                format!("/items/{a}"),
                Some(json!({"title":"改名"})),
            ),
            ("DELETE", format!("/items/{a}"), None),
            ("DELETE", format!("/items/{b}?force=true"), None),
            (
                "POST",
                format!("/wishlists/{}/items/reorder", wl.id),
                Some(json!({"item_ids":[b, a]})),
            ),
        ];
        for (m, uri, body) in reqs {
            let (s, _, e) = call(&pool, m, &uri, &ck(&t), body).await;
            assert_eq!(
                (s, e["code"].as_str()),
                (StatusCode::CONFLICT, Some("WISHLIST_CLOSED")),
                "{status} {m} {uri} {e}"
            );
        }
        let alive: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM wishlist_items WHERE wishlist_id=$1 AND deleted_at IS NULL",
        )
        .bind(wl.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(alive, 2, "{status}: 品項不可被刪除");
    }
    // closed 仍可改標題（archived 只能單獨取消封存，見 wishlist_guards.rs unarchive_returns_to_closed）
    let wl = wishlist(
        &pool,
        o,
        ListOpts {
            status: Some("closed"),
            ..Default::default()
        },
    )
    .await;
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", wl.id),
        &ck(&t),
        Some(json!({"title":"結束了"})),
    )
    .await;
    assert_eq!(
        (s, b["title"].as_str()),
        (StatusCode::OK, Some("結束了")),
        "{b}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn status_transitions(pool: PgPool) {
    let TestUser { id: o, token: t } = user(&pool, "o").await;
    let go = |id, status: &str| {
        let pool = pool.clone();
        let h = ck(&t);
        let body = json!({ "status": status });
        async move { call(&pool, "PATCH", &format!("/wishlists/{id}"), &h, Some(body)).await }
    };
    // 不允許的轉換（契約 04：422 VALIDATION_FAILED，errors[].code = TRANSITION）
    for (from, to) in [
        ("draft", "closed"),
        ("active", "draft"),
        ("closed", "draft"),
    ] {
        let wl = wishlist(
            &pool,
            o,
            ListOpts {
                status: Some(from),
                ..Default::default()
            },
        )
        .await;
        item(&pool, wl.id, "奶瓶", 1).await;
        let (s, _, e) = go(wl.id, to).await;
        assert_eq!(
            (s, e["errors"][0]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("TRANSITION")),
            "{from}->{to} {e}"
        );
    }
    // 允許的轉換與同狀態冪等
    let wl = wishlist(&pool, o, ListOpts::default()).await;
    item(&pool, wl.id, "奶瓶", 1).await;
    for to in [
        "active", "closed", "closed", "active", "closed", "archived", "closed",
    ] {
        let (s, _, b) = go(wl.id, to).await;
        assert_eq!(
            (s, b["status"].as_str()),
            (StatusCode::OK, Some(to)),
            "-> {to} {b}"
        );
    }
}
