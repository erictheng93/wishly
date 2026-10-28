//! 救災公開端點（訪客、可被 CDN 快取）、探索頁排除與許可到期背景工作。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn relief_item(pool: &PgPool, wl: Uuid, urgency: &str, needed: i32, claimed: i32) {
    sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,qty_claimed,category,urgency) VALUES($1,'瓶裝水',$2,$3,'water',$4)")
        .bind(wl).bind(needed).bind(claimed).bind(urgency).execute(pool).await.unwrap();
}

async fn relief_crowdfund(pool: &PgPool, wl: Uuid, urgency: &str, status: &str) {
    sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,target_points,pledged_points,funding_status,funding_deadline,fulfillment_type,category,urgency)
        VALUES($1,'發電機',1,'crowdfund',100,CASE WHEN $3='open' THEN 0 ELSE 100 END,$3::funding_status,now()+interval '7 days','concierge','other',$2)")
        .bind(wl).bind(urgency).bind(status).execute(pool).await.unwrap();
}

async fn get(pool: &PgPool, path: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "GET", path, &[], None).await;
    (s, v)
}

/// owner + verified org + approved permit + event + active site
async fn setup(pool: &PgPool) -> (TestUser, Uuid, Uuid, (Uuid, String), TestList) {
    let u = user(pool, "A").await;
    let o = org(pool, u.id, "verified").await;
    member(pool, o, u.id, "owner").await;
    let p = permit(pool, o, "approved", 30).await;
    let e = event(pool, o).await;
    let s = site(pool, o, u.id, e.0, p, "active").await;
    (u, o, p, e, s)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_site_includes_org_and_event(pool: PgPool) {
    let (_, o, _, e, s) = setup(&pool).await;
    let (st, v) = get(&pool, &format!("/public/wishlists/{}", s.slug)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let os: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        v["org"],
        json!({"slug": os, "display_name": "測試協會", "verified": true})
    );
    assert_eq!(v["event"], json!({"slug": e.1, "name": "測試事件"}));
    assert_eq!(
        (v["site_address"].as_str(), v["site_status"].as_str()),
        (Some("花蓮縣某處"), Some("active"))
    );
}

/// 救災據點不曝露建立者個人資料；事件下架為 null，已合併則解析到最終事件。
#[sqlx::test(migrations = "../../db/migrations")]
async fn public_site_hides_owner_and_resolves_event(pool: PgPool) {
    let (_, o, _, a, s) = setup(&pool).await;
    let path = format!("/public/wishlists/{}", s.slug);
    let (_, v) = get(&pool, &path).await;
    assert!(
        v["owner"].is_null(),
        "relief page leaked the creator: {}",
        v["owner"]
    );
    let b = event(&pool, o).await;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1")
        .bind(a.0)
        .bind(b.0)
        .execute(&pool)
        .await
        .unwrap();
    let (_, v) = get(&pool, &path).await;
    assert_eq!(
        v["event"],
        json!({"slug": b.1, "name": "測試事件"}),
        "merged event not resolved: {v}"
    );
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(b.0)
    .execute(&pool)
    .await
    .unwrap();
    let (_, v) = get(&pool, &path).await;
    assert!(v["event"].is_null(), "hidden event exposed: {v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn explore_excludes_relief(pool: PgPool) {
    let (_, _, _, _, s) = setup(&pool).await;
    relief_item(&pool, s.id, "high", 1, 0).await;
    let (st, v) = get(&pool, "/public/wishlists").await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["wishlists"].as_array().unwrap().is_empty(), "{v}");
    let (_, v) = get(&pool, "/public/wishlists?type=relief").await;
    assert!(v["wishlists"].as_array().unwrap().is_empty(), "{v}");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn events_list_only_with_active_public_sites(pool: PgPool) {
    let (u, o, p, e, s) = setup(&pool).await;
    let (e2, _) = event(&pool, o).await; // 沒有據點
    let (e3, _) = event(&pool, o).await; // 只有暫停據點
    let paused = site(&pool, o, u.id, e3, p, "active").await;
    sqlx::query("UPDATE wishlists SET site_status='paused' WHERE id=$1")
        .bind(paused.id)
        .execute(&pool)
        .await
        .unwrap();
    let (st, v) = get(&pool, "/public/relief-events").await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let slugs: Vec<&str> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["slug"].as_str().unwrap())
        .collect();
    assert_eq!(slugs, vec![e.1.as_str()], "{v}");
    assert!(v["next_page"].is_null());
    assert_eq!(v["events"][0]["merged_into"], Value::Null);
    // 事件下架 → 不列
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(e.0)
    .execute(&pool)
    .await
    .unwrap();
    assert!(get(&pool, "/public/relief-events").await.1["events"]
        .as_array()
        .unwrap()
        .is_empty());
    let _ = (e2, s);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn event_detail_needs_summary(pool: PgPool) {
    let (_, _, _, e, s) = setup(&pool).await;
    relief_item(&pool, s.id, "critical", 5, 5).await; // 已認滿
    relief_item(&pool, s.id, "critical", 5, 2).await;
    relief_item(&pool, s.id, "low", 3, 0).await;
    relief_crowdfund(&pool, s.id, "high", "open").await; // 未達標 → 算
    relief_crowdfund(&pool, s.id, "high", "funded").await; // 已達標 → 不算
    let (st, v) = get(&pool, &format!("/public/relief-events/{}", e.1)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["merged_into"], Value::Null);
    assert_eq!(v["event"]["slug"], e.1.as_str());
    let site = &v["sites"][0];
    assert_eq!(site["slug"], s.slug.as_str());
    assert_eq!(site["org"]["display_name"], "測試協會");
    assert_eq!(
        site["needs"],
        json!({"critical": 1, "high": 1, "medium": 0, "low": 1}),
        "{v}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn event_detail_hidden_404(pool: PgPool) {
    let (_, _, _, e, _) = setup(&pool).await;
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(e.0)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        get(&pool, &format!("/public/relief-events/{}", e.1))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&pool, "/public/relief-events/nosuchslug").await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn merged_event_returns_merged_into(pool: PgPool) {
    let (u, o, _, a, _) = setup(&pool).await;
    let b = event(&pool, o).await;
    let c = event(&pool, o).await;
    let _ = u;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1")
        .bind(a.0)
        .bind(b.0)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1")
        .bind(b.0)
        .bind(c.0)
        .execute(&pool)
        .await
        .unwrap();
    let (st, v) = get(&pool, &format!("/public/relief-events/{}", a.1)).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["merged_into"].as_str(),
            v["event"].is_null(),
            v["sites"].as_array().unwrap().len()
        ),
        (Some(c.1.as_str()), true, 0)
    );
    // 最終事件本身正常
    let (_, v) = get(&pool, &format!("/public/relief-events/{}", c.1)).await;
    assert_eq!(
        (v["merged_into"].is_null(), v["event"]["slug"].as_str()),
        (true, Some(c.1.as_str()))
    );
    // 環不會無窮迴圈
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1")
        .bind(c.0)
        .bind(a.0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&pool, &format!("/public/relief-events/{}", a.1))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_public_page_only_verified(pool: PgPool) {
    let (u, o, _, _, s) = setup(&pool).await;
    let slug: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (st, v) = get(&pool, &format!("/public/orgs/{slug}")).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(
        (v["display_name"].as_str(), v["public_email"].as_str()),
        (Some("測試協會"), Some("org@example.com"))
    );
    assert_eq!(v["sites"][0]["slug"], s.slug.as_str());
    assert!(v.get("tax_id").is_none() && v.get("logo_key").is_none());
    let pend = org(&pool, u.id, "pending").await;
    let ps: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(pend)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        get(&pool, &format!("/public/orgs/{ps}")).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn permit_expiry_pauses_sites_and_purges(pool: PgPool) {
    let cdn = common::cdn::init();
    let (u, o, p, e, s) = setup(&pool).await;
    let s2 = site(&pool, o, u.id, e.0, p, "active").await;
    assert_eq!(
        wishly_api::relief_events::permit_expiry_tick(&pool)
            .await
            .unwrap(),
        0
    ); // 未過期
    sqlx::query("UPDATE org_permits SET valid_from=valid_from-10, valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date-1 WHERE id=$1").bind(p).execute(&pool).await.unwrap();
    assert_eq!(
        wishly_api::relief_events::permit_expiry_tick(&pool)
            .await
            .unwrap(),
        2
    );
    for sl in [&s, &s2] {
        let ss: String = sqlx::query_scalar("SELECT site_status FROM wishlists WHERE id=$1")
            .bind(sl.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ss, "paused");
        cdn.wait_for(&sl.slug).await;
    }
    cdn.wait_for_url(&common::cdn::event_url(&e.1)).await;
    // 兩個據點、同一許可 → 只通知一次；再跑一次不重發
    assert_eq!(
        wishly_api::relief_events::permit_expiry_tick(&pool)
            .await
            .unwrap(),
        0
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE kind='org.permit_expired' AND user_id=$1",
    )
    .bind(u.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unverified_org_or_draft_site_not_public(pool: PgPool) {
    let (u, o, p, e, s) = setup(&pool).await;
    let os: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    // 草稿據點（另一個事件）不外洩
    let (e2, e2s) = event(&pool, o).await;
    site(&pool, o, u.id, e2, p, "draft").await;
    assert_eq!(
        get(&pool, &format!("/public/relief-events/{e2s}")).await.1["sites"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let (_, v) = get(&pool, "/public/relief-events").await;
    assert_eq!(v["events"].as_array().unwrap().len(), 1, "{v}");
    // 繞過撤銷流程直接改成 rejected，據點仍是 active
    sqlx::query(
        "UPDATE organizations SET verification_status='rejected', verified_at=NULL WHERE id=$1",
    )
    .bind(o)
    .execute(&pool)
    .await
    .unwrap();
    let (_, v) = get(&pool, &format!("/public/relief-events/{}", e.1)).await;
    assert!(v["sites"].as_array().unwrap().is_empty(), "{v}");
    assert!(get(&pool, "/public/relief-events").await.1["events"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(
        get(&pool, &format!("/public/orgs/{os}")).await.0,
        StatusCode::NOT_FOUND
    );
    let (_, v) = get(&pool, &format!("/public/wishlists/{}", s.slug)).await;
    assert_eq!(v["org"]["verified"], false, "{v}");
}

#[test]
fn permit_expired_copy() {
    use wishly_api::notify::{render_lang, Lang};
    let p = json!({"org_id":"abc","display_name":"某會","permit_no":"P-9"});
    for (lang, needle) in [
        (Lang::Zh, "已到期"),
        (Lang::En, "expired"),
        (Lang::Vi, "hết hạn"),
        (Lang::Id, "kedaluwarsa"),
    ] {
        let (subj, body) = render_lang(lang, "org.permit_expired", "", &p);
        assert!(
            subj.contains(needle) && subj.contains("P-9") && body.contains("/orgs/abc"),
            "{subj} {body}"
        );
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_endpoints_ignore_cookie(pool: PgPool) {
    let (u, o, _, e, s) = setup(&pool).await;
    relief_item(&pool, s.id, "high", 2, 0).await;
    let os: String = sqlx::query_scalar("SELECT slug::text FROM organizations WHERE id=$1")
        .bind(o)
        .fetch_one(&pool)
        .await
        .unwrap();
    for path in [
        format!("/public/wishlists/{}", s.slug),
        "/public/relief-events".into(),
        format!("/public/relief-events/{}", e.1),
        format!("/public/orgs/{os}"),
        "/public/wishlists".into(),
    ] {
        let (s1, h1, b1) = call(&pool, "GET", &path, &[], None).await;
        let (s2, h2, b2) = call(&pool, "GET", &path, &ck(&u.token), None).await;
        assert_eq!((s1, &b1), (s2, &b2), "{path}");
        assert_eq!(h1.get("cache-control"), h2.get("cache-control"));
        assert_eq!(
            h1.get("cache-control").unwrap(),
            "public, s-maxage=10, stale-while-revalidate=60",
            "{path}"
        );
    }
}
