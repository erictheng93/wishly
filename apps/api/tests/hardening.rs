//! F-01 / F-03 / F-09 / F-16 / F-17：冪等重播、集中式文字驗證、日期範圍、強制刪除善後
mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

/// active 清單 + qty_needed=5 的品項
async fn fixture(pool: &PgPool, owner: Uuid) -> (Uuid, String, Uuid) {
    let wl = wishlist(pool, owner, ListOpts::default()).await;
    let it = item(pool, wl.id, "奶瓶", 5).await;
    (wl.id, wl.slug, it)
}

async fn claim(
    pool: &PgPool,
    it: Uuid,
    key: Uuid,
    hdr: Vec<(&'static str, String)>,
    body: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut h = idem(key);
    h.extend(hdr);
    call(pool, "POST", &format!("/items/{it}/claims"), &h, Some(body)).await
}

fn field(b: &Value) -> &str {
    b["errors"][0]["pointer"].as_str().unwrap_or("")
}

// ---------- F-01 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn lost_first_response_replay_returns_same_claim(pool: PgPool) {
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "u").await;
    let (_, _, it) = fixture(&pool, owner).await;
    let me = user(&pool, "小明").await;
    let key = Uuid::new_v4();
    let body = json!({"qty": 2});
    // 首次回應「遺失」：伺服器已處理，客戶端以同 key 重送
    let (s1, _, b1) = claim(&pool, it, key, ck(&me.token), body.clone()).await;
    assert_eq!(s1, StatusCode::CREATED);
    let (s2, h2, b2) = claim(&pool, it, key, ck(&me.token), body.clone()).await;
    assert_eq!(s2, StatusCode::CREATED);
    assert_eq!(h2["idempotency-replayed"], "true");
    assert!(!h2.contains_key("set-cookie"));
    assert_eq!(b1, b2);
    // 沒有重複 claim、沒有 guest，qty_claimed 只加一次
    let (g, c, q): (i64, i64, i32) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM guests), (SELECT count(*) FROM claims), qty_claimed FROM wishlist_items WHERE id=$1",
    )
    .bind(it)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((g, c, q), (0, 1, 2));
    assert_all_reconciled(&pool).await;
}

