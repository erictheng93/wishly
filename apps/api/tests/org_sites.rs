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
    Fx {
        org,
        ev,
        permit,
        coord,
    }
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
    let (s, _, v) = call(
        pool,
        "POST",
        &format!("/orgs/{}/sites", f.org),
        &ck(tok),
        Some(b),
    )
    .await;
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
    assert_eq!(
        (
            v["type"].as_str(),
            v["visibility"].as_str(),
            v["status"].as_str()
        ),
        (Some("relief"), Some("public"), Some("draft"))
    );
    assert_eq!(
        (v["claim_ttl_hours"].as_i64(), v["site_status"].as_str()),
        (Some(48), Some("active"))
    );
    assert_eq!(
        (
            v["org_id"].as_str().unwrap(),
            v["event_id"].as_str().unwrap()
        ),
        (f.org.to_string().as_str(), f.ev.to_string().as_str())
    );
    assert_eq!(v["org"]["id"], json!(f.org));
    assert_eq!(v["org"]["display_name"], "測試協會");
    assert_eq!(v["site_address"], "花蓮縣光復鄉中山路一號");
    assert_eq!(v["has_shipping_address"], true);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM shipping_addresses WHERE wishlist_id=$1")
        .bind(id(&v))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let a: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_logs WHERE action='site.create' AND entity_id=$1",
    )
    .bind(id(&v))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(a, 1);
    // 收件資訊與個人清單同一套加密，能由 address_get 讀回（遮罩）
    let (s, _, a) = call(
        &pool,
        "GET",
        &format!("/wishlists/{}/shipping-address", id(&v)),
        &ck(&f.coord.token),
        None,
    )
    .await;
    assert_eq!(
        (s, a["has_shipping_address"].clone()),
        (StatusCode::OK, json!(true))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn volunteer_cannot_create_site_and_outsider_gets_404(pool: PgPool) {
    let f = fx(&pool).await;
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    let (s, v) = create_site(&pool, &f, &vol.token, body(&f)).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::FORBIDDEN, "ORG_ROLE_REQUIRED")
    );
    let out = user(&pool, "out").await;
    assert_eq!(
        create_site(&pool, &f, &out.token, body(&f)).await.0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_validates_permit_event_and_phone(pool: PgPool) {
    let f = fx(&pool).await;
    let other_org = org(&pool, f.coord.id, "verified").await;
    let foreign = permit(&pool, other_org, "approved", 30).await;
    let pending = permit(&pool, f.org, "pending", 30).await;
    let (s, v) = create_site(
        &pool,
        &f,
        &f.coord.token,
        with(&f, "permit_id", json!(foreign)),
    )
    .await;
    assert_eq!(
        (s, v["errors"][0]["pointer"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "/permit_id")
    );
    let (s, v) = create_site(
        &pool,
        &f,
        &f.coord.token,
        with(&f, "permit_id", json!(pending)),
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "PERMIT_INVALID")
    );
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(f.ev)
    .execute(&pool)
    .await
    .unwrap();
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE")
    );
    let (s, v) = create_site(
        &pool,
        &f,
        &f.coord.token,
        with(&f, "recipient_phone", json!("abc-defg")),
    )
    .await;
    assert_eq!(
        (s, v["errors"][0]["pointer"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "/recipient_phone")
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlists WHERE type='relief'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_sites_for_members_only(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    let (s, _, l) = call(
        &pool,
        "GET",
        &format!("/orgs/{}/sites", f.org),
        &ck(&vol.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        (
            l.as_array().unwrap().len(),
            l[0]["id"].clone(),
            l[0]["org"]["id"].clone()
        ),
        (1, v["id"].clone(), json!(f.org))
    );
    let out = user(&pool, "out").await;
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/orgs/{}/sites", f.org),
            &ck(&out.token),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
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
    sqlx::query(
        "UPDATE organizations SET verification_status='pending', verified_at=NULL WHERE id=$1",
    )
    .bind(f.org)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = publish().await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::FORBIDDEN, "ORG_NOT_VERIFIED")
    );
    sqlx::query(
        "UPDATE organizations SET verification_status='verified', verified_at=now() WHERE id=$1",
    )
    .bind(f.org)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date-1 WHERE id=$1",
    )
    .bind(f.permit)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = publish().await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "PERMIT_INVALID")
    );
    sqlx::query(
        "UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date+5 WHERE id=$1",
    )
    .bind(f.permit)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = publish().await;
    assert_eq!(
        (s, e["status"].as_str()),
        (StatusCode::OK, Some("active")),
        "{e}"
    );
}

