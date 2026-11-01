//! 支持品項：不計入完成度與急需統計（本任務尚無 API，以 SQL 直接插入）。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn support_item(pool: &PgPool, wl: Uuid, target: Uuid, relief: bool) {
    sqlx::query(
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id,category,urgency)
         VALUES($1,'支持',2,'support',$2,$3,$4)",
    )
    .bind(wl)
    .bind(target)
    .bind(relief.then_some("food"))
    .bind(relief.then_some("critical"))
    .execute(pool)
    .await
    .unwrap();
}

async fn completion(pool: &PgPool, slug: &str) -> Value {
    let (s, _, v) = call(pool, "GET", &format!("/public/wishlists/{slug}"), &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["completion"].clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_item_excluded_from_completion(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    item(&pool, list.id, "一般", 2).await;
    support_item(&pool, list.id, target.id, false).await;
    let c = completion(&pool, &list.slug).await;
    assert_eq!(c["item_count"], 1);
    assert_eq!(c["fulfilled_count"], 0);
    assert_eq!(c["completion_pct"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_support_items_is_not_complete(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    support_item(&pool, list.id, target.id, false).await;
    let c = completion(&pool, &list.slug).await;
    assert_eq!(c["item_count"], 0);
    assert_eq!(c["fulfilled_count"], 0);
    assert_eq!(c["completion_pct"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_needs_ignore_support_item(pool: PgPool) {
    let owner = user(&pool, "O").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e, slug) = event(&pool, o).await;
    let site = site(&pool, o, owner.id, e, p, "active").await;
    let target = wishlist(&pool, owner.id, ListOpts::default()).await;
    support_item(&pool, site.id, target.id, true).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/public/relief-events/{slug}"),
        &[],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["sites"][0]["needs"]["critical"], 0, "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn explore_keeps_list_with_only_support_items(pool: PgPool) {
    let owner = user(&pool, "owner").await;
    let other = user(&pool, "other").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    support_item(&pool, list.id, target.id, false).await;
    let (s, _, v) = call(&pool, "GET", "/public/wishlists", &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let l = v["wishlists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["slug"] == list.slug.as_str())
        .expect("留在探索頁");
    assert_eq!(l["item_count"], 1);
    assert_eq!(l["completion_pct"], 0);
}

async fn create(pool: &PgPool, tok: &str, wl: Uuid, slug: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "POST",
        &format!("/wishlists/{wl}/items"),
        &ck(tok),
        Some(json!({ "title": "支持", "funding_mode": "support", "support_slug": slug })),
    )
    .await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_support_item_without_date(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &a.token, list.id, &target.slug).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(v["funding_mode"], "support");
    assert!(v["funding_deadline"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rejects_unavailable_target(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let friends = wishlist(
        &pool,
        b.id,
        ListOpts {
            visibility: Some("friends"),
            ..Default::default()
        },
    )
    .await;
    let private = wishlist(
        &pool,
        b.id,
        ListOpts {
            visibility: Some("private"),
            ..Default::default()
        },
    )
    .await;
    let mut details = vec![];
    for slug in [friends.slug.as_str(), private.slug.as_str(), "nonexistent"] {
        let (s, v) = create(&pool, &a.token, list.id, slug).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert_eq!(v["code"], "SUPPORT_TARGET_INVALID");
        details.push(v["detail"].clone());
    }
    assert!(details.windows(2).all(|w| w[0] == w[1]));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rejects_own_list(pool: PgPool) {
    let a = user(&pool, "a").await;
    let own = wishlist(&pool, a.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &a.token, list.id, &own.slug).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "SUPPORT_TARGET_SELF");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rejects_manageable_org_list(pool: PgPool) {
    let owner = user(&pool, "o").await;
    let coord = user(&pool, "c").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    member(&pool, o, coord.id, "coordinator").await;
    let p = permit(&pool, o, "approved", 30).await;
    let (e, _) = event(&pool, o).await;
    let st = site(&pool, o, owner.id, e, p, "active").await;
    let list = wishlist(&pool, coord.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &coord.token, list.id, &st.slug).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["code"], "SUPPORT_TARGET_SELF");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_rate_limited(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    for _ in 0..20 {
        let (s, v) = create(&pool, &a.token, list.id, &target.slug).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let (s, _) = create(&pool, &a.token, list.id, &target.slug).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_mode_cannot_change(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (_, v) = create(&pool, &a.token, list.id, &target.slug).await;
    let id = v["id"].as_str().unwrap();
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{id}"),
        &ck(&a.token),
        Some(json!({ "funding_mode": "crowdfund" })),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["code"], "FUNDING_MODE_LOCKED");
    let q = item(&pool, list.id, "一般", 1).await;
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{q}"),
        &ck(&a.token),
        Some(json!({ "funding_mode": "support" })),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_target_is_immutable(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (_, v) = create(&pool, &a.token, list.id, &target.slug).await;
    let id = v["id"].as_str().unwrap();
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &format!("/items/{id}"),
        &ck(&a.token),
        Some(json!({ "support_slug": target.slug })),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_and_pledge_rejected_on_support_item(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (_, v) = create(&pool, &a.token, list.id, &target.slug).await;
    let id = v["id"].as_str().unwrap();
    let d = user(&pool, "d").await;
    for (path, body) in [
        ("claims", json!({ "qty": 1, "visibility": "public" })),
        ("contributions", json!({ "points": 10 })),
    ] {
        let h = [ck(&d.token), idem(Uuid::new_v4())].concat();
        let (s, _, v) = call(
            &pool,
            "POST",
            &format!("/items/{id}/{path}"),
            &h,
            Some(body),
        )
        .await;
        assert!(!s.is_success(), "{path} {v}");
    }
    let (c, p): (i32, i64) =
        sqlx::query_as("SELECT qty_claimed, pledged_points FROM wishlist_items WHERE id=$1::uuid")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((c, p), (0, 0));
}

// ---------- 歸因、彙總與公開回應 ----------

async fn sup(pool: &PgPool, wl: Uuid, target: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id)
         VALUES($1,'支持',1,'support',$2) RETURNING id",
    )
    .bind(wl)
    .bind(target)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pledge_via(pool: &PgPool, item: Uuid, tok: &str, pts: i64, via: Option<Uuid>) -> Resp {
    call(
        pool,
        "POST",
        &format!("/items/{item}/contributions"),
        &[ck(tok), idem(Uuid::new_v4())].concat(),
        Some(json!({ "points": pts, "via_item_id": via })),
    )
    .await
}

async fn claim_via(pool: &PgPool, item: Uuid, tok: &str, qty: i32, via: Option<Uuid>) -> Resp {
    call(
        pool,
        "POST",
        &format!("/items/{item}/claims"),
        &[ck(tok), idem(Uuid::new_v4())].concat(),
        Some(json!({ "qty": qty, "visibility": "public", "via_item_id": via })),
    )
    .await
}

/// 公開頁上該支持品項的 support 物件
async fn support_of(pool: &PgPool, slug: &str, id: Uuid) -> Value {
    let (s, _, v) = call(pool, "GET", &format!("/public/wishlists/{slug}"), &[], None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == id.to_string())
        .unwrap()["support"]
        .clone()
}

struct Fx {
    list: TestList,
    sid: Uuid,
    cf: Uuid,
    qty: Uuid,
    donor: TestUser,
}

async fn fx(pool: &PgPool) -> Fx {
    let a = user(pool, "a").await;
    let b = user(pool, "b").await;
    let donor = user(pool, "d").await;
    grant(pool, donor.id, 1000).await;
    let target = wishlist(pool, b.id, ListOpts::default()).await;
    let cf = crowdfund_item(pool, target.id, 500).await;
    let qty = item(pool, target.id, "物資", 5).await;
    let list = wishlist(pool, a.id, ListOpts::default()).await;
    let sid = sup(pool, list.id, target.id).await;
    Fx {
        list,
        sid,
        cf,
        qty,
        donor,
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn pledge_via_support_item_counts(pool: PgPool) {
    let f = fx(&pool).await;
    let (s, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 100, Some(f.sid)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let sp = support_of(&pool, &f.list.slug, f.sid).await;
    assert_eq!(sp["points"], 100);
    assert_eq!(sp["qty"], 0);
    assert_eq!(sp["ended"], false);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_via_support_item_counts(pool: PgPool) {
    let f = fx(&pool).await;
    let (s, _, v) = claim_via(&pool, f.qty, &f.donor.token, 2, Some(f.sid)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let sp = support_of(&pool, &f.list.slug, f.sid).await;
    assert_eq!(sp["qty"], 2);
    assert_eq!(sp["points"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn via_ignored_when_invalid(pool: PgPool) {
    let f = fx(&pool).await;
    let other = wishlist(&pool, f.donor.id, ListOpts::default()).await;
    let other_target = user(&pool, "t2").await;
    let t2 = wishlist(&pool, other_target.id, ListOpts::default()).await;
    let elsewhere = sup(&pool, other.id, t2.id).await; // 指向別份清單
    let gone = sup(
        &pool,
        f.list.id,
        sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id=$1")
            .bind(f.cf)
            .fetch_one(&pool)
            .await
            .unwrap(),
    )
    .await;
    sqlx::query("UPDATE wishlist_items SET deleted_at=now() WHERE id=$1")
        .bind(gone)
        .execute(&pool)
        .await
        .unwrap();
    for via in [None, Some(elsewhere), Some(gone), Some(f.cf)] {
        let (s, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 10, via).await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    let (s, _, v) = claim_via(&pool, f.qty, &f.donor.token, 1, Some(f.cf)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM contributions WHERE via_item_id IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 0);
    let sp = support_of(&pool, &f.list.slug, f.sid).await;
    assert_eq!(
        (sp["points"].as_i64(), sp["qty"].as_i64()),
        (Some(0), Some(0))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_and_expiry_reduce_count(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 100, Some(f.sid)).await;
    let cid = v["contribution"]["id"].as_str().unwrap().to_string();
    let (_, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 60, Some(f.sid)).await;
    let c2 = v["contribution"]["id"].as_str().unwrap().to_string();
    let (_, _, v) = claim_via(&pool, f.qty, &f.donor.token, 3, Some(f.sid)).await;
    let claim = v["id"]
        .as_str()
        .or(v["claim"]["id"].as_str())
        .unwrap()
        .to_string();
    assert_eq!(support_of(&pool, &f.list.slug, f.sid).await["points"], 160);
    // 部分退款
    sqlx::query("UPDATE contributions SET refunded_points=20 WHERE id=$1::uuid")
        .bind(&c2)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(support_of(&pool, &f.list.slug, f.sid).await["points"], 140);
    // 撤回
    let (s, _, v) = call(
        &pool,
        "DELETE",
        &format!("/contributions/{cid}"),
        &ck(&f.donor.token),
        None,
    )
    .await;
    assert!(s.is_success(), "{v}");
    assert_eq!(support_of(&pool, &f.list.slug, f.sid).await["points"], 40);
    // 取消 claim
    let (s, _, v) = call(
        &pool,
        "DELETE",
        &format!("/claims/{claim}"),
        &ck(&f.donor.token),
        None,
    )
    .await;
    assert!(s.is_success(), "{v}");
    assert_eq!(support_of(&pool, &f.list.slug, f.sid).await["qty"], 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reallocate_keeps_attribution(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 100, Some(f.sid)).await;
    let cid = v["contribution"]["id"].as_str().unwrap().to_string();
    let wl: Uuid = sqlx::query_scalar("SELECT wishlist_id FROM wishlist_items WHERE id=$1")
        .bind(f.cf)
        .fetch_one(&pool)
        .await
        .unwrap();
    let tgt = crowdfund_item(&pool, wl, 500).await;
    sqlx::query("UPDATE wishlist_items SET funding_status='expired', expired_at=now() WHERE id=$1")
        .bind(f.cf)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/contributions/{cid}/reallocate"),
        &[ck(&f.donor.token), idem(Uuid::new_v4())].concat(),
        Some(json!({ "target_item_id": tgt })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(support_of(&pool, &f.list.slug, f.sid).await["points"], 100);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn replay_not_counted_twice(pool: PgPool) {
    let f = fx(&pool).await;
    let h = [ck(&f.donor.token), idem(Uuid::new_v4())].concat();
    let body = json!({ "points": 100, "via_item_id": f.sid });
    for _ in 0..2 {
        let (s, _, v) = call(
            &pool,
            "POST",
            &format!("/items/{}/contributions", f.cf),
            &h,
            Some(body.clone()),
        )
        .await;
        assert_eq!(s, StatusCode::CREATED, "{v}");
    }
    assert_eq!(support_of(&pool, &f.list.slug, f.sid).await["points"], 100);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn ended_target_hides_details(pool: PgPool) {
    for how in ["private", "archived", "deleted", "org"] {
        let a = user(&pool, &format!("a-{how}")).await;
        let b = user(&pool, &format!("b-{how}")).await;
        let (target_id, tslug, tid) = if how == "org" {
            let o = org(&pool, b.id, "verified").await;
            member(&pool, o, b.id, "owner").await;
            let p = permit(&pool, o, "approved", 30).await;
            let (e, _) = event(&pool, o).await;
            let s = site(&pool, o, b.id, e, p, "active").await;
            (s.id, s.slug, o)
        } else {
            let t = wishlist(&pool, b.id, ListOpts::default()).await;
            (t.id, t.slug, Uuid::nil())
        };
        sqlx::query("UPDATE wishlists SET title='祕密標題XYZ' WHERE id=$1")
            .bind(target_id)
            .execute(&pool)
            .await
            .unwrap();
        let cf = if how == "org" {
            sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'物資',5,'food','critical') RETURNING id")
                .bind(target_id).fetch_one(&pool).await.unwrap()
        } else {
            crowdfund_item(&pool, target_id, 500).await
        };
        grant(&pool, a.id, 1000).await;
        let list = wishlist(&pool, a.id, ListOpts::default()).await;
        let sid = sup(&pool, list.id, target_id).await;
        let donor = user(&pool, &format!("dn-{how}")).await;
        grant(&pool, donor.id, 1000).await;
        let (s, _, v) = if how == "org" {
            claim_via(&pool, cf, &donor.token, 5, Some(sid)).await
        } else {
            pledge_via(&pool, cf, &donor.token, 100, Some(sid)).await
        };
        assert_eq!(s, StatusCode::CREATED, "{how} {v}");
        let want = if how == "org" { 5 } else { 100 };
        let live = support_of(&pool, &list.slug, sid).await;
        assert_eq!(live["ended"], false, "{how}");
        assert_eq!(live["target"]["slug"], tslug.as_str());
        match how {
            "private" => sqlx::query("UPDATE wishlists SET visibility='private' WHERE id=$1").bind(target_id).execute(&pool).await.unwrap(),
            "archived" => sqlx::query("UPDATE wishlists SET moderation_status='hidden', moderation_reason='x' WHERE id=$1").bind(target_id).execute(&pool).await.unwrap(),
            "deleted" => sqlx::query("UPDATE wishlists SET deleted_at=now() WHERE id=$1").bind(target_id).execute(&pool).await.unwrap(),
            _ => sqlx::query("UPDATE organizations SET verification_status='rejected', verified_at=NULL WHERE id=$1").bind(tid).execute(&pool).await.unwrap(),
        };
        let (s, _, body) = call(
            &pool,
            "GET",
            &format!("/public/wishlists/{}", list.slug),
            &[],
            None,
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{how}");
        let sp = body["items"][0]["support"].clone();
        assert_eq!(sp["ended"], true, "{how}");
        assert!(sp["target"].is_null(), "{how}");
        assert_eq!(
            sp[if how == "org" { "qty" } else { "points" }],
            want,
            "{how}"
        );
        let text = body.to_string();
        assert!(
            !text.contains("祕密標題XYZ") && !text.contains(&tslug),
            "{how} {text}"
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn via_survives_target_ended_after_click(pool: PgPool) {
    let f = fx(&pool).await;
    // 對方已不是 public，但品項指向仍成立：valid_via 不看 TARGET_LIVE_SQL
    sqlx::query("UPDATE wishlists SET visibility='friends' WHERE id=(SELECT wishlist_id FROM wishlist_items WHERE id=$1)").bind(f.cf).execute(&pool).await.unwrap();
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM wishlists WHERE id=(SELECT wishlist_id FROM wishlist_items WHERE id=$1)").bind(f.cf).fetch_one(&pool).await.unwrap();
    befriend(&pool, owner, f.donor.id).await;
    let (s, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 100, Some(f.sid)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let via: Option<Uuid> = sqlx::query_scalar("SELECT via_item_id FROM contributions LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(via, Some(f.sid));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn surprise_target_hides_numbers(pool: PgPool) {
    let f = fx(&pool).await;
    let (s, _, v) = pledge_via(&pool, f.cf, &f.donor.token, 100, Some(f.sid)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    sqlx::query("UPDATE wishlists SET surprise_mode=true, event_date=current_date+10 WHERE id=(SELECT wishlist_id FROM wishlist_items WHERE id=$1)").bind(f.cf).execute(&pool).await.unwrap();
    let sp = support_of(&pool, &f.list.slug, f.sid).await;
    assert!(sp["points"].is_null() && sp["qty"].is_null(), "{sp}");
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM wishlists WHERE id=$1")
        .bind(f.list.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let tok = session(&pool, owner).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/wishlists/{}", f.list.id),
        &ck(&tok),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let it = &v["items"][0]["support"];
    assert!(it["points"].is_null() && it["qty"].is_null(), "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_list_cache_headers(pool: PgPool) {
    let f = fx(&pool).await;
    let uri = format!("/public/wishlists/{}", f.list.slug);
    let (s, h, _) = call(&pool, "GET", &uri, &[], None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["cache-control"], "public, s-maxage=10");
    assert!(h.get("etag").is_none());
    let (s, _, _) = call(
        &pool,
        "GET",
        &uri,
        &[("if-none-match", "\"w-x\"".into())],
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    // 無支持品項：標頭不變、仍有 ETag
    let c = user(&pool, "c").await;
    let plain = wishlist(&pool, c.id, ListOpts::default()).await;
    item(&pool, plain.id, "一般", 1).await;
    let (_, h, _) = call(
        &pool,
        "GET",
        &format!("/public/wishlists/{}", plain.slug),
        &[],
        None,
    )
    .await;
    assert_eq!(
        h["cache-control"],
        "public, s-maxage=10, stale-while-revalidate=60"
    );
    assert!(h.get("etag").is_some());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn no_donor_identity_in_response(pool: PgPool) {
    let f = fx(&pool).await;
    pledge_via(&pool, f.cf, &f.donor.token, 100, Some(f.sid)).await;
    let sp = support_of(&pool, &f.list.slug, f.sid).await;
    let mut keys: Vec<_> = sp.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["ended", "points", "qty", "target"]);
    let mut tk: Vec<_> = sp["target"].as_object().unwrap().keys().cloned().collect();
    tk.sort();
    assert_eq!(
        tk,
        [
            "cover_image_url",
            "cover_preset",
            "owner_name",
            "slug",
            "title"
        ]
    );
}

// ---------- 站內通知 ----------

async fn supported(pool: &PgPool, user: Uuid) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT payload FROM notifications WHERE kind='wishlist.supported' AND user_id=$1",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn published_list_notifies_target_owner(pool: PgPool) {
    let a = user(&pool, "阿花").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &a.token, list.id, &target.slug).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let n = supported(&pool, b.id).await;
    assert_eq!(n.len(), 1);
    assert_eq!(n[0]["wishlist_id"], target.id.to_string());
    assert_eq!(n[0]["supporter_name"], "阿花");
    assert_eq!(n[0]["list_title"], "清單");
    assert_eq!(
        wishly_api::notify::inbox_link("wishlist.supported", &n[0]),
        format!("/lists/{}/edit", target.id)
    );
    assert!(supported(&pool, a.id).await.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn draft_notifies_on_publish_once(pool: PgPool) {
    let a = user(&pool, "a").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(
        &pool,
        a.id,
        ListOpts {
            status: Some("draft"),
            ..Default::default()
        },
    )
    .await;
    let (s, v) = create(&pool, &a.token, list.id, &target.slug).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert!(supported(&pool, b.id).await.is_empty());
    let path = format!("/wishlists/{}", list.id);
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &path,
        &ck(&a.token),
        Some(json!({ "status": "active" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(supported(&pool, b.id).await.len(), 1);
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &path,
        &ck(&a.token),
        Some(json!({ "title": "改名" })),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(supported(&pool, b.id).await.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_target_notifies_owners_and_coordinators(pool: PgPool) {
    let owner = user(&pool, "o").await;
    let coord = user(&pool, "c").await;
    let vol = user(&pool, "v").await;
    let off = user(&pool, "off").await;
    let a = user(&pool, "a").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    member(&pool, o, coord.id, "coordinator").await;
    member(&pool, o, vol.id, "volunteer").await;
    member(&pool, o, off.id, "coordinator").await;
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE user_id=$1")
        .bind(off.id)
        .execute(&pool)
        .await
        .unwrap();
    let p = permit(&pool, o, "approved", 30).await;
    let (e, _) = event(&pool, o).await;
    let st = site(&pool, o, owner.id, e, p, "active").await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    let (s, v) = create(&pool, &a.token, list.id, &st.slug).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!(supported(&pool, owner.id).await.len(), 1);
    assert_eq!(supported(&pool, coord.id).await.len(), 1);
    assert!(supported(&pool, vol.id).await.is_empty());
    assert!(supported(&pool, off.id).await.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn supported_is_in_app_only(pool: PgPool) {
    use wishly_api::notify::{category, email_tier, Category, EmailTier};
    let mut c = wishly_api::config::Config::dev();
    c.cf_account_id = None;
    c.cf_email_api_token = None;
    c.smtp_port = 1;
    wishly_api::config::init(c);
    assert_eq!(email_tier("wishlist.supported"), EmailTier::InAppOnly);
    assert_eq!(category("wishlist.supported"), Category::Social);
    let a = user(&pool, "阿花").await;
    let b = user(&pool, "b").await;
    let target = wishlist(&pool, b.id, ListOpts::default()).await;
    let list = wishlist(&pool, a.id, ListOpts::default()).await;
    create(&pool, &a.token, list.id, &target.slug).await;
    wishly_api::notify::tick_at(&pool, noon()).await.unwrap();
    let (st, err, title, link): (String, Option<String>, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT status::text, last_error, inbox_title, inbox_link FROM notifications WHERE kind='wishlist.supported'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (st.as_str(), err.as_deref()),
        ("cancelled", Some("in-app only"))
    );
    assert_eq!(title.unwrap(), "阿花 把你的清單「清單」放進了他們的願望");
    assert_eq!(link.unwrap(), format!("/lists/{}/edit", target.id));
}
