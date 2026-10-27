//! 救災據點建立，以及擁有權改由 can_manage_wishlist 判斷（組織 coordinator 可管理據點、停用者失權、個人清單不變）。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

struct Fx {
    org: Uuid,
    ev: Uuid,
    permit: Uuid,
    coord: TestUser,
}
/// verified 組織 + approved 許可 + 事件 + 一位 coordinator
async fn fx(pool: &PgPool) -> Fx {
    let owner = user(pool, "org-owner").await;
    let org = org(pool, owner.id, "verified").await;
    member(pool, org, owner.id, "owner").await;
    let coord = user(pool, "coord").await;
    member(pool, org, coord.id, "coordinator").await;
    let permit = permit(pool, org, "approved", 30).await;
    let (ev, _) = event(pool, org).await;
    Fx { org, ev, permit, coord }
}
fn body(f: &Fx) -> Value {
    json!({"title":"花蓮物資站","event_id":f.ev,"permit_id":f.permit,"site_address":"花蓮縣光復鄉中山路一號",
        "recipient_name":"王小明","recipient_phone":"0912-345-678"})
}
/// 以 k=v 覆寫預設 body
fn with(f: &Fx, k: &str, v: Value) -> Value {
    let mut b = body(f);
    b[k] = v;
    b
}
async fn create_site(pool: &PgPool, f: &Fx, tok: &str, b: Value) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", &format!("/orgs/{}/sites", f.org), &ck(tok), Some(b)).await;
    (s, v)
}
fn id(v: &Value) -> Uuid {
    v["id"].as_str().unwrap().parse().unwrap()
}
async fn relief_item(pool: &PgPool, wl: Uuid) -> Uuid {
    sqlx::query_scalar("INSERT INTO wishlist_items(wishlist_id,title,qty_needed,category,urgency) VALUES($1,'礦泉水',10,'water','high') RETURNING id")
        .bind(wl).fetch_one(pool).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn coordinator_creates_draft_site(pool: PgPool) {
    let f = fx(&pool).await;
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["type"].as_str(), v["visibility"].as_str(), v["status"].as_str()), (Some("relief"), Some("public"), Some("draft")));
    assert_eq!((v["claim_ttl_hours"].as_i64(), v["site_status"].as_str()), (Some(48), Some("active")));
    assert_eq!((v["org_id"].as_str().unwrap(), v["event_id"].as_str().unwrap()), (f.org.to_string().as_str(), f.ev.to_string().as_str()));
    assert_eq!(v["org"]["id"], json!(f.org));
    assert_eq!(v["org"]["display_name"], "測試協會");
    assert_eq!(v["site_address"], "花蓮縣光復鄉中山路一號");
    assert_eq!(v["has_shipping_address"], true);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM shipping_addresses WHERE wishlist_id=$1").bind(id(&v)).fetch_one(&pool).await.unwrap();
    assert_eq!(n, 1);
    let a: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='site.create' AND entity_id=$1").bind(id(&v)).fetch_one(&pool).await.unwrap();
    assert_eq!(a, 1);
    // 收件資訊與個人清單同一套加密，能由 address_get 讀回（遮罩）
    let (s, _, a) = call(&pool, "GET", &format!("/wishlists/{}/shipping-address", id(&v)), &ck(&f.coord.token), None).await;
    assert_eq!((s, a["has_shipping_address"].clone()), (StatusCode::OK, json!(true)));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn volunteer_cannot_create_site_and_outsider_gets_404(pool: PgPool) {
    let f = fx(&pool).await;
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    let (s, v) = create_site(&pool, &f, &vol.token, body(&f)).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::FORBIDDEN, "ORG_ROLE_REQUIRED"));
    let out = user(&pool, "out").await;
    assert_eq!(create_site(&pool, &f, &out.token, body(&f)).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_validates_permit_event_and_phone(pool: PgPool) {
    let f = fx(&pool).await;
    let other_org = org(&pool, f.coord.id, "verified").await;
    let foreign = permit(&pool, other_org, "approved", 30).await;
    let pending = permit(&pool, f.org, "pending", 30).await;
    let (s, v) = create_site(&pool, &f, &f.coord.token, with(&f, "permit_id", json!(foreign))).await;
    assert_eq!((s, v["errors"][0]["pointer"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "/permit_id"));
    let (s, v) = create_site(&pool, &f, &f.coord.token, with(&f, "permit_id", json!(pending))).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "PERMIT_INVALID"));
    sqlx::query("UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1").bind(f.ev).execute(&pool).await.unwrap();
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"));
    let (s, v) = create_site(&pool, &f, &f.coord.token, with(&f, "recipient_phone", json!("abc-defg"))).await;
    assert_eq!((s, v["errors"][0]["pointer"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "/recipient_phone"));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlists WHERE type='relief'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_sites_for_members_only(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    let (s, _, l) = call(&pool, "GET", &format!("/orgs/{}/sites", f.org), &ck(&vol.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!((l.as_array().unwrap().len(), l[0]["id"].clone(), l[0]["org"]["id"].clone()), (1, v["id"].clone(), json!(f.org)));
    let out = user(&pool, "out").await;
    assert_eq!(call(&pool, "GET", &format!("/orgs/{}/sites", f.org), &ck(&out.token), None).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn publish_requires_verified_org_and_valid_permit(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    relief_item(&pool, wid).await;
    let url = format!("/wishlists/{wid}");
    let tk = ck(&f.coord.token);
    let publish = || call(&pool, "PATCH", &url, &tk, Some(json!({"status":"active"})));
    sqlx::query("UPDATE organizations SET verification_status='pending', verified_at=NULL WHERE id=$1").bind(f.org).execute(&pool).await.unwrap();
    let (s, _, e) = publish().await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::FORBIDDEN, "ORG_NOT_VERIFIED"));
    sqlx::query("UPDATE organizations SET verification_status='verified', verified_at=now() WHERE id=$1").bind(f.org).execute(&pool).await.unwrap();
    sqlx::query("UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date-1 WHERE id=$1").bind(f.permit).execute(&pool).await.unwrap();
    let (s, _, e) = publish().await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::CONFLICT, "PERMIT_INVALID"));
    sqlx::query("UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date+5 WHERE id=$1").bind(f.permit).execute(&pool).await.unwrap();
    let (s, _, e) = publish().await;
    assert_eq!((s, e["status"].as_str()), (StatusCode::OK, Some("active")), "{e}");
    // 已上線後許可過期：編輯其他欄位回 409，不是 DB trigger 的 500
    sqlx::query("UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date-1 WHERE id=$1").bind(f.permit).execute(&pool).await.unwrap();
    let (s, _, e) = call(&pool, "PATCH", &url, &ck(&f.coord.token), Some(json!({"title":"改名"}))).await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::CONFLICT, "PERMIT_INVALID"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_immutable_and_patchable_fields(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let url = format!("/wishlists/{}", id(&v));
    let tk = ck(&f.coord.token);
    let patch = |b: Value| call(&pool, "PATCH", &url, &tk, Some(b));
    for (k, val) in [("visibility", json!("friends")), ("type", json!("registry")), ("org_id", json!(Uuid::new_v4()))] {
        let (s, _, e) = patch(json!({ k: val })).await;
        assert_eq!((s, e["errors"][0]["pointer"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, format!("/{k}").as_str()));
    }
    let (s, _, e) = patch(json!({"visibility":"public","site_status":"paused","site_address":"花蓮縣鳳林鎮中正路二號"})).await;
    assert_eq!((s, e["site_status"].as_str(), e["site_address"].as_str()), (StatusCode::OK, Some("paused"), Some("花蓮縣鳳林鎮中正路二號")), "{e}");
    let other = permit(&pool, org(&pool, f.coord.id, "verified").await, "approved", 30).await;
    let (s, _, _) = patch(json!({"permit_id": other})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (ev2, _) = event(&pool, f.org).await;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1").bind(f.ev).bind(ev2).execute(&pool).await.unwrap();
    let (s, _, _) = patch(json!({"event_id": f.ev})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, e) = patch(json!({"event_id": ev2})).await;
    assert_eq!((s, e["event_id"].clone()), (StatusCode::OK, json!(ev2)));
    // 個人清單不接受據點欄位
    let me = user(&pool, "p").await;
    let l = wishlist(&pool, me.id, ListOpts::default()).await;
    let (s, _, _) = call(&pool, "PATCH", &format!("/wishlists/{}", l.id), &ck(&me.token), Some(json!({"site_status":"paused"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn other_coordinator_can_edit_site(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    let item = relief_item(&pool, wid).await;
    let c2 = user(&pool, "coord2").await;
    member(&pool, f.org, c2.id, "coordinator").await;
    let (s, _, e) = call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&c2.token), Some(json!({"title":"新名稱"}))).await;
    assert_eq!((s, e["title"].as_str()), (StatusCode::OK, Some("新名稱")), "org coordinator lost site access: {e}");
    let (s, _, e) = call(&pool, "PATCH", &format!("/items/{item}"), &ck(&c2.token), Some(json!({"title":"瓶裝水"}))).await;
    assert_eq!((s, e["title"].as_str()), (StatusCode::OK, Some("瓶裝水")), "{e}");
    assert_eq!(call(&pool, "GET", &format!("/wishlists/{wid}"), &ck(&c2.token), None).await.0, StatusCode::OK);
    // volunteer 不能改
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    assert_eq!(call(&pool, "PATCH", &format!("/wishlists/{wid}"), &ck(&vol.token), Some(json!({"title":"x"}))).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn disabled_coordinator_gets_404_on_site_and_item(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    let item = relief_item(&pool, wid).await;
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE org_id=$1 AND user_id=$2").bind(f.org).bind(f.coord.id).execute(&pool).await.unwrap();
    let t = ck(&f.coord.token);
    assert_eq!(call(&pool, "PATCH", &format!("/wishlists/{wid}"), &t, Some(json!({"title":"x"}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "PATCH", &format!("/items/{item}"), &t, Some(json!({"title":"x"}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "GET", &format!("/wishlists/{wid}"), &t, None).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn coordinator_can_deliver_site_claims(pool: PgPool) {
    let f = fx(&pool).await;
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let item = relief_item(&pool, l.id).await;
    let (_, claim) = user_claim(&pool, item, 1).await;
    let c2 = user(&pool, "coord2").await;
    member(&pool, f.org, c2.id, "coordinator").await;
    let (s, _, e) = call(&pool, "PATCH", &format!("/claims/{claim}"), &ck(&c2.token), Some(json!({"status":"delivered"}))).await;
    assert_eq!(s, StatusCode::OK, "org coordinator cannot mark site claim delivered: {e}");
    // 非成員、停用者仍是 404
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE user_id=$1").bind(c2.id).execute(&pool).await.unwrap();
    let (_, claim2) = user_claim(&pool, item, 1).await;
    assert_eq!(call(&pool, "PATCH", &format!("/claims/{claim2}"), &ck(&c2.token), Some(json!({"status":"delivered"}))).await.0, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_sites_not_in_personal_list(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let personal = wishlist(&pool, f.coord.id, ListOpts::default()).await;
    let (s, _, l) = call(&pool, "GET", "/wishlists", &ck(&f.coord.token), None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = l["data"].as_array().unwrap().iter().map(|w| w["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![personal.id.to_string()], "org site {} leaked into personal list", v["id"]);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn personal_lists_unchanged(pool: PgPool) {
    let u = user(&pool, "p").await;
    let other = user(&pool, "q").await;
    let l = wishlist(&pool, u.id, ListOpts { status: Some("draft"), ..Default::default() }).await;
    let it = item(&pool, l.id, "杯子", 1).await;
    let (s, _, e) = call(&pool, "PATCH", &format!("/wishlists/{}", l.id), &ck(&u.token), Some(json!({"title":"新"}))).await;
    assert_eq!((s, e["org_id"].clone(), e["org"].clone(), e["type"].clone()), (StatusCode::OK, Value::Null, Value::Null, json!("registry")));
    assert_eq!(call(&pool, "PATCH", &format!("/items/{it}"), &ck(&u.token), Some(json!({"title":"碗"}))).await.0, StatusCode::OK);
    assert_eq!(call(&pool, "PATCH", &format!("/wishlists/{}", l.id), &ck(&other.token), Some(json!({"title":"x"}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "PATCH", &format!("/items/{it}"), &ck(&other.token), Some(json!({"title":"x"}))).await.0, StatusCode::NOT_FOUND);
}

/// 上線（PATCH status=active）須先 FOR SHARE 鎖組織列（organizations → wishlists）：
/// 撤銷端先鎖列並改成 rejected，上線端要等到提交後才看到，回 403 而非讓據點在被撤銷的組織下上線。
#[sqlx::test(migrations = "../../db/migrations")]
async fn publish_waits_on_org_lock_then_sees_revocation(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    relief_item(&pool, wid).await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *tx).await.unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE").bind(f.org).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("UPDATE organizations SET verification_status='rejected', verified_at=NULL WHERE id=$1").bind(f.org).execute(&mut *tx).await.unwrap();
    let (a2, uri, tok) = (app.clone(), format!("/wishlists/{wid}"), f.coord.token.clone());
    let h = tokio::spawn(async move { call_app(&a2, "PATCH", &uri, &ck(&tok), Some(json!({"status":"active"}))).await });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, e) = h.await.unwrap();
    assert_eq!((s, e["code"].as_str().unwrap_or("")), (StatusCode::FORBIDDEN, "ORG_NOT_VERIFIED"), "publish ignored the concurrent revocation: {e}");
    let st: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id=$1").bind(wid).fetch_one(&pool).await.unwrap();
    assert_eq!(st, "draft");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_on_merged_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    let (ev2, _) = event(&pool, f.org).await;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1").bind(f.ev).bind(ev2).execute(&pool).await.unwrap();
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_on_hidden_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    sqlx::query("UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1").bind(f.ev).execute(&pool).await.unwrap();
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn resume_site_under_hidden_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let url = format!("/wishlists/{}", id(&v));
    let tk = ck(&f.coord.token);
    let (s, _, _) = call(&pool, "PATCH", &url, &tk, Some(json!({"site_status":"paused"}))).await;
    assert_eq!(s, StatusCode::OK);
    sqlx::query("UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1").bind(f.ev).execute(&pool).await.unwrap();
    let (s, _, e) = call(&pool, "PATCH", &url, &tk, Some(json!({"site_status":"active"}))).await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"));
    // 事件恢復後可恢復營運；已 active 的據點編輯不受事件狀態影響
    sqlx::query("UPDATE relief_events SET moderation_status='ok', moderation_reason=NULL WHERE id=$1").bind(f.ev).execute(&pool).await.unwrap();
    let (s, _, e) = call(&pool, "PATCH", &url, &tk, Some(json!({"site_status":"active"}))).await;
    assert_eq!((s, e["site_status"].as_str()), (StatusCode::OK, Some("active")), "{e}");
}

/// lock_event 須 FOR SHARE：事件端（合併 / 下架）先鎖列並改成 hidden，建立據點要等到提交後才看到，回 422。
#[sqlx::test(migrations = "../../db/migrations")]
async fn create_waits_on_event_lock_then_sees_hidden(pool: PgPool) {
    let f = fx(&pool).await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *tx).await.unwrap();
    sqlx::query("SELECT id FROM relief_events WHERE id=$1 FOR UPDATE").bind(f.ev).fetch_one(&mut *tx).await.unwrap();
    sqlx::query("UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1").bind(f.ev).execute(&mut *tx).await.unwrap();
    let (a2, uri, tok, b) = (app.clone(), format!("/orgs/{}/sites", f.org), f.coord.token.clone(), body(&f));
    let h = tokio::spawn(async move { call_app(&a2, "POST", &uri, &ck(&tok), Some(b)).await });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, e) = h.await.unwrap();
    assert_eq!((s, e["code"].as_str().unwrap_or("")), (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"), "site created under a concurrently hidden event: {e}");
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlists WHERE type='relief'").fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0);
}
