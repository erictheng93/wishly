//! 永久刪除清單：DELETE /wishlists/{id}?permanent=true（只限草稿或已封存的個人清單；寫 deleted_at，不實體刪列）。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn del(pool: &PgPool, id: Uuid, tok: &str, permanent: bool) -> Resp {
    let q = if permanent { "?permanent=true" } else { "" };
    call(
        pool,
        "DELETE",
        &format!("/wishlists/{id}{q}"),
        &ck(tok),
        None,
    )
    .await
}

/// 已封存（經 API）的清單：先封存、再回傳
async fn archived(pool: &PgPool, owner: &TestUser, vis: &'static str) -> TestList {
    let w = wishlist(
        pool,
        owner.id,
        ListOpts {
            visibility: Some(vis),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(
        del(pool, w.id, &owner.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    w
}

async fn row(pool: &PgPool, id: Uuid) -> (String, bool) {
    sqlx::query_as("SELECT status::text, deleted_at IS NOT NULL FROM wishlists WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn add_shipping(pool: &PgPool, w: Uuid) {
    let sealed = |s: &str| wishly_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO shipping_addresses (wishlist_id, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4)")
        .bind(w).bind(sealed("王小米")).bind(sealed("0912345678")).bind(sealed("台北市大安區某路 1 號")).execute(pool).await.unwrap();
}

/// 眾籌品項認捐滿額 → funded（建立 pending 採購單）
async fn funded_item(pool: &PgPool, w: Uuid) -> (Uuid, TestUser) {
    add_shipping(pool, w).await;
    let it = crowdfund_item(pool, w, 500).await;
    let donor = user(pool, "贊助者").await;
    grant(pool, donor.id, 1000).await;
    let (s, _, b) = pledge(&test_app(pool), it, &donor.token, 500).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    assert_eq!(b["funded"], true, "{b}");
    (it, donor)
}

/// 營運推進採購單（走 /admin/orders，與正式流程相同：delivered 時品項轉 fulfilled）
async fn advance_order(pool: &PgPool, item: Uuid, to: &str) {
    let staff_user = staff(pool, "營運").await;
    let oid: Uuid = sqlx::query_scalar("SELECT id FROM purchase_orders WHERE item_id=$1")
        .bind(item)
        .fetch_one(pool)
        .await
        .unwrap();
    let body = match to {
        "placed" => json!({"status": "placed", "merchant_order_id": "SHOP-1", "amount": 500}),
        _ => json!({"status": to}),
    };
    let (s, _, b) = call(
        pool,
        "PATCH",
        &format!("/admin/orders/{oid}"),
        &ck(&staff_user.token),
        Some(body),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{to}: {b}");
}

fn ids(v: &Value) -> Vec<String> {
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn permanent_delete_hides_list_everywhere(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = archived(&pool, &o, "public").await;
    item(&pool, w.id, "奶瓶", 1).await;
    let (s, _, b) = del(&pool, w.id, &o.token, true).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    assert_eq!(row(&pool, w.id).await, ("archived".into(), true));

    let h = ck(&o.token);
    let wid = w.id.to_string();
    for uri in [
        "/wishlists",
        "/wishlists?status=archived",
        "/wishlists?status=draft,active,closed",
    ] {
        let (s, _, b) = call(&pool, "GET", uri, &h, None).await;
        assert_eq!(s, StatusCode::OK);
        assert!(!ids(&b).contains(&wid), "{uri} 不可列出已刪除清單: {b}");
    }
    let base = format!("/wishlists/{}", w.id);
    for (m, uri, body) in [
        ("GET", base.clone(), None),
        ("PATCH", base.clone(), Some(json!({"status": "closed"}))),
        ("PATCH", base.clone(), Some(json!({"title": "改名"}))),
        (
            "POST",
            format!("{base}/items"),
            Some(json!({"title": "新品項"})),
        ),
        ("GET", format!("{base}/dashboard"), None),
        ("GET", format!("{base}/shipping-address"), None),
        ("DELETE", base.clone(), None),
        ("DELETE", format!("{base}?permanent=true"), None),
    ] {
        let (s, _, b) = call(&pool, m, &uri, &h, body).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} {uri}: {b}");
    }
    for uri in [
        format!("/public/wishlists/{}", w.slug),
        format!("/restricted/wishlists/{}", w.slug),
    ] {
        assert_eq!(
            call(&pool, "GET", &uri, &h, None).await.0,
            StatusCode::NOT_FOUND,
            "{uri}"
        );
    }
    assert_eq!(row(&pool, w.id).await, ("archived".into(), true));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn plain_delete_still_archives(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    assert_eq!(
        del(&pool, w.id, &o.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(row(&pool, w.id).await, ("archived".into(), false));
    // 封存後仍可取消封存（未帶 permanent 不會刪除）
    let (s, _, b) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", w.id),
        &ck(&o.token),
        Some(json!({"status": "closed"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn permanent_delete_requires_archived(pool: PgPool) {
    let o = user(&pool, "o").await;
    for st in ["active", "closed"] {
        let w = wishlist(
            &pool,
            o.id,
            ListOpts {
                status: Some(st),
                ..Default::default()
            },
        )
        .await;
        let (s, _, b) = del(&pool, w.id, &o.token, true).await;
        assert_eq!(s, StatusCode::CONFLICT, "非封存清單不可永久刪除 {st}: {b}");
        assert_eq!(b["code"], "WISHLIST_NOT_ARCHIVED", "{b}");
        assert_eq!(row(&pool, w.id).await, (st.to_string(), false));
    }
}

/// 從未發佈的草稿可直接永久刪除（不必先封存）；刪除後狀態定為 archived，圖片照常退役。
#[sqlx::test(migrations = "../../db/migrations")]
async fn draft_can_be_deleted_directly(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(
        &pool,
        o.id,
        ListOpts {
            status: Some("draft"),
            ..Default::default()
        },
    )
    .await;
    let (cover, img) = (
        format!("covers/{}.jpg", Uuid::new_v4()),
        format!("items/{}.png", Uuid::new_v4()),
    );
    sqlx::query("UPDATE wishlists SET cover_image_key=$2, cover_image_status='ready' WHERE id=$1")
        .bind(w.id)
        .bind(&cover)
        .execute(&pool)
        .await
        .unwrap();
    let it = item(&pool, w.id, "奶瓶", 1).await;
    sqlx::query("UPDATE wishlist_items SET image_key=$2, image_status='ready' WHERE id=$1")
        .bind(it)
        .bind(&img)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, b) = del(&pool, w.id, &o.token, true).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    assert_eq!(row(&pool, w.id).await, ("archived".into(), true));
    let closed: bool =
        sqlx::query_scalar("SELECT closed_at IS NOT NULL FROM wishlists WHERE id=$1")
            .bind(w.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(closed);
    for k in [&cover, &img] {
        let retiring: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_media WHERE user_id=$1 AND object_key=$2 AND (retire_requested_at IS NOT NULL OR deleted_at IS NOT NULL))")
            .bind(o.id).bind(k).fetch_one(&pool).await.unwrap();
        assert!(retiring, "{k} 須進入退役流程");
    }
    assert_eq!(
        del(&pool, w.id, &o.token, true).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn active_claims_block_delete_until_finished(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    let it = item(&pool, w.id, "奶瓶", 3).await;
    let (t1, c1) = user_claim(&pool, it, 1).await;
    let (t2, c2) = user_claim(&pool, it, 1).await;
    let patch = |tok: String, c: Uuid, st: &'static str| {
        let pool = pool.clone();
        async move {
            let (s, _, b) = call(
                &pool,
                "PATCH",
                &format!("/claims/{c}"),
                &ck(&tok),
                Some(json!({"status": st})),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{b}");
        }
    };
    patch(t2.clone(), c2, "purchased").await;
    assert_eq!(
        del(&pool, w.id, &o.token, false).await.0,
        StatusCode::NO_CONTENT
    );

    let (s, _, b) = del(&pool, w.id, &o.token, true).await;
    assert_eq!(s, StatusCode::CONFLICT, "進行中的認領須擋下刪除: {b}");
    assert_eq!(b["code"], "WISHLIST_HAS_ACTIVE_CLAIMS");
    assert_eq!(b["errors"][0]["code"], "ACTIVE_CLAIMS");
    assert_eq!(
        b["errors"][0]["count"], 2,
        "進行中的認領須擋下刪除（reserved + purchased）: {b}"
    );

    // 取消 reserved：剩下 purchased 仍擋
    let (s, _, b) = call(&pool, "DELETE", &format!("/claims/{c1}"), &ck(&t1), None).await;
    assert!(s.is_success(), "{b}");
    let (s, _, b) = del(&pool, w.id, &o.token, true).await;
    assert_eq!(
        s,
        StatusCode::CONFLICT,
        "進行中的認領須擋下刪除（purchased）: {b}"
    );
    assert_eq!(b["errors"][0]["count"], 1, "{b}");
    assert_eq!(row(&pool, w.id).await, ("archived".into(), false));

    patch(t2, c2, "delivered").await;
    let (s, _, b) = del(&pool, w.id, &o.token, true).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    assert_claims_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn active_purchase_blocks_delete_until_delivered(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    let (it, _) = funded_item(&pool, w.id).await;
    assert_eq!(
        del(&pool, w.id, &o.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    for po in ["pending", "placed", "shipped"] {
        if po != "pending" {
            advance_order(&pool, it, po).await;
        }
        let (s, _, b) = del(&pool, w.id, &o.token, true).await;
        assert_eq!(s, StatusCode::CONFLICT, "進行中的採購須擋下刪除 {po}: {b}");
        assert_eq!(b["code"], "WISHLIST_HAS_ACTIVE_ORDERS", "{b}");
        assert_eq!(b["errors"][0]["code"], "ACTIVE_ORDERS");
        assert_eq!(b["errors"][0]["count"], 1);
    }
    advance_order(&pool, it, "delivered").await; // 送達後可刪
    let (s, _, b) = del(&pool, w.id, &o.token, true).await;
    assert_eq!(s, StatusCode::NO_CONTENT, "{b}");
    assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_owner_can_delete_and_org_sites_are_not_applicable(pool: PgPool) {
    let o = user(&pool, "o").await;
    let friend = user(&pool, "friend").await;
    let stranger = user(&pool, "stranger").await;
    befriend(&pool, o.id, friend.id).await;
    let w = archived(&pool, &o, "friends").await;
    for t in [&friend, &stranger] {
        assert_eq!(
            del(&pool, w.id, &t.token, true).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(row(&pool, w.id).await, ("archived".into(), false));

    let org_id = org(&pool, o.id, "verified").await;
    member(&pool, org_id, o.id, "owner").await;
    let p = permit(&pool, org_id, "approved", 30).await;
    let (e, _) = event(&pool, org_id).await;
    let s = site(&pool, org_id, o.id, e, p, "active").await;
    sqlx::query("UPDATE wishlists SET status='archived' WHERE id=$1")
        .bind(s.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, b) = del(&pool, s.id, &o.token, true).await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["errors"][0]["code"], "NOT_APPLICABLE", "{b}");
    assert_eq!(row(&pool, s.id).await, ("archived".into(), false));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_clears_address_media_and_invites(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(
        &pool,
        o.id,
        ListOpts {
            visibility: Some("friends"),
            ..Default::default()
        },
    )
    .await;
    add_shipping(&pool, w.id).await;
    let (cover, img) = (
        format!("covers/{}.jpg", Uuid::new_v4()),
        format!("items/{}.png", Uuid::new_v4()),
    );
    sqlx::query("UPDATE wishlists SET cover_image_key=$2, cover_image_status='ready' WHERE id=$1")
        .bind(w.id)
        .bind(&cover)
        .execute(&pool)
        .await
        .unwrap();
    let it = item(&pool, w.id, "奶瓶", 1).await;
    sqlx::query("UPDATE wishlist_items SET image_key=$2, image_status='ready' WHERE id=$1")
        .bind(it)
        .bind(&img)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, inv) = call(
        &pool,
        "POST",
        "/friends/invites",
        &ck(&o.token),
        Some(json!({"wishlist_id": w.id})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{inv}");
    let token = inv["token"].as_str().unwrap().to_string();
    assert_eq!(
        del(&pool, w.id, &o.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        del(&pool, w.id, &o.token, true).await.0,
        StatusCode::NO_CONTENT
    );

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM shipping_addresses WHERE wishlist_id=$1")
        .bind(w.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "收件資訊須立即刪除");
    let keys: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT w.cover_image_key, i.image_key FROM wishlists w JOIN wishlist_items i ON i.wishlist_id = w.id WHERE w.id=$1")
        .bind(w.id).fetch_one(&pool).await.unwrap();
    assert_eq!(keys, (None, None));
    for k in [&cover, &img] {
        let retiring: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM user_media WHERE user_id=$1 AND object_key=$2 AND (retire_requested_at IS NOT NULL OR deleted_at IS NOT NULL))")
            .bind(o.id).bind(k).fetch_one(&pool).await.unwrap();
        assert!(retiring, "{k} 須進入退役流程");
    }
    let revoked: bool = sqlx::query_scalar(
        "SELECT bool_and(revoked_at IS NOT NULL) FROM friend_invites WHERE wishlist_id=$1",
    )
    .bind(w.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revoked);
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/friends/invites/{token}"),
            &[],
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn givers_keep_history_marked_deleted(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET title='寶寶清單' WHERE id=$1")
        .bind(w.id)
        .execute(&pool)
        .await
        .unwrap();
    let it = item(&pool, w.id, "奶瓶", 1).await;
    let (tok, c) = user_claim(&pool, it, 1).await;
    for st in ["purchased", "delivered"] {
        assert_eq!(
            call(
                &pool,
                "PATCH",
                &format!("/claims/{c}"),
                &ck(&tok),
                Some(json!({"status": st}))
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let (cf, donor) = funded_item(&pool, w.id).await;
    for to in ["placed", "shipped", "delivered"] {
        advance_order(&pool, cf, to).await;
    }
    let mine = |tok: String| {
        let pool = pool.clone();
        async move { call(&pool, "GET", "/guest/me", &ck(&tok), None).await.2 }
    };
    let wallet = |tok: String| {
        let pool = pool.clone();
        async move {
            call(&pool, "GET", "/wallet/contributions", &ck(&tok), None)
                .await
                .2
        }
    };
    assert_eq!(
        mine(tok.clone()).await["claims"][0]["wishlist_deleted"],
        false
    );
    assert_eq!(
        wallet(donor.token.clone()).await["data"][0]["wishlist_deleted"],
        false
    );
    assert_eq!(
        del(&pool, w.id, &o.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        del(&pool, w.id, &o.token, true).await.0,
        StatusCode::NO_CONTENT
    );

    let b = mine(tok).await;
    let r = &b["claims"][0];
    assert_eq!(r["claim"]["id"], c.to_string(), "{b}");
    assert_eq!(r["wishlist_deleted"], true, "{b}");
    assert_eq!(r["wishlist"]["title"], "寶寶清單");
    let b = wallet(donor.token).await;
    assert_eq!(b["data"][0]["wishlist_deleted"], true, "{b}");
    assert_eq!(b["data"][0]["wishlist"]["title"], "寶寶清單");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_item_shows_deleted_target_as_ended(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id) VALUES($1,'支持',1,'support',$2)")
        .bind(list.id).bind(target.id).execute(&pool).await.unwrap();
    let support = |pool: PgPool, slug: String| async move {
        call(
            &pool,
            "GET",
            &format!("/public/wishlists/{slug}"),
            &[],
            None,
        )
        .await
        .2["items"][0]["support"]
            .clone()
    };
    assert_eq!(
        support(pool.clone(), list.slug.clone()).await["ended"],
        false
    );
    assert_eq!(
        del(&pool, target.id, &b.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        del(&pool, target.id, &b.token, true).await.0,
        StatusCode::NO_CONTENT
    );
    let sp = support(pool.clone(), list.slug.clone()).await;
    assert_eq!(sp["ended"], true, "{sp}");
    assert!(sp["target"].is_null(), "{sp}");
}

/// 發佈含支持品項的清單會通知對象（wishlist.supported）；對象已永久刪除就不通知（連結會指向不存在的清單）
#[sqlx::test(migrations = "../../db/migrations")]
async fn publishing_support_item_skips_deleted_target(pool: PgPool) {
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let supported = || {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='wishlist.supported'",
            )
            .bind(b.id)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let publish = |name: &'static str| {
        let pool = pool.clone();
        async move {
            let s = user(&pool, name).await;
            let w = wishlist(
                &pool,
                s.id,
                ListOpts {
                    status: Some("draft"),
                    ..Default::default()
                },
            )
            .await;
            sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id) VALUES($1,'支持',1,'support',$2)")
                .bind(w.id).bind(target.id).execute(&pool).await.unwrap();
            let (st, _, body) = call(
                &pool,
                "PATCH",
                &format!("/wishlists/{}", w.id),
                &ck(&s.token),
                Some(json!({"status": "active"})),
            )
            .await;
            assert_eq!(st, StatusCode::OK, "{body}");
        }
    };
    publish("s1").await;
    assert_eq!(supported().await, 1);
    assert_eq!(
        del(&pool, target.id, &b.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        del(&pool, target.id, &b.token, true).await.0,
        StatusCode::NO_CONTENT
    );
    publish("s2").await;
    assert_eq!(
        supported().await,
        1,
        "已刪除的支持對象不可再收到 wishlist.supported"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_audit_has_no_pii(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, ListOpts::default()).await;
    add_shipping(&pool, w.id).await;
    item(&pool, w.id, "奶瓶", 1).await;
    assert_eq!(
        del(&pool, w.id, &o.token, false).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        del(&pool, w.id, &o.token, true).await.0,
        StatusCode::NO_CONTENT
    );
    let (actor, diff): (Option<Uuid>, Value) = sqlx::query_as(
        "SELECT actor_id, diff FROM audit_logs WHERE action='wishlist.delete' AND entity='wishlists' AND entity_id=$1")
        .bind(w.id).fetch_one(&pool).await.unwrap();
    assert_eq!(actor, Some(o.id));
    assert_eq!(diff["before"]["title"], "清單");
    assert_eq!(diff["before"]["item_count"], 1);
    let text = diff.to_string();
    for pii in ["王小米", "0912345678", "台北市", "@example.com"] {
        assert!(!text.contains(pii), "稽核不可含個資 {pii}: {text}");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_delete_and_unarchive_never_leave_deleted_unarchived(pool: PgPool) {
    let o = user(&pool, "o").await;
    let app = test_app(&pool);
    for _ in 0..5 {
        let w = archived(&pool, &o, "public").await;
        let h = ck(&o.token);
        let (du, pu) = (
            format!("/wishlists/{}?permanent=true", w.id),
            format!("/wishlists/{}", w.id),
        );
        let (d, p) = tokio::join!(
            call_app(&app, "DELETE", &du, &h, None),
            call_app(&app, "PATCH", &pu, &h, Some(json!({"status": "closed"}))),
        );
        let (deleted, unarchived) = (d.0 == StatusCode::NO_CONTENT, p.0 == StatusCode::OK);
        assert!(
            deleted ^ unarchived,
            "只能有一方成功: delete={} {} patch={} {}",
            d.0,
            d.2,
            p.0,
            p.2
        );
        if deleted {
            assert_eq!(p.0, StatusCode::NOT_FOUND, "{}", p.2);
        } else {
            assert_eq!(d.2["code"], "WISHLIST_NOT_ARCHIVED", "{}", d.2);
        }
        let bad: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM wishlists WHERE deleted_at IS NOT NULL AND status <> 'archived'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(bad, 0);
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn funding_tick_skips_deleted_lists(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = archived(&pool, &o, "public").await;
    let it = crowdfund_item(&pool, w.id, 500).await;
    assert_eq!(
        del(&pool, w.id, &o.token, true).await.0,
        StatusCode::NO_CONTENT
    );
    sqlx::query(
        "UPDATE wishlist_items SET funding_deadline = now() - interval '1 minute' WHERE id=$1",
    )
    .bind(it)
    .execute(&pool)
    .await
    .unwrap();
    wishly_api::contributions::tick_funding(&pool)
        .await
        .unwrap();
    let fs: String =
        sqlx::query_scalar("SELECT funding_status::text FROM wishlist_items WHERE id=$1")
            .bind(it)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(fs, "open", "背景工作不可再處理已刪除清單");
}

// ---- 刪除與「寫入子表」的競態：寫入端須在交易內鎖清單（FOR SHARE）並重驗 ----

fn ship_body() -> Value {
    json!({"recipient_name": "王小米", "phone": "0912345678", "address": "台北市大安區某路 1 號"})
}

type Racer = (&'static str, String, Option<Value>);

/// 三個寫入端；from-default 需先有帳號預設收件地址
async fn racers(pool: &PgPool, o: &TestUser, w: Uuid) -> Vec<Racer> {
    let sealed = |s: &str| wishly_api::sealed::seal(s.as_bytes());
    sqlx::query("INSERT INTO user_addresses (user_id, kind, recipient_name_enc, phone_enc, address_enc) VALUES ($1, 'shipping', $2, $3, $4)")
        .bind(o.id).bind(sealed("王小米")).bind(sealed("0912345678")).bind(sealed("台北市大安區某路 1 號")).execute(pool).await.unwrap();
    vec![
        (
            "PUT",
            format!("/wishlists/{w}/shipping-address"),
            Some(ship_body()),
        ),
        (
            "POST",
            format!("/wishlists/{w}/shipping-address/from-default"),
            None,
        ),
        (
            "POST",
            "/friends/invites".to_string(),
            Some(json!({ "wishlist_id": w })),
        ),
    ]
}

async fn leftovers(pool: &PgPool, w: Uuid) -> (i64, i64) {
    let a = sqlx::query_scalar("SELECT count(*) FROM shipping_addresses WHERE wishlist_id=$1")
        .bind(w)
        .fetch_one(pool)
        .await
        .unwrap();
    let i = sqlx::query_scalar(
        "SELECT count(*) FROM friend_invites WHERE wishlist_id=$1 AND revoked_at IS NULL",
    )
    .bind(w)
    .fetch_one(pool)
    .await
    .unwrap();
    (a, i)
}

fn draft_opts() -> ListOpts {
    ListOpts {
        status: Some("draft"),
        visibility: Some("friends"),
        ..Default::default()
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn address_and_invite_writers_404_on_deleted_draft(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, draft_opts()).await;
    let reqs = racers(&pool, &o, w.id).await;
    assert_eq!(
        del(&pool, w.id, &o.token, true).await.0,
        StatusCode::NO_CONTENT
    );
    for (m, uri, body) in reqs {
        let h = [ck(&o.token), idem(Uuid::new_v4())].concat();
        let (s, _, b) = call(&pool, m, &uri, &h, body).await;
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} {uri}: {b}");
    }
    assert_eq!(leftovers(&pool, w.id).await, (0, 0));
}

/// 另一交易先持有清單列鎖並完成「刪除」：請求必須卡住等鎖，放行後 404、不留子表資料
#[sqlx::test(migrations = "../../db/migrations")]
async fn address_and_invite_writers_wait_for_list_lock_then_404(pool: PgPool) {
    let o = user(&pool, "o").await;
    let w = wishlist(&pool, o.id, draft_opts()).await;
    for (m, uri, body) in racers(&pool, &o, w.id).await {
        let mut held = pool.begin().await.unwrap();
        sqlx::query("SELECT 1 FROM wishlists WHERE id=$1 FOR UPDATE")
            .bind(w.id)
            .execute(&mut *held)
            .await
            .unwrap();
        let app = test_app(&pool);
        let h = [ck(&o.token), idem(Uuid::new_v4())].concat();
        let mut req = tokio::spawn(async move { call_app(&app, m, &uri, &h, body).await });
        let early = tokio::time::timeout(std::time::Duration::from_millis(600), &mut req).await;
        assert!(early.is_err(), "{m} 應卡在清單列鎖上: {early:?}");
        sqlx::query("DELETE FROM shipping_addresses WHERE wishlist_id=$1")
            .bind(w.id)
            .execute(&mut *held)
            .await
            .unwrap();
        sqlx::query("UPDATE friend_invites SET revoked_at = now() WHERE wishlist_id=$1")
            .bind(w.id)
            .execute(&mut *held)
            .await
            .unwrap();
        sqlx::query("UPDATE wishlists SET deleted_at = now(), status = 'archived' WHERE id=$1")
            .bind(w.id)
            .execute(&mut *held)
            .await
            .unwrap();
        held.commit().await.unwrap();
        let (s, _, b) = req.await.unwrap();
        assert_eq!(s, StatusCode::NOT_FOUND, "{m} 不可寫回已刪除的清單: {b}");
        assert_eq!(leftovers(&pool, w.id).await, (0, 0));
        // 還原給下一個寫入端
        sqlx::query("UPDATE wishlists SET deleted_at = NULL, status = 'draft' WHERE id=$1")
            .bind(w.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
