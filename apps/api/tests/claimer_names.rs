//! 分享頁捐助者名字：restricted 路徑依觀看者（好友 / 本人）顯示真名、公開路徑維持匿名；supporter_count
mod common;
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use common::*;

struct Ctx {
    owner: TestUser,
    donor: TestUser,
    friend: TestUser,
    stranger: TestUser,
    slug: String,
    wid: Uuid,
    item: Uuid,
}
async fn ctx(pool: &PgPool) -> Ctx {
    let owner = user(pool, "主人").await;
    let donor = user(pool, "捐助者").await;
    let friend = user(pool, "好友").await;
    let stranger = user(pool, "路人").await;
    befriend(pool, donor.id, friend.id).await;
    let l = wishlist(pool, owner.id, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET show_claimer_names = true WHERE id = $1")
        .bind(l.id)
        .execute(pool)
        .await
        .unwrap();
    let item = item(pool, l.id, "奶瓶", 10).await;
    Ctx {
        owner,
        donor,
        friend,
        stranger,
        slug: l.slug,
        wid: l.id,
        item,
    }
}

async fn claim(pool: &PgPool, c: &Ctx, by: Uuid, vis: &str, anon: bool, qty: i32) {
    sqlx::query("INSERT INTO claims (item_id, user_id, claimer_name, qty, visibility, is_anonymous) VALUES ($1, $2, '小捐', $3, $4::share_level, $5)")
        .bind(c.item).bind(by).bind(qty).bind(vis).bind(anon).execute(pool).await.unwrap();
}

async fn contribute(pool: &PgPool, c: &Ctx, by: Uuid, vis: &str, anon: bool, pts: i64) {
    grant(pool, by, 1).await;
    let wallet: Uuid = sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
        .bind(by)
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points, status, captured_at, visibility, is_anonymous)
                 VALUES ($1, $2, $3, $4, '小贊', $5, 'captured', now(), $6::share_level, $7)")
        .bind(c.item).bind(c.wid).bind(by).bind(wallet).bind(pts).bind(vis).bind(anon).execute(pool).await.unwrap();
}

async fn restricted(pool: &PgPool, c: &Ctx, tok: Option<&str>) -> Value {
    let h = tok.map(ck).unwrap_or_default();
    let (s, _, v) = call(
        pool,
        "GET",
        &format!("/restricted/wishlists/{}", c.slug),
        &h,
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    v
}
async fn public(pool: &PgPool, c: &Ctx) -> Value {
    let (s, _, v) = call(
        pool,
        "GET",
        &format!("/public/wishlists/{}", c.slug),
        &[],
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    v
}
fn first(v: &Value, key: &str) -> Value {
    v["items"][0][key][0].clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friends_level_claim_by_viewer(pool: PgPool) {
    let c = ctx(&pool).await;
    claim(&pool, &c, c.donor.id, "friends", false, 2).await;
    let f = first(
        &restricted(&pool, &c, Some(&c.friend.token)).await,
        "claimers",
    );
    assert_eq!(
        (f["display_name"].as_str(), f["anonymous"].as_bool()),
        (Some("小捐"), Some(false))
    );
    for tok in [Some(c.stranger.token.as_str()), None] {
        let f = first(&restricted(&pool, &c, tok).await, "claimers");
        assert_eq!(
            (f["display_name"].as_str(), f["anonymous"].as_bool()),
            (Some("匿名"), Some(true))
        );
    }
    // 公開（可快取）路徑永遠匿名
    let f = first(&public(&pool, &c).await, "claimers");
    assert_eq!(f["display_name"], "匿名");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn anonymous_flag_hides_even_from_friend_and_self(pool: PgPool) {
    let c = ctx(&pool).await;
    claim(&pool, &c, c.donor.id, "friends", true, 1).await;
    for t in [&c.friend.token, &c.donor.token] {
        let f = first(&restricted(&pool, &c, Some(t)).await, "claimers");
        assert_eq!(f["display_name"], "匿名");
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn own_private_claim_shown_to_self_only(pool: PgPool) {
    let c = ctx(&pool).await;
    claim(&pool, &c, c.donor.id, "private", false, 1).await;
    assert_eq!(
        first(
            &restricted(&pool, &c, Some(&c.donor.token)).await,
            "claimers"
        )["display_name"],
        "小捐"
    );
    assert_eq!(
        first(
            &restricted(&pool, &c, Some(&c.friend.token)).await,
            "claimers"
        )["display_name"],
        "匿名"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn names_off_hides_claimers_in_restricted(pool: PgPool) {
    let c = ctx(&pool).await;
    sqlx::query("UPDATE wishlists SET show_claimer_names = false WHERE id = $1")
        .bind(c.wid)
        .execute(&pool)
        .await
        .unwrap();
    claim(&pool, &c, c.donor.id, "public", false, 2).await;
    let v = restricted(&pool, &c, Some(&c.friend.token)).await;
    assert!(v["items"][0].get("claimers").is_none());
    assert_eq!(v["items"][0]["supporter_count"], 1); // 人數不是身分，仍輸出
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn supporter_count_distinct_and_locked_null(pool: PgPool) {
    let c = ctx(&pool).await;
    claim(&pool, &c, c.donor.id, "public", false, 1).await;
    claim(&pool, &c, c.friend.id, "public", false, 1).await;
    // 舊訪客認捐無 user_id：每筆算一人
    legacy_guest_claim(&pool, c.item, 1, None).await;
    // 同一人先取消再認捐，只算有效的一人
    sqlx::query("INSERT INTO claims (item_id, user_id, claimer_name, qty, visibility, status) VALUES ($1, $2, 'x', 1, 'public', 'cancelled')")
        .bind(c.item).bind(c.donor.id).execute(&pool).await.unwrap();
    assert_eq!(public(&pool, &c).await["items"][0]["supporter_count"], 3);
    // 驚喜鎖定
    sqlx::query(
        "UPDATE wishlists SET surprise_mode = true, event_date = current_date + 5 WHERE id = $1",
    )
    .bind(c.wid)
    .execute(&pool)
    .await
    .unwrap();
    let v = public(&pool, &c).await;
    assert!(v["items"][0]["supporter_count"].is_null());
    assert_eq!(v["claimers_visible"], false);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn crowdfund_contributors_follow_same_rule(pool: PgPool) {
    let c = ctx(&pool).await;
    let c = Ctx {
        item: crowdfund_item(&pool, c.wid, 1000).await,
        ..c
    };
    contribute(&pool, &c, c.donor.id, "friends", false, 100).await;
    contribute(&pool, &c, c.donor.id, "friends", false, 50).await; // 同人合併
    contribute(&pool, &c, c.friend.id, "public", true, 30).await; // 匿名各自一筆
    let v = restricted(&pool, &c, Some(&c.friend.token)).await;
    let list = v["items"][1]["contributors"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (list[0]["display_name"].as_str(), list[0]["points"].as_i64()),
        (Some("小贊"), Some(150))
    );
    assert_eq!(list[1]["anonymous"], true);
    assert_eq!(v["items"][1]["supporter_count"], 2);
    // 非好友與公開路徑：friends 級視同匿名，各自一筆
    for v in [
        restricted(&pool, &c, Some(&c.stranger.token)).await,
        public(&pool, &c).await,
    ] {
        let list = v["items"][1]["contributors"].as_array().unwrap();
        assert_eq!(list.len(), 3);
        assert!(list.iter().all(|x| x["anonymous"] == true));
    }
    let _ = &c.owner;
}