/// 已上線、許可已過期的據點：(PATCH url, wishlist id, fixture)
async fn expired_active_site(pool: &PgPool) -> (String, Uuid, Fx) {
    let f = fx(pool).await;
    let (_, v) = create_site(pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    relief_item(pool, wid).await;
    let url = format!("/wishlists/{wid}");
    let (s, _, e) = call(
        pool,
        "PATCH",
        &url,
        &ck(&f.coord.token),
        Some(json!({"status":"active"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    sqlx::query(
        "UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date-1 WHERE id=$1",
    )
    .bind(f.permit)
    .execute(pool)
    .await
    .unwrap();
    (url, wid, f)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn expired_permit_does_not_block_editing_pausing_or_closing_active_site(pool: PgPool) {
    let (url, wid, f) = expired_active_site(&pool).await;
    let tk = ck(&f.coord.token);
    let patch = |b: Value| call(&pool, "PATCH", &url, &tk, Some(b));
    let (s, _, e) = patch(json!({"title":"改名"})).await;
    assert_eq!(
        (s, e["title"].as_str()),
        (StatusCode::OK, Some("改名")),
        "{e}"
    );
    let (s, _, e) = patch(json!({"site_status":"paused"})).await;
    assert_eq!(
        (s, e["site_status"].as_str()),
        (StatusCode::OK, Some("paused")),
        "{e}"
    );
    let (s, _, e) = patch(json!({"site_status":"closed"})).await;
    assert_eq!(
        (s, e["site_status"].as_str()),
        (StatusCode::OK, Some("closed")),
        "{e}"
    );
    let (s, _, e) = patch(json!({"status":"closed"})).await;
    assert_eq!(
        (s, e["status"].as_str()),
        (StatusCode::OK, Some("closed")),
        "{e}"
    );
    // 重新上線仍要有效許可
    let (s, _, e) = patch(json!({"status":"active"})).await;
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::CONFLICT, "PERMIT_INVALID")
    );
    let st: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id=$1")
        .bind(wid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "closed");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn changing_permit_on_active_site_requires_a_valid_new_permit(pool: PgPool) {
    let (url, _, f) = expired_active_site(&pool).await;
    let tk = ck(&f.coord.token);
    let expired2 = permit(&pool, f.org, "approved", -2).await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"permit_id": expired2})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::CONFLICT, "PERMIT_INVALID")
    );
    let pending = permit(&pool, f.org, "pending", 30).await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"permit_id": pending})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::CONFLICT, "PERMIT_INVALID")
    );
    let fresh = permit(&pool, f.org, "approved", 30).await;
    let (s, _, e) = call(&pool, "PATCH", &url, &tk, Some(json!({"permit_id": fresh}))).await;
    assert_eq!(
        (s, e["permit_id"].clone()),
        (StatusCode::OK, json!(fresh)),
        "{e}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_immutable_and_patchable_fields(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let url = format!("/wishlists/{}", id(&v));
    let tk = ck(&f.coord.token);
    let patch = |b: Value| call(&pool, "PATCH", &url, &tk, Some(b));
    for (k, val) in [
        ("visibility", json!("friends")),
        ("type", json!("registry")),
        ("org_id", json!(Uuid::new_v4())),
    ] {
        let (s, _, e) = patch(json!({ k: val })).await;
        assert_eq!(
            (s, e["errors"][0]["pointer"].as_str().unwrap()),
            (StatusCode::UNPROCESSABLE_ENTITY, format!("/{k}").as_str())
        );
    }
    let (s, _, e) = patch(json!({"visibility":"public","site_status":"paused","site_address":"花蓮縣鳳林鎮中正路二號"})).await;
    assert_eq!(
        (s, e["site_status"].as_str(), e["site_address"].as_str()),
        (
            StatusCode::OK,
            Some("paused"),
            Some("花蓮縣鳳林鎮中正路二號")
        ),
        "{e}"
    );
    let other = permit(
        &pool,
        org(&pool, f.coord.id, "verified").await,
        "approved",
        30,
    )
    .await;
    let (s, _, _) = patch(json!({"permit_id": other})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (ev2, _) = event(&pool, f.org).await;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1")
        .bind(f.ev)
        .bind(ev2)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, _) = patch(json!({"event_id": f.ev})).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, e) = patch(json!({"event_id": ev2})).await;
    assert_eq!((s, e["event_id"].clone()), (StatusCode::OK, json!(ev2)));
    // 個人清單不接受據點欄位
    let me = user(&pool, "p").await;
    let l = wishlist(&pool, me.id, ListOpts::default()).await;
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", l.id),
        &ck(&me.token),
        Some(json!({"site_status":"paused"})),
    )
    .await;
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
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{wid}"),
        &ck(&c2.token),
        Some(json!({"title":"新名稱"})),
    )
    .await;
    assert_eq!(
        (s, e["title"].as_str()),
        (StatusCode::OK, Some("新名稱")),
        "org coordinator lost site access: {e}"
    );
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/items/{item}"),
        &ck(&c2.token),
        Some(json!({"title":"瓶裝水"})),
    )
    .await;
    assert_eq!(
        (s, e["title"].as_str()),
        (StatusCode::OK, Some("瓶裝水")),
        "{e}"
    );
    assert_eq!(
        call(
            &pool,
            "GET",
            &format!("/wishlists/{wid}"),
            &ck(&c2.token),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    // volunteer 不能改
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{wid}"),
            &ck(&vol.token),
            Some(json!({"title":"x"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn disabled_coordinator_gets_404_on_site_and_item(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    let item = relief_item(&pool, wid).await;
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE org_id=$1 AND user_id=$2")
        .bind(f.org)
        .bind(f.coord.id)
        .execute(&pool)
        .await
        .unwrap();
    let t = ck(&f.coord.token);
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{wid}"),
            &t,
            Some(json!({"title":"x"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/items/{item}"),
            &t,
            Some(json!({"title":"x"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&pool, "GET", &format!("/wishlists/{wid}"), &t, None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn coordinator_can_deliver_site_claims(pool: PgPool) {
    let f = fx(&pool).await;
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let item = relief_item(&pool, l.id).await;
    let (_, claim) = user_claim(&pool, item, 1).await;
    let c2 = user(&pool, "coord2").await;
    member(&pool, f.org, c2.id, "coordinator").await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/claims/{claim}"),
        &ck(&c2.token),
        Some(json!({"status":"delivered"})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "org coordinator cannot mark site claim delivered: {e}"
    );
    // 非成員、停用者仍是 404
    sqlx::query("UPDATE org_members SET disabled_at=now() WHERE user_id=$1")
        .bind(c2.id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, claim2) = user_claim(&pool, item, 1).await;
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/claims/{claim2}"),
            &ck(&c2.token),
            Some(json!({"status":"delivered"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_sites_not_in_personal_list(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let personal = wishlist(&pool, f.coord.id, ListOpts::default()).await;
    let (s, _, l) = call(&pool, "GET", "/wishlists", &ck(&f.coord.token), None).await;
    assert_eq!(s, StatusCode::OK);
    let ids: Vec<&str> = l["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![personal.id.to_string()],
        "org site {} leaked into personal list",
        v["id"]
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn personal_lists_unchanged(pool: PgPool) {
    let u = user(&pool, "p").await;
    let other = user(&pool, "q").await;
    let l = wishlist(
        &pool,
        u.id,
        ListOpts {
            status: Some("draft"),
            ..Default::default()
        },
    )
    .await;
    let it = item(&pool, l.id, "杯子", 1).await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", l.id),
        &ck(&u.token),
        Some(json!({"title":"新"})),
    )
    .await;
    assert_eq!(
        (s, e["org_id"].clone(), e["org"].clone(), e["type"].clone()),
        (StatusCode::OK, Value::Null, Value::Null, json!("registry"))
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/items/{it}"),
            &ck(&u.token),
            Some(json!({"title":"碗"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{}", l.id),
            &ck(&other.token),
            Some(json!({"title":"x"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/items/{it}"),
            &ck(&other.token),
            Some(json!({"title":"x"}))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
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
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(f.org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE organizations SET verification_status='rejected', verified_at=NULL WHERE id=$1",
    )
    .bind(f.org)
    .execute(&mut *tx)
    .await
    .unwrap();
    let (a2, uri, tok) = (
        app.clone(),
        format!("/wishlists/{wid}"),
        f.coord.token.clone(),
    );
    let h = tokio::spawn(async move {
        call_app(
            &a2,
            "PATCH",
            &uri,
            &ck(&tok),
            Some(json!({"status":"active"})),
        )
        .await
    });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, e) = h.await.unwrap();
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::FORBIDDEN, "ORG_NOT_VERIFIED"),
        "publish ignored the concurrent revocation: {e}"
    );
    let st: String = sqlx::query_scalar("SELECT status::text FROM wishlists WHERE id=$1")
        .bind(wid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "draft");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_on_merged_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    let (ev2, _) = event(&pool, f.org).await;
    sqlx::query("UPDATE relief_events SET merged_into=$2 WHERE id=$1")
        .bind(f.ev)
        .bind(ev2)
        .execute(&pool)
        .await
        .unwrap();
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE")
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_on_hidden_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(f.ev)
    .execute(&pool)
    .await
    .unwrap();
    let (s, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE")
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn resume_site_under_hidden_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let url = format!("/wishlists/{}", id(&v));
    let tk = ck(&f.coord.token);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"site_status":"paused"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(f.ev)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"site_status":"active"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE")
    );
    // 事件恢復後可恢復營運；已 active 的據點編輯不受事件狀態影響
    sqlx::query(
        "UPDATE relief_events SET moderation_status='ok', moderation_reason=NULL WHERE id=$1",
    )
    .bind(f.ev)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"site_status":"active"})),
    )
    .await;
    assert_eq!(
        (s, e["site_status"].as_str()),
        (StatusCode::OK, Some("active")),
        "{e}"
    );
}

/// lock_event 須 FOR SHARE：事件端（合併 / 下架）先鎖列並改成 hidden，建立據點要等到提交後才看到，回 422。
#[sqlx::test(migrations = "../../db/migrations")]
async fn create_waits_on_event_lock_then_sees_hidden(pool: PgPool) {
    let f = fx(&pool).await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM relief_events WHERE id=$1 FOR UPDATE")
        .bind(f.ev)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(f.ev)
    .execute(&mut *tx)
    .await
    .unwrap();
    let (a2, uri, tok, b) = (
        app.clone(),
        format!("/orgs/{}/sites", f.org),
        f.coord.token.clone(),
        body(&f),
    );
    let h = tokio::spawn(async move { call_app(&a2, "POST", &uri, &ck(&tok), Some(b)).await });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, e) = h.await.unwrap();
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"),
        "site created under a concurrently hidden event: {e}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM wishlists WHERE type='relief'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn site_has_no_allowed_users(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let url = format!("/wishlists/{}/allowed-users", id(&v));
    let tk = ck(&f.coord.token);
    assert_eq!(
        call(&pool, "GET", &url, &tk, None).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(&pool, "PUT", &url, &tk, Some(json!({"user_ids": []})))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn patching_site_address_updates_sealed_shipping_address(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let wid = id(&v);
    let url = format!("/wishlists/{wid}");
    let tk = ck(&f.coord.token);
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"site_address":"abcd"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &tk,
        Some(json!({"site_address":"花蓮縣鳳林鎮中正路二號"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    let (n, p, a): (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as("SELECT recipient_name_enc, phone_enc, address_enc FROM shipping_addresses WHERE wishlist_id=$1")
        .bind(wid).fetch_one(&pool).await.unwrap();
    assert_eq!(
        wishly_api::sealed::open_str(&a).as_deref(),
        Some("花蓮縣鳳林鎮中正路二號")
    );
    assert_eq!(wishly_api::sealed::open_str(&n).as_deref(), Some("王小明")); // 收件人、電話不動
    assert_eq!(
        wishly_api::sealed::open_str(&p).as_deref(),
        Some("0912-345-678")
    );
}

/// apply_site_fields 改事件前須先 FOR SHARE 鎖組織列（organizations → relief_events → wishlists），與撤銷驗證互斥。
#[sqlx::test(migrations = "../../db/migrations")]
async fn patching_event_waits_on_org_lock(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let (ev2, _) = event(&pool, f.org).await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(f.org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let (a2, uri, tok) = (
        app.clone(),
        format!("/wishlists/{}", id(&v)),
        f.coord.token.clone(),
    );
    let h = tokio::spawn(async move {
        call_app(
            &a2,
            "PATCH",
            &uri,
            &ck(&tok),
            Some(json!({"event_id": ev2})),
        )
        .await
    });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, e) = h.await.unwrap();
    assert_eq!(
        (s, e["event_id"].clone()),
        (StatusCode::OK, json!(ev2)),
        "{e}"
    );
}

async fn delete_me(pool: &PgPool, tok: &str) -> (StatusCode, Value) {
    let (s, _, v) = call(
        pool,
        "DELETE",
        "/me",
        &ck(tok),
        Some(json!({"confirm":"DELETE"})),
    )
    .await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn deleting_creator_hands_site_to_another_coordinator(pool: PgPool) {
    let _cdn = common::cdn::init();
    let f = fx(&pool).await;
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    relief_item(&pool, l.id).await;
    let (s, v) = delete_me(&pool, &f.coord.token).await;
    assert_eq!(
        (s, v["deleted"].clone()),
        (StatusCode::OK, json!(true)),
        "{v}"
    );
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM wishlists WHERE id=$1")
        .bind(l.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let org_owner: Uuid =
        sqlx::query_scalar("SELECT user_id FROM org_members WHERE org_id=$1 AND role='owner'")
            .bind(f.org)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        owner, org_owner,
        "owner role is preferred over other coordinators"
    );
    let (st, vis, ss): (String, String, String) = sqlx::query_as(
        "SELECT status::text, visibility::text, site_status FROM wishlists WHERE id=$1",
    )
    .bind(l.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (st.as_str(), vis.as_str(), ss.as_str()),
        ("active", "public", "active")
    );
    let dis: bool = sqlx::query_scalar(
        "SELECT disabled_at IS NOT NULL FROM org_members WHERE org_id=$1 AND user_id=$2",
    )
    .bind(f.org)
    .bind(f.coord.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(dis);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn deleting_sole_manager_closes_site_but_keeps_it_public(pool: PgPool) {
    let _cdn = common::cdn::init();
    let f = fx(&pool).await;
    // 組織的 owner 成員改成 volunteer：coord 是唯一能管理的人
    sqlx::query("UPDATE org_members SET role='volunteer' WHERE org_id=$1 AND role='owner'")
        .bind(f.org)
        .execute(&pool)
        .await
        .unwrap();
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    relief_item(&pool, l.id).await;
    let (s, v) = delete_me(&pool, &f.coord.token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (owner, st, vis, ss): (Uuid, String, String, String) = sqlx::query_as(
        "SELECT owner_id, status::text, visibility::text, site_status FROM wishlists WHERE id=$1",
    )
    .bind(l.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (owner, st.as_str(), vis.as_str(), ss.as_str()),
        (f.coord.id, "closed", "public", "closed")
    );
    let dis: bool = sqlx::query_scalar(
        "SELECT disabled_at IS NOT NULL FROM org_members WHERE org_id=$1 AND user_id=$2",
    )
    .bind(f.org)
    .bind(f.coord.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(dis);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn last_manager_deleting_closes_all_org_sites(pool: PgPool) {
    let _cdn = common::cdn::init();
    let f = fx(&pool).await;
    // 只留兩位 coordinator 當管理者；原 owner 降為 volunteer
    sqlx::query("UPDATE org_members SET role='volunteer' WHERE org_id=$1 AND role='owner'")
        .bind(f.org)
        .execute(&pool)
        .await
        .unwrap();
    let c2 = user(&pool, "coord2").await;
    member(&pool, f.org, c2.id, "coordinator").await;
    let vol: Uuid =
        sqlx::query_scalar("SELECT user_id FROM org_members WHERE org_id=$1 AND role='volunteer'")
            .bind(f.org)
            .fetch_one(&pool)
            .await
            .unwrap();
    let s1 = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let s2 = site(&pool, f.org, vol, f.ev, f.permit, "active").await; // 擁有者不是刪帳號的人
    let (s, v) = delete_me(&pool, &f.coord.token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let status = |id: Uuid| {
        sqlx::query_as::<_, (String, String)>(
            "SELECT status::text, visibility::text FROM wishlists WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&pool)
    };
    assert_eq!(
        status(s1.id).await.unwrap(),
        ("active".into(), "public".into()),
        "still has a coordinator"
    );
    assert_eq!(status(s2.id).await.unwrap().0, "active");
    let (s, v) = delete_me(&pool, &c2.token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    for id in [s1.id, s2.id] {
        assert_eq!(
            status(id).await.unwrap(),
            ("closed".into(), "public".into())
        );
        let ss: String = sqlx::query_scalar("SELECT site_status FROM wishlists WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(ss, "closed");
    }
}

/// delete_me 須 FOR UPDATE 鎖本人所屬的組織列：同組織管理者同時刪帳號被序列化。
#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_me_waits_on_org_lock(pool: PgPool) {
    let _cdn = common::cdn::init();
    let f = fx(&pool).await;
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(f.org)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    let (a2, tok) = (app.clone(), f.coord.token.clone());
    let h = tokio::spawn(async move {
        call_app(
            &a2,
            "DELETE",
            "/me",
            &ck(&tok),
            Some(json!({"confirm":"DELETE"})),
        )
        .await
    });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, v) = h.await.unwrap();
    assert_eq!(s, StatusCode::OK, "{v}");
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM wishlists WHERE id=$1")
        .bind(l.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(owner, f.coord.id);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn address_put_rejected_on_relief(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let (s, _, _) = call(
        &pool,
        "PUT",
        &format!("/wishlists/{}/shipping-address", id(&v)),
        &ck(&f.coord.token),
        Some(
            json!({"recipient_name":"某人","phone":"0912345678","address":"台北市中正區某路一號"}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
}

/// 據點的圖片不因建立者刪帳號而被清掉：退役檢查看到組織據點引用就保留。
#[sqlx::test(migrations = "../../db/migrations")]
async fn org_site_images_survive_creator_deletion(pool: PgPool) {
    let _cdn = common::cdn::init();
    let f = fx(&pool).await;
    sqlx::query("UPDATE org_members SET role='volunteer' WHERE org_id=$1 AND role='owner'")
        .bind(f.org)
        .execute(&pool)
        .await
        .unwrap();
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let key = format!("covers/{}.jpg", Uuid::new_v4());
    sqlx::query("UPDATE wishlists SET cover_image_key=$2, cover_image_status='ready' WHERE id=$1")
        .bind(l.id)
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_media (user_id, object_key) VALUES ($1,$2)")
        .bind(f.coord.id)
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();
    let (s, v) = delete_me(&pool, &f.coord.token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    wishly_api::uploads::drain_media(&pool).await;
    let (req, deleted): (bool, bool) = sqlx::query_as("SELECT retire_requested_at IS NOT NULL, deleted_at IS NOT NULL FROM user_media WHERE object_key=$1")
        .bind(&key).fetch_one(&pool).await.unwrap();
    assert_eq!(
        (req, deleted),
        (false, false),
        "org site cover must not be retired when its creator is deleted"
    );
    let k: Option<String> = sqlx::query_scalar("SELECT cover_image_key FROM wishlists WHERE id=$1")
        .bind(l.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(k.as_deref(), Some(key.as_str()));
}

/// 恢復營運（site_status 非 active → active）要用 PATCH 後的許可重驗：撤銷 / 過期 / 組織失格都不能復活據點。
#[sqlx::test(migrations = "../../db/migrations")]
async fn resume_rechecks_permit_and_org(pool: PgPool) {
    let f = fx(&pool).await;
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    relief_item(&pool, l.id).await;
    let url = format!("/wishlists/{}", l.id);
    let tk = ck(&f.coord.token);
    let patch = |b: Value| call(&pool, "PATCH", &url, &tk, Some(b));
    let (s, _, e) = patch(json!({"site_status":"paused"})).await;
    assert_eq!(s, StatusCode::OK, "{e}");
    sqlx::query("UPDATE org_permits SET status='rejected' WHERE id=$1")
        .bind(f.permit)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, e) = patch(json!({"site_status":"active"})).await;
    // 不用 assert_eq（失敗時會印出 500 的 INTERNAL_ERROR，被 mutate.py 當成基礎設施失敗）
    assert!(
        s == StatusCode::CONFLICT && e["code"] == "PERMIT_INVALID",
        "resume ignored the revoked permit (status {s})"
    );
    // 資料庫 trigger 也擋：直接把 site_status 改回 active
    let err = sqlx::query("UPDATE wishlists SET site_status='active' WHERE id=$1")
        .bind(l.id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        err.as_database_error().and_then(|d| d.code()).as_deref(),
        Some("23514"),
        "{err}"
    );
    // 過期
    sqlx::query("UPDATE org_permits SET status='approved', valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date-1 WHERE id=$1").bind(f.permit).execute(&pool).await.unwrap();
    assert_eq!(
        patch(json!({"site_status":"active"})).await.0,
        StatusCode::CONFLICT
    );
    // 組織未通過審核
    sqlx::query(
        "UPDATE org_permits SET valid_to=(now() AT TIME ZONE 'Asia/Taipei')::date+5 WHERE id=$1",
    )
    .bind(f.permit)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE organizations SET verification_status='pending', verified_at=NULL WHERE id=$1",
    )
    .bind(f.org)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = patch(json!({"site_status":"active"})).await;
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::FORBIDDEN, "ORG_NOT_VERIFIED"),
        "{e}"
    );
    sqlx::query(
        "UPDATE organizations SET verification_status='verified', verified_at=now() WHERE id=$1",
    )
    .bind(f.org)
    .execute(&pool)
    .await
    .unwrap();
    let (s, _, e) = patch(json!({"site_status":"active"})).await;
    assert_eq!(
        (s, e["site_status"].as_str()),
        (StatusCode::OK, Some("active")),
        "{e}"
    );
}

/// 草稿據點在事件下架後不能上線（與重開 / 建立一致：lock_event 檢查）。
#[sqlx::test(migrations = "../../db/migrations")]
async fn publish_draft_under_hidden_event_is_422(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    relief_item(&pool, id(&v)).await;
    sqlx::query(
        "UPDATE relief_events SET moderation_status='hidden', moderation_reason='x' WHERE id=$1",
    )
    .bind(f.ev)
    .execute(&pool)
    .await
    .unwrap();
    let url = format!("/wishlists/{}", id(&v));
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&f.coord.token),
        Some(json!({"status":"active"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::UNPROCESSABLE_ENTITY, "EVENT_UNAVAILABLE"),
        "{e}"
    );
}

/// PATCH 讀到的 site_status / event_id 要在 UPDATE 時比對：並行的撤銷 / 下架（暫停）或事件合併改過就回 409，
/// 不可把舊值寫回（否則改標題會讓被暫停的據點復活、或掛回已合併的事件）。
#[sqlx::test(migrations = "../../db/migrations")]
async fn patch_overwriting_concurrent_site_status_or_event_is_409(pool: PgPool) {
    let f = fx(&pool).await;
    let (ev2, _) = event(&pool, f.org).await;
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let app = test_app(&pool);
    for change in [
        "site_status='paused'".to_string(),
        format!("event_id='{ev2}'"),
    ] {
        let mut tx = pool.begin().await.unwrap();
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "UPDATE wishlists SET {change} WHERE id=$1"
        )))
        .bind(l.id)
        .execute(&mut *tx)
        .await
        .unwrap();
        let (a2, uri, tok) = (
            app.clone(),
            format!("/wishlists/{}", l.id),
            f.coord.token.clone(),
        );
        let h = tokio::spawn(async move {
            call_app(
                &a2,
                "PATCH",
                &uri,
                &ck(&tok),
                Some(json!({"title":"改標題"})),
            )
            .await
        });
        wait_for_blocked(&pool, pid, 1).await;
        tx.commit().await.unwrap();
        let (s, _, e) = h.await.unwrap();
        assert_eq!(
            (s, e["code"].as_str().unwrap_or("")),
            (StatusCode::CONFLICT, "STALE_VERSION"),
            "PATCH overwrote a concurrent {change}: {e}"
        );
    }
    let (t, ss, ev): (String, String, Uuid) =
        sqlx::query_as("SELECT title, site_status, event_id FROM wishlists WHERE id=$1")
            .bind(l.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((t.as_str(), ss.as_str(), ev), ("據點", "paused", ev2));
}

/// 最後一位 owner 刪帳號：最早加入的 coordinator 升為 owner，組織不會沒有 owner。
#[sqlx::test(migrations = "../../db/migrations")]
async fn deleting_last_owner_promotes_earliest_coordinator(pool: PgPool) {
    let _cdn = common::cdn::init();
    let owner = user(&pool, "owner").await;
    let o = org(&pool, owner.id, "verified").await;
    member(&pool, o, owner.id, "owner").await;
    let (c1, c2) = (user(&pool, "c1").await, user(&pool, "c2").await);
    member(&pool, o, c1.id, "coordinator").await;
    member(&pool, o, c2.id, "coordinator").await;
    let (s, v) = delete_me(&pool, &owner.token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let roles: Vec<(Uuid, String)> = sqlx::query_as("SELECT user_id, role::text FROM org_members WHERE org_id=$1 AND disabled_at IS NULL ORDER BY created_at").bind(o).fetch_all(&pool).await.unwrap();
    assert_eq!(
        roles,
        vec![
            (c1.id, "owner".to_string()),
            (c2.id, "coordinator".to_string())
        ]
    );
}

/// 建立者帳號刪除後據點被關閉；新 coordinator 重開時 owner_id 改記操作者，否則認捐查不到 owner 而 404。
#[sqlx::test(migrations = "../../db/migrations")]
async fn reopening_site_with_deleted_creator_takes_over_owner(pool: PgPool) {
    let _cdn = common::cdn::init();
    let f = fx(&pool).await;
    sqlx::query("UPDATE org_members SET role='volunteer' WHERE org_id=$1 AND role='owner'")
        .bind(f.org)
        .execute(&pool)
        .await
        .unwrap();
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let item = relief_item(&pool, l.id).await;
    let (s, v) = delete_me(&pool, &f.coord.token).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let c2 = user(&pool, "coord2").await;
    member(&pool, f.org, c2.id, "coordinator").await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", l.id),
        &ck(&c2.token),
        Some(json!({"status":"active","site_status":"active"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM wishlists WHERE id=$1")
        .bind(l.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(owner, c2.id);
    user_claim(&pool, item, 1).await; // 201 由 helper 斷言
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn relief_site_rejects_show_claimer_names(pool: PgPool) {
    let f = fx(&pool).await;
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let url = format!("/wishlists/{}", id(&v));
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &url,
        &ck(&f.coord.token),
        Some(json!({"show_claimer_names":true})),
    )
    .await;
    assert_eq!(
        (
            s,
            e["errors"][0]["pointer"].as_str(),
            e["errors"][0]["code"].as_str()
        ),
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some("/show_claimer_names"),
            Some("NOT_APPLICABLE")
        ),
        "{e}"
    );
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &url,
            &ck(&f.coord.token),
            Some(json!({"show_claimer_names":false}))
        )
        .await
        .0,
        StatusCode::OK
    );
}

/// 據點 PATCH 改營運狀態後，據點頁與所屬事件頁（含換事件前的舊事件）的快取要清。
#[sqlx::test(migrations = "../../db/migrations")]
async fn site_patch_purges_site_and_events(pool: PgPool) {
    let cdn = common::cdn::init();
    let f = fx(&pool).await;
    let (ev2, ev2_slug) = event(&pool, f.org).await;
    let ev1_slug: String = sqlx::query_scalar("SELECT slug::text FROM relief_events WHERE id=$1")
        .bind(f.ev)
        .fetch_one(&pool)
        .await
        .unwrap();
    let l = site(&pool, f.org, f.coord.id, f.ev, f.permit, "active").await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/wishlists/{}", l.id),
        &ck(&f.coord.token),
        Some(json!({"site_status":"paused","event_id":ev2})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{e}");
    cdn.wait_for(&l.slug).await;
    cdn.wait_for_url(&common::cdn::event_url(&ev1_slug)).await;
    cdn.wait_for_url(&common::cdn::event_url(&ev2_slug)).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn apply_permit_rebinds_only_open_sites_with_invalid_permits(pool: PgPool) {
    let (_, wid, f) = expired_active_site(&pool).await;
    let tk = ck(&f.coord.token);
    // 已關閉的據點、以及綁著有效許可的據點都不動
    let valid = permit(&pool, f.org, "approved", 30).await;
    let (_, v) = create_site(
        &pool,
        &f,
        &f.coord.token,
        with(&f, "permit_id", json!(valid)),
    )
    .await;
    let keep = id(&v);
    let (_, v) = create_site(&pool, &f, &f.coord.token, body(&f)).await;
    let closed = id(&v);
    sqlx::query("UPDATE wishlists SET status='closed' WHERE id=$1")
        .bind(closed)
        .execute(&pool)
        .await
        .unwrap();
    let apply = |p: Uuid, tok: Hdr| {
        let (pool, org) = (pool.clone(), f.org);
        async move {
            call(
                &pool,
                "POST",
                &format!("/orgs/{org}/permits/{p}/apply"),
                &tok,
                None,
            )
            .await
        }
    };
    let pending = permit(&pool, f.org, "pending", 30).await;
    let (s, _, e) = apply(pending, tk.clone()).await;
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::CONFLICT, "PERMIT_INVALID")
    );
    let vol = user(&pool, "vol").await;
    member(&pool, f.org, vol.id, "volunteer").await;
    let fresh = permit(&pool, f.org, "approved", 60).await;
    let (s, _, _) = apply(fresh, ck(&vol.token)).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, e) = apply(fresh, tk.clone()).await;
    assert_eq!((s, e["updated"].as_i64()), (StatusCode::OK, Some(1)), "{e}");
    let bound = |w: Uuid| {
        sqlx::query_scalar::<_, Uuid>("SELECT permit_id FROM wishlists WHERE id=$1")
            .bind(w)
            .fetch_one(&pool)
    };
    assert_eq!(
        (
            bound(wid).await.unwrap(),
            bound(keep).await.unwrap(),
            bound(closed).await.unwrap()
        ),
        (fresh, valid, f.permit)
    );
    let (s, _, e) = apply(fresh, tk).await; // 重按沒有可換的
    assert_eq!((s, e["updated"].as_i64()), (StatusCode::OK, Some(0)));
}
