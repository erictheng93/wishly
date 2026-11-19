//! 擁有者推進與取消捐助。
mod common;
use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

#[sqlx::test(migrations = "../../db/migrations")]
async fn owner_delivers_and_cancels_claims(pool: PgPool) {
    let TestUser { id: o, token: ot } = user(&pool, "u").await;
    let TestUser { id: _, token: st } = user(&pool, "u").await;
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
    let wid = wl.id;
    let it = item(&pool, wl.id, "奶瓶", 3).await;
    let (_, c1) = user_claim(&pool, it, 1).await;
    let (_, c2) = user_claim(&pool, it, 1).await;
    let url = |c: Uuid| format!("/claims/{c}");
    // 路人 → 404（不洩漏存在）；擁有者改數量或 purchased → 403
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &url(c1),
            &ck(&st),
            Some(json!({"status":"delivered"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &url(c1),
            &ck(&ot),
            Some(json!({"status":"purchased"}))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&pool, "PATCH", &url(c1), &ck(&ot), Some(json!({"qty":2})))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    // 推進 delivered（qty_claimed 不變）
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &url(c1),
        &ck(&ot),
        Some(json!({"status":"delivered"})),
    )
    .await;
    assert_eq!(
        (
            s,
            v["claim"]["status"].as_str(),
            v["item"]["qty_claimed"].as_i64()
        ),
        (StatusCode::OK, Some("delivered"), Some(2)),
        "{v}"
    );
    // delivered 不可取消
    assert_eq!(
        call(&pool, "DELETE", &url(c1), &ck(&ot), None).await.0,
        StatusCode::CONFLICT
    );
    // 取消別人的捐助 → 回補、audit
    assert_eq!(
        call(&pool, "DELETE", &url(c2), &ck(&ot), None).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&pool, "DELETE", &url(c2), &ck(&ot), None).await.0,
        StatusCode::NO_CONTENT
    );
    let q: i32 = sqlx::query_scalar("SELECT qty_claimed FROM wishlist_items WHERE id=$1")
        .bind(it)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(q, 1);
    let a: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type='user' AND actor_id=$1 AND action='claim.owner_update'").bind(o).fetch_one(&pool).await.unwrap();
    assert_eq!(a, 2);
    // 驚喜鎖定期間禁止
    let wl = wishlist(
        &pool,
        o,
        ListOpts {
            surprise: true,
            event_offset_days: Some(30),
            ..Default::default()
        },
    )
    .await;
    let it2 = item(&pool, wl.id, "奶瓶", 3).await;
    let (_, c3) = user_claim(&pool, it2, 1).await;
    assert_eq!(
        call(&pool, "DELETE", &url(c3), &ck(&ot), None).await.0,
        StatusCode::FORBIDDEN
    );
    let _ = wid;
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn private_thank_you_is_owner_only_and_once_per_delivery(pool: PgPool) {
    let owner = user(&pool, "Owner").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await;
    let gift = item(&pool, wl.id, "Water", 4).await;
    let (donor, claim) = user_claim(&pool, gift, 1).await;
    let (blocked_donor, blocked_claim) = user_claim(&pool, gift, 1).await;
    sqlx::query(
        "INSERT INTO user_blocks(blocker_id,blocked_id) SELECT user_id,$2 FROM claims WHERE id=$1",
    )
    .bind(blocked_claim)
    .bind(owner.id)
    .execute(&pool)
    .await
    .unwrap();
    let (guest, legacy) = legacy_guest_claim(&pool, gift, 1, Some("unchecked@example.org")).await;
    let path = format!("/claims/{claim}/thank-you");
    let msg = Some(json!({"message":"  非常感謝！ "}));
    assert_eq!(
        call(&pool, "POST", &path, &ck(&donor), msg.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&pool, "POST", &path, &ck(&owner.token), msg.clone())
            .await
            .0,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE claims SET status='delivered', delivered_at=now(), is_anonymous=true WHERE item_id=$1").bind(gift).execute(&pool).await.unwrap();
    sqlx::query("UPDATE wishlists SET surprise_mode=true, event_date=current_date+10 WHERE id=$1")
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(&pool, "POST", &path, &ck(&owner.token), msg.clone())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE wishlists SET surprise_mode=false WHERE id=$1")
        .bind(wl.id)
        .execute(&pool)
        .await
        .unwrap();
    let h = ck(&owner.token);
    for invalid in [" ".into(), "a".repeat(501)] {
        assert_eq!(
            call(&pool, "POST", &path, &h, Some(json!({"message":invalid})))
                .await
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let (a, b) = tokio::join!(
        call(&pool, "POST", &path, &h, msg.clone()),
        call(&pool, "POST", &path, &h, msg.clone())
    );
    assert_eq!((a.0, b.0), (StatusCode::OK, StatusCode::OK));
    assert_eq!(
        call(
            &pool,
            "POST",
            &format!("/claims/{legacy}/thank-you"),
            &h,
            msg
        )
        .await
        .0,
        StatusCode::OK
    );
    let (s, _, same_result) = call(
        &pool,
        "POST",
        &format!("/claims/{blocked_claim}/thank-you"),
        &h,
        Some(json!({"message":"感謝"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(same_result, json!({"thanked":true}));
    let (_, _, blocked_me) = call(&pool, "GET", "/guest/me", &ck(&blocked_donor), None).await;
    assert!(blocked_me["claims"][0]["thank_you"].is_null());
    let notifications: Vec<(Option<Uuid>, Option<Uuid>)> =
        sqlx::query_as("SELECT user_id, guest_id FROM notifications WHERE kind='claim.thank_you'")
            .fetch_all(&pool)
            .await
            .unwrap();
    let uid: Uuid = sqlx::query_scalar("SELECT user_id FROM claims WHERE id=$1")
        .bind(claim)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(notifications, vec![(Some(uid), None)]);
    sqlx::query(
        "UPDATE users SET notification_prefs=jsonb_build_object('email_gifts', false) WHERE id=$1",
    )
    .bind(uid)
    .execute(&pool)
    .await
    .unwrap();
    wishly_api::notify::tick_at(&pool, noon()).await.unwrap();
    let (link, body, muted): (String,String,bool) = sqlx::query_as(
        "SELECT inbox_link,inbox_body,last_error='muted' FROM notifications WHERE kind='claim.thank_you'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(link, format!("/me/claims?claim={claim}"));
    assert!(muted);
    assert!(!body.contains("非常感謝"));
    for headers in [ck(&donor), gt(&guest)] {
        let (_, _, v) = call(&pool, "GET", "/guest/me", &headers, None).await;
        assert_eq!(v["claims"][0]["thank_you"], "非常感謝！", "{v}");
    }
    let (_, _, foreign) = call(
        &pool,
        "GET",
        &format!("/guest/me?claim_id={legacy}"),
        &ck(&donor),
        None,
    )
    .await;
    assert_eq!(foreign["claims"], json!([]));
    let (_, _, dash) = call(
        &pool,
        "GET",
        &format!("/wishlists/{}/dashboard", wl.id),
        &h,
        None,
    )
    .await;
    assert!(dash["claims"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["thanked"] == true
            && c["can_thank"] == false
            && c["claimer_name"] == "匿名朋友"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn thank_you_waits_on_claim_lock_and_never_overwrites(pool: PgPool) {
    let owner = user(&pool, "Owner").await;
    let wl = wishlist(&pool, owner.id, ListOpts::default()).await;
    let gift = item(&pool, wl.id, "Water", 1).await;
    let (_, claim) = user_claim(&pool, gift, 1).await;
    sqlx::query("UPDATE claims SET status='delivered', delivered_at=now() WHERE id=$1")
        .bind(claim)
        .execute(&pool)
        .await
        .unwrap();
    // 另一個請求已寫入謝卡但尚未 commit
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE claims SET thank_you='先到的謝卡' WHERE id=$1")
        .bind(claim)
        .execute(&mut *tx)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let (p, path, h) = (
        pool.clone(),
        format!("/claims/{claim}/thank-you"),
        ck(&owner.token),
    );
    let req = tokio::spawn(async move {
        call(&p, "POST", &path, &h, Some(json!({"message":"後到的謝卡"}))).await
    });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, v) = req.await.unwrap();
    assert_eq!(s, StatusCode::CONFLICT, "謝卡不可被並行請求覆寫 {v}");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='claim.thank_you'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0, "謝卡不可被並行請求覆寫");
}
