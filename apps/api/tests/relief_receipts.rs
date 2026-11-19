//! 據點實收登記：權限、結束據點、跨組織、冪等與合計。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

struct Fx {
    org: Uuid,
    vol: TestUser,
    site: TestList,
    item: Uuid,
}
async fn fx(pool: &PgPool, status: &str) -> Fx {
    let owner = user(pool, "O").await;
    let vol = user(pool, "志工").await;
    let o = org(pool, owner.id, "verified").await;
    member(pool, o, owner.id, "owner").await;
    member(pool, o, vol.id, "volunteer").await;
    let p = permit(pool, o, "approved", 30).await;
    let (e, _) = event(pool, o).await;
    let site = site(pool, o, owner.id, e, p, "active").await;
    sqlx::query("UPDATE wishlists SET site_status=$2 WHERE id=$1")
        .bind(site.id)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    let item = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'礦泉水',100,'water','high') RETURNING id")
        .bind(site.id).fetch_one(pool).await.unwrap();
    Fx {
        org: o,
        vol,
        site,
        item,
    }
}
async fn rec(
    pool: &PgPool,
    f: &Fx,
    qty: i64,
    key: Uuid,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut h = ck(&f.vol.token);
    h.extend(idem(key));
    call(
        pool,
        "POST",
        &format!("/orgs/{}/receipts", f.org),
        &h,
        Some(json!({"item_id": f.item, "qty": qty, "note": "一箱"})),
    )
    .await
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn volunteer_records_receipt(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let (s, _, v) = rec(&pool, &f, 5, Uuid::new_v4()).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["qty"], 5);
    rec(&pool, &f, 7, Uuid::new_v4()).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/orgs/{}/sites/{}/receipts", f.org, f.site.id),
        &ck(&f.vol.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["site"]["id"], json!(f.site.id));
    assert_eq!(
        (
            v["site"]["slug"].as_str(),
            v["site"]["site_status"].as_str()
        ),
        (Some(f.site.slug.as_str()), Some("active"))
    );
    let it = &v["items"][0];
    assert_eq!(it["item_id"], json!(f.item));
    assert_eq!(
        (
            it["title"].as_str(),
            it["qty_needed"].as_i64(),
            it["category"].as_str(),
            it["urgency"].as_str()
        ),
        (Some("礦泉水"), Some(100), Some("water"), Some("high"))
    );
    assert_eq!(it["total"], 12);
    assert_eq!(it["entries"].as_array().unwrap().len(), 2);
    assert_eq!(it["entries"][0]["qty"], 7); // 新的在前
    assert_eq!(it["entries"][0]["recorded_by_name"], "志工");
}