// ---------- F-03 / F-16 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn text_validation_wishlists_and_items(pool: PgPool) {
    let TestUser { id: _, token: tok } = user(&pool, "u").await;
    let h = ck(&tok);
    for (t, why) in [
        ("a\u{0}b", "nul"),
        ("a\u{7}b", "bel"),
        ("a\u{1b}b", "esc"),
        ("a\u{202e}b", "rlo"),
        ("a\u{2066}b", "isolate"),
        ("\u{200b}\u{200d}\u{feff}", "zero-width only"),
        ("a\nb", "newline in single line"),
    ] {
        let (s, _, b) = call(
            &pool,
            "POST",
            "/wishlists",
            &h,
            Some(json!({"type": "registry", "title": t})),
        )
        .await;
        assert_eq!(
            (s, field(&b)),
            (StatusCode::UNPROCESSABLE_ENTITY, "/title"),
            "{why}"
        );
        assert_eq!(b["code"], "VALIDATION_FAILED");
    }
    // description 保留 \n \t，但拒絕 NUL / 控制字元
    let (s, _, w) = call(
        &pool,
        "POST",
        "/wishlists",
        &h,
        Some(json!({"type": "registry", "title": " 好清單 ", "description": "第一行\n\t第二行"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(
        (
            w["title"].as_str().unwrap(),
            w["description"].as_str().unwrap()
        ),
        ("好清單", "第一行\n\t第二行")
    );
    let (s, _, b) = call(
        &pool,
        "POST",
        "/wishlists",
        &h,
        Some(json!({"type": "registry", "title": "ok", "description": "x\u{0}y"})),
    )
    .await;
    assert_eq!(
        (s, field(&b)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/description")
    );
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", w["id"].as_str().unwrap()),
        &h,
        Some(json!({"title": "a\u{0}"})),
    )
    .await;
    assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/title"));

    let wid = w["id"].as_str().unwrap();
    for k in ["title", "description", "brand", "spec", "product_url"] {
        let (s, _, b) = call(
            &pool,
            "POST",
            &format!("/wishlists/{wid}/items"),
            &h,
            Some(json!({"title": "t", k: "a\u{0}b"})),
        )
        .await;
        assert_eq!(
            (s, field(&b)),
            (StatusCode::UNPROCESSABLE_ENTITY, format!("/{k}").as_str()),
            "{k}"
        );
    }
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/wishlists/{wid}/items"),
        &h,
        Some(json!({"title": "\u{200b}"})),
    )
    .await;
    assert_eq!((s, field(&b)), (StatusCode::UNPROCESSABLE_ENTITY, "/title"));
    for u in ["javascript:alert(1)", "data:text/html,x", "ftp://x.com/a"] {
        let (s, _, b) = call(
            &pool,
            "POST",
            &format!("/wishlists/{wid}/items"),
            &h,
            Some(json!({"title": "t", "product_url": u})),
        )
        .await;
        assert_eq!(
            (s, field(&b)),
            (StatusCode::UNPROCESSABLE_ENTITY, "/product_url"),
            "{u}"
        );
    }
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/wishlists/{wid}/items"),
        &h,
        Some(json!({"title": "t", "product_url": "https://example.com/a"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn text_validation_claims_reports_slug(pool: PgPool) {
    let TestUser {
        id: owner,
        token: _,
    } = user(&pool, "u").await;
    let (_, slug, it) = fixture(&pool, owner).await;
    let me = user(&pool, "小明").await;
    for (k, v) in [
        ("note", "a\u{0}b"),
        ("note", "a\u{7}b"),
        ("note", "a\u{202e}b"),
        ("note", "\u{200b}\u{200d}x\u{feff}\u{0}"),
    ] {
        let mut body = json!({"qty": 1});
        body[k] = json!(v);
        let (s, _, b) = claim(&pool, it, Uuid::new_v4(), ck(&me.token), body).await;
        assert_eq!(
            (s, field(&b)),
            (StatusCode::UNPROCESSABLE_ENTITY, format!("/{k}").as_str()),
            "{k}"
        );
    }
    // 備註可含換行
    let (s, _, _) = claim(
        &pool,
        it,
        Uuid::new_v4(),
        ck(&me.token),
        json!({"qty": 1, "note": "a\nb"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    // 失敗請求不留下 claim
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM claims")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    // 檢舉 detail
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/public/wishlists/{slug}/reports"),
        &[],
        Some(json!({"reason": "scam", "detail": "x\u{0}y"})),
    )
    .await;
    assert_eq!(
        (s, field(&b)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/detail")
    );
    // slug 含 NUL → 404 而非 500
    assert_eq!(
        call(&pool, "GET", "/public/wishlists/abc%00defghi", &[], None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/public/wishlists/abc%00defghi/reports",
            &[],
            Some(json!({"reason": "scam"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_all_reconciled(&pool).await;
}

// ---------- F-17 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn event_date_range(pool: PgPool) {
    let TestUser { id: _, token: tok } = user(&pool, "u").await;
    let h = ck(&tok);
    for d in [
        "0000-01-01",
        "-0001-01-01",
        "1999-12-31",
        "2101-01-01",
        "9999-12-31",
    ] {
        for surprise in [false, true] {
            let (s, _, b) = call(&pool, "POST", "/wishlists", &h, Some(json!({"type": "registry", "title": "t", "event_date": d, "surprise_mode": surprise}))).await;
            assert_eq!(
                (s, field(&b)),
                (StatusCode::UNPROCESSABLE_ENTITY, "/event_date"),
                "{d} {surprise}"
            );
        }
    }
    for d in ["2000-01-01", "2100-12-31"] {
        let (s, _, _) = call(
            &pool,
            "POST",
            "/wishlists",
            &h,
            Some(json!({"type": "registry", "title": "t", "event_date": d})),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{d}");
    }
    assert_all_reconciled(&pool).await;
}

// ---------- F-09 ----------
#[sqlx::test(migrations = "../../db/migrations")]
async fn force_delete_cleans_up_claims(pool: PgPool) {
    let TestUser {
        id: owner,
        token: tok,
    } = user(&pool, "u").await;
    let (wid, _, it) = fixture(&pool, owner).await;
    // 舊訪客甲（有 email）、乙（無 email）+ 登入使用者各一筆
    legacy_guest_claim(&pool, it, 2, Some("a@example.com")).await;
    legacy_guest_claim(&pool, it, 1, None).await;
    let TestUser {
        id: cu,
        token: ctok,
    } = user(&pool, "u").await; // 登入使用者捐助（有 email）
    let (s, _, _) = claim(&pool, it, Uuid::new_v4(), ck(&ctok), json!({"qty": 1})).await;
    assert_eq!(s, StatusCode::CREATED);
    let h = ck(&tok);
    // 無 force 仍 409
    assert_eq!(
        call(&pool, "DELETE", &format!("/items/{it}"), &h, None)
            .await
            .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/items/{it}?force=true"),
            &h,
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );

    let (q, del): (i32, bool) = sqlx::query_as(
        "SELECT qty_claimed, deleted_at IS NOT NULL FROM wishlist_items WHERE id=$1",
    )
    .bind(it)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((q, del), (0, true));
    let cancelled: i64 =
        sqlx::query_scalar("SELECT count(*) FROM claims WHERE item_id=$1 AND status='cancelled'")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(cancelled, 3);
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='claim.cancel_by_item_delete' AND actor_id=$1").bind(owner).fetch_one(&pool).await.unwrap();
    assert_eq!(audits, 3);
    // 通知：有 email 的訪客甲 + 使用者各一則；無 email 的乙沒有
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.item_removed' AND payload->>'wishlist_id' = $1").bind(wid.to_string()).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 2);
    let nu: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind='claim.item_removed' AND user_id=$1",
    )
    .bind(cu)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(nu, 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn force_delete_still_forbidden_while_surprise_locked(pool: PgPool) {
    let TestUser {
        id: owner,
        token: tok,
    } = user(&pool, "u").await;
    let (wid, _, it) = fixture(&pool, owner).await;
    sqlx::query(
        "UPDATE wishlists SET surprise_mode = true, event_date = current_date + 30 WHERE id = $1",
    )
    .bind(wid)
    .execute(&pool)
    .await
    .unwrap();
    let me = user(&pool, "甲").await;
    let (s, _, _) = claim(&pool, it, Uuid::new_v4(), ck(&me.token), json!({"qty": 1})).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(
        call(
            &pool,
            "DELETE",
            &format!("/items/{it}?force=true"),
            &ck(&tok),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(q, 1);
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_off_allowed_without_event_date_but_not_with_future_date(pool: PgPool) {
    let TestUser { id: owner, token } = user(&pool, "u").await;
    let (wid, _, _) = fixture(&pool, owner).await;
    let patch = |pool: PgPool, token: String| async move {
        call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&token), Some(json!({"surprise_mode": false}))).await
    };
    sqlx::query("UPDATE wishlists SET surprise_mode = true, event_date = current_date + 30 WHERE id = $1")
        .bind(wid).execute(&pool).await.unwrap();
    assert_eq!(patch(pool.clone(), token.clone()).await.0, StatusCode::FORBIDDEN);
    // 同一請求清掉日期不能繞過鎖定
    let (s, _, _) = call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&token), Some(json!({"surprise_mode": false, "event_date": null}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    sqlx::query("UPDATE wishlists SET surprise_mode = true, event_date = NULL WHERE id = $1")
        .bind(wid).execute(&pool).await.unwrap();
    let (s, _, b) = patch(pool.clone(), token).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let on: bool = sqlx::query_scalar("SELECT surprise_mode FROM wishlists WHERE id=$1")
        .bind(wid).fetch_one(&pool).await.unwrap();
    assert!(!on);
}