/// 志工登記頁要列出全部品項：沒有實收的 total = 0、entries = []；已刪除的品項不列。
#[sqlx::test(migrations = "../../db/migrations")]
async fn overview_lists_items_without_receipts(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let empty: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency,sort_order) VALUES($1,'毛毯',5,'shelter','low',9) RETURNING id")
        .bind(f.site.id).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency,deleted_at) VALUES($1,'已刪',1,'food','low',now())").bind(f.site.id).execute(&pool).await.unwrap();
    rec(&pool, &f, 4, Uuid::new_v4()).await;
    let (s, _, v) = call(
        &pool,
        "GET",
        &format!("/orgs/{}/sites/{}/receipts", f.org, f.site.id),
        &ck(&f.vol.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{v}");
    assert_eq!(
        (items[0]["item_id"].clone(), items[0]["total"].clone()),
        (json!(f.item), json!(4))
    );
    assert_eq!(
        (
            items[1]["item_id"].clone(),
            items[1]["total"].clone(),
            items[1]["entries"].clone()
        ),
        (json!(empty), json!(0), json!([]))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn closed_site_rejects_but_paused_allows(pool: PgPool) {
    let f = fx(&pool, "closed").await;
    let (s, _, v) = rec(&pool, &f, 1, Uuid::new_v4()).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "SITE_CLOSED")
    );
    sqlx::query("UPDATE wishlists SET site_status='paused' WHERE id=$1")
        .bind(f.site.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        rec(&pool, &f, 1, Uuid::new_v4()).await.0,
        StatusCode::CREATED
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn item_of_other_org_404(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let g = fx(&pool, "active").await;
    let mut h = ck(&f.vol.token);
    h.extend(idem(Uuid::new_v4()));
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/orgs/{}/receipts", f.org),
        &h,
        Some(json!({"item_id": g.item, "qty": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // 非成員
    let (s, _, _) = call(
        &pool,
        "GET",
        &format!("/orgs/{}/sites/{}/receipts", g.org, g.site.id),
        &ck(&f.vol.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn idempotent_replay(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let k = Uuid::new_v4();
    let (_, h1, _) = rec(&pool, &f, 3, k).await;
    assert!(h1.get("idempotency-replayed").is_none());
    let (s, h2, _) = rec(&pool, &f, 3, k).await;
    assert_eq!(s, StatusCode::CREATED);
    assert!(h2.get("idempotency-replayed").is_some());
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM receipts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn support_item_not_receivable_nor_listed(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let other = user(&pool, "別人").await;
    let target = wishlist(&pool, other.id, ListOpts::default()).await;
    let sup: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,funding_mode,support_wishlist_id,category,urgency) VALUES($1,'支持',1,'support',$2,'food','high') RETURNING id")
        .bind(f.site.id).bind(target.id).fetch_one(&pool).await.unwrap();
    let mut h = ck(&f.vol.token);
    h.extend(idem(Uuid::new_v4()));
    let (s, _, _) = call(
        &pool,
        "POST",
        &format!("/orgs/{}/receipts", f.org),
        &h,
        Some(json!({"item_id": sup, "qty": 1})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, _, v) = call(
        &pool,
        "GET",
        &format!("/orgs/{}/sites/{}/receipts", f.org, f.site.id),
        &ck(&f.vol.token),
        None,
    )
    .await;
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
}

async fn list_site(pool: &PgPool, f: &Fx, tok: &str) -> Value {
    call(
        pool,
        "GET",
        &format!("/orgs/{}/sites/{}/receipts", f.org, f.site.id),
        &ck(tok),
        None,
    )
    .await
    .2
}
async fn void_call(
    pool: &PgPool,
    org: Uuid,
    rid: &str,
    tok: &str,
    body: Value,
) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "POST",
        &format!("/orgs/{org}/receipts/{rid}/void"),
        &ck(tok),
        Some(body),
    )
    .await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unverified_org_rejects_record(pool: PgPool) {
    let f = fx(&pool, "active").await;
    sqlx::query(
        "UPDATE organizations SET verification_status='rejected', verified_at=NULL WHERE id=$1",
    )
    .bind(f.org)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, v) = rec(&pool, &f, 1, Uuid::new_v4()).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "ORG_NOT_VERIFIED")
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn void_permissions_total_and_audit(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let other = user(&pool, "他人").await;
    member(&pool, f.org, other.id, "volunteer").await;
    let coord = user(&pool, "協調").await;
    member(&pool, f.org, coord.id, "coordinator").await;
    rec(&pool, &f, 5, Uuid::new_v4()).await;
    rec(&pool, &f, 7, Uuid::new_v4()).await;
    let v = list_site(&pool, &f, &f.vol.token).await;
    let e = &v["items"][0]["entries"];
    let (id7, id5) = (e[0]["id"].as_str().unwrap(), e[1]["id"].as_str().unwrap());
    assert_eq!(e[0]["can_void"], true);
    assert_eq!(
        list_site(&pool, &f, &other.token).await["items"][0]["entries"][0]["can_void"],
        false
    );
    // 他人 volunteer 不行；原因必填
    let (s, _) = void_call(&pool, f.org, id7, &other.token, json!({"reason": "打錯"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = void_call(&pool, f.org, id7, &f.vol.token, json!({"reason": "  "})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = void_call(
        &pool,
        f.org,
        id7,
        &f.vol.token,
        json!({"reason": "字".repeat(201)}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    // 本人可作廢；重複作廢 409
    let (s, _) = void_call(&pool, f.org, id7, &f.vol.token, json!({"reason": "打錯"})).await;
    assert_eq!(s, StatusCode::OK);
    let (s, v) = void_call(&pool, f.org, id7, &f.vol.token, json!({"reason": "再一次"})).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "ALREADY_VOIDED")
    );
    // coordinator 可作廢他人的；組織撤銷後仍可更正
    sqlx::query(
        "UPDATE organizations SET verification_status='rejected', verified_at=NULL WHERE id=$1",
    )
    .bind(f.org)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _) = void_call(&pool, f.org, id5, &coord.token, json!({"reason": "重複"})).await;
    assert_eq!(s, StatusCode::OK);
    let v = list_site(&pool, &f, &f.vol.token).await;
    assert_eq!(v["items"][0]["total"], 0);
    assert_eq!(v["site"]["org_status"], "rejected");
    assert_eq!(v["items"][0]["entries"][0]["void_reason"], "打錯");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='receipt.void'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 2);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn void_other_org_404(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let g = fx(&pool, "active").await;
    rec(&pool, &g, 3, Uuid::new_v4()).await;
    let rid: Uuid = sqlx::query_scalar("SELECT id FROM receipts WHERE wishlist_id=$1")
        .bind(g.site.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, _) = void_call(
        &pool,
        f.org,
        &rid.to_string(),
        &f.vol.token,
        json!({"reason": "x"}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn linked_receipt_notifies_only_its_donor_and_can_be_corrected(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let (token, claim) = user_claim(&pool, f.item, 2).await;
    let (_, other) = user_claim(&pool, f.item, 2).await;
    let path = format!("/orgs/{}/receipts", f.org);
    let body = json!({"item_id": f.item,"qty":2,"claim_id":claim,"note":"internal"});
    let headers = || [ck(&f.vol.token), idem(Uuid::new_v4())].concat();
    let wrong = json!({"item_id":f.item,"qty":1,"claim_id":claim});
    assert_eq!(
        call(&pool, "POST", &path, &headers(), Some(wrong)).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let h = headers();
    let (s, _, r) = call(&pool, "POST", &path, &h, Some(body.clone())).await;
    assert_eq!(s, StatusCode::CREATED, "{r}");
    assert_eq!(
        call(&pool, "POST", &path, &h, Some(body.clone())).await.0,
        StatusCode::CREATED
    );
    assert_eq!(
        call(&pool, "POST", &path, &headers(), Some(body.clone()))
            .await
            .0,
        StatusCode::CONFLICT,
        "同一認領不可重複登記實收"
    );
    let uid: Uuid = sqlx::query_scalar("SELECT user_id FROM claims WHERE id=$1")
        .bind(claim)
        .fetch_one(&pool)
        .await
        .unwrap();
    let notifications: Vec<(Uuid, Value)> = sqlx::query_as(
        "SELECT user_id,payload FROM notifications WHERE kind='claim.receipt_received'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        notifications,
        vec![(
            uid,
            json!({"receipt_id":r["id"],"claim_id":claim,"item_id":f.item,"wishlist_id":f.site.id})
        )]
    );
    let (_, _, me) = call(&pool, "GET", "/guest/me", &ck(&token), None).await;
    assert_eq!(me["claims"][0]["wishlist"]["type"], "relief");
    assert_eq!(me["claims"][0]["receipt_received"], true);
    assert!(!me.to_string().contains(&other.to_string()));
    let rid = r["id"].as_str().unwrap();
    let void = format!("/orgs/{}/receipts/{rid}/void", f.org);
    assert_eq!(
        call(
            &pool,
            "POST",
            &void,
            &ck(&f.vol.token),
            Some(json!({"reason":"error"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, _, me) = call(&pool, "GET", "/guest/me", &ck(&token), None).await;
    assert_eq!(me["claims"][0]["receipt_received"], false);
    wishly_api::notify::tick_at(&pool, noon()).await.unwrap();
    let suppressed: bool = sqlx::query_scalar("SELECT inbox_at IS NULL AND status='cancelled' FROM notifications WHERE kind='claim.receipt_received' AND payload->>'receipt_id'=$1")
        .bind(rid).fetch_one(&pool).await.unwrap();
    assert!(suppressed);

    assert_eq!(
        call(&pool, "POST", &path, &headers(), Some(body)).await.0,
        StatusCode::CREATED
    );
}

/// 交易持有指定的鎖時送出帶認領編號的實收登記，等它卡在該交易上；回傳 (交易, 進行中的請求)
async fn receipt_blocked_on(
    pool: &PgPool,
    f: &Fx,
    claim: Uuid,
    lock_sql: &str,
    target: Uuid,
) -> (
    sqlx::Transaction<'static, sqlx::Postgres>,
    tokio::task::JoinHandle<(StatusCode, axum::http::HeaderMap, Value)>,
) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(sqlx::AssertSqlSafe(lock_sql.to_string()))
        .bind(target)
        .execute(&mut *tx)
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let (p, path) = (pool.clone(), format!("/orgs/{}/receipts", f.org));
    let headers = [ck(&f.vol.token), idem(Uuid::new_v4())].concat();
    let body = json!({"item_id": f.item, "qty": 2, "claim_id": claim});
    let h = tokio::spawn(async move { call(&p, "POST", &path, &headers, Some(body)).await });
    wait_for_blocked(pool, pid, 1).await;
    (tx, h)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn linked_receipt_locks_item_before_claim(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let (_, claim) = user_claim(&pool, f.item, 2).await;
    // 模擬捐助者改認領：已鎖品項、接著要鎖認領
    let (mut tx, h) = receipt_blocked_on(
        &pool,
        &f,
        claim,
        "SELECT 1 FROM wishlist_items WHERE id=$1 FOR UPDATE",
        f.item,
    )
    .await;
    sqlx::query("SELECT 1 FROM claims WHERE id=$1 FOR UPDATE NOWAIT")
        .bind(claim)
        .execute(&mut *tx)
        .await
        .expect("實收登記在等品項鎖時已鎖住認領：與認捐鎖序相反，會死結");
    tx.commit().await.unwrap();
    assert_eq!(h.await.unwrap().0, StatusCode::CREATED);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn linked_receipt_waits_on_claim_lock_then_sees_cancel(pool: PgPool) {
    let f = fx(&pool, "active").await;
    let (_, claim) = user_claim(&pool, f.item, 2).await;
    let (tx, h) = receipt_blocked_on(
        &pool,
        &f,
        claim,
        "UPDATE claims SET status='cancelled', cancelled_at=now() WHERE id=$1",
        claim,
    )
    .await;
    tx.commit().await.unwrap();
    let (s, _, v) = h.await.unwrap();
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
}
