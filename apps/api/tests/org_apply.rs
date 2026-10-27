//! 組織申請：草稿、送審、撤回、/me orgs 與角色守衛。
mod common;
use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

fn full(tax: &str) -> Value {
    json!({"legal_name":"某某協會","display_name":"某協會","org_type":"association","tax_id":tax,
        "registration_no":"REG-9","authority":"內政部","rep_name":"代表","contact_name":"聯絡","contact_phone":"0912345678",
        "public_email":"a@example.com"})
}
async fn create(pool: &PgPool, tok: &str, b: Value) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", "/orgs", &ck(tok), Some(b)).await;
    (s, v)
}
async fn add_docs(pool: &PgPool, org: Uuid, user: Uuid, doc: bool, permit_row: bool) {
    if doc {
        sqlx::query("INSERT INTO org_documents(org_id,kind,object_key,content_type,size_bytes,uploaded_by) VALUES($1,'registration','k','application/pdf',1,$2)")
            .bind(org).bind(user).execute(pool).await.unwrap();
    }
    if permit_row {
        permit(pool, org, "pending", 30).await;
    }
}
async fn submit(pool: &PgPool, tok: &str, org: Uuid) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "POST", &format!("/orgs/{org}/submit"), &ck(tok), None).await;
    (s, v)
}
fn oid(v: &Value) -> Uuid {
    v["id"].as_str().unwrap().parse().unwrap()
}
fn ptr(v: &Value) -> &str {
    v["errors"][0]["pointer"].as_str().unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_draft_makes_caller_owner(pool: PgPool) {
    let u = user(&pool, "A").await;
    let (s, v) = create(&pool, &u.token, json!({"legal_name":"某某協會","display_name":"某協會","contact_phone":"0912"})).await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["role"], "owner");
    assert_eq!(v["contact_phone"], "0912");
    assert!(v.get("contact_phone_enc").is_none());
    let (_, _, me) = call(&pool, "GET", "/me", &ck(&u.token), None).await;
    assert_eq!(me["orgs"][0], json!({"id":v["id"],"name":"某協會","role":"owner","verification_status":"draft"}));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn second_draft_conflicts(pool: PgPool) {
    let u = user(&pool, "A").await;
    let b = json!({"legal_name":"甲","display_name":"甲"});
    assert_eq!(create(&pool, &u.token, b.clone()).await.0, StatusCode::CREATED);
    let (s, v) = create(&pool, &u.token, b).await;
    assert_eq!((s, v["code"].as_str().unwrap()), (StatusCode::CONFLICT, "ORG_DRAFT_EXISTS"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn submit_requires_fields_documents_permit(pool: PgPool) {
    let u = user(&pool, "A").await;
    let mut b = full("12345675");
    b.as_object_mut().unwrap().remove("tax_id");
    let (_, v) = create(&pool, &u.token, b).await;
    let id = oid(&v);
    let (s, e) = submit(&pool, &u.token, id).await;
    assert_eq!((s, ptr(&e)), (StatusCode::UNPROCESSABLE_ENTITY, "/tax_id"));
    call(&pool, "PATCH", &format!("/orgs/{id}"), &ck(&u.token), Some(json!({"tax_id":"12345675"}))).await;
    let (s, e) = submit(&pool, &u.token, id).await;
    assert_eq!((s, ptr(&e)), (StatusCode::UNPROCESSABLE_ENTITY, "/documents"));
    add_docs(&pool, id, u.id, true, false).await;
    let (s, e) = submit(&pool, &u.token, id).await;
    assert_eq!((s, ptr(&e)), (StatusCode::UNPROCESSABLE_ENTITY, "/permits"));
    add_docs(&pool, id, u.id, false, true).await;
    let (s, v) = submit(&pool, &u.token, id).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["verification_status"], "pending");
    assert!(!v["submitted_at"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tax_id_taken_by_live_org(pool: PgPool) {
    let a = user(&pool, "A").await;
    let o = org(&pool, a.id, "pending").await;
    sqlx::query("UPDATE organizations SET tax_id='12345675' WHERE id=$1").bind(o).execute(&pool).await.unwrap();
    let b = user(&pool, "B").await;
    let (_, v) = create(&pool, &b.token, full("12345675")).await;
    let id = oid(&v);
    add_docs(&pool, id, b.id, true, true).await;
    let (s, e) = submit(&pool, &b.token, id).await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::CONFLICT, "TAX_ID_TAKEN"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invalid_tax_id_checksum(pool: PgPool) {
    let u = user(&pool, "A").await;
    let (s, e) = create(&pool, &u.token, full("12345678")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!((ptr(&e), e["errors"][0]["code"].as_str().unwrap()), ("/tax_id", "TAX_ID_CHECKSUM"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn needs_changes_only_flagged_fields_editable(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "needs_changes").await;
    member(&pool, o, u.id, "owner").await;
    sqlx::query("UPDATE organizations SET flagged_fields='{intro}' WHERE id=$1").bind(o).execute(&pool).await.unwrap();
    let uri = format!("/orgs/{o}");
    let (s, _, _) = call(&pool, "PATCH", &uri, &ck(&u.token), Some(json!({"intro":"新"}))).await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, e) = call(&pool, "PATCH", &uri, &ck(&u.token), Some(json!({"legal_name":"改"}))).await;
    assert_eq!((s, e["errors"][0]["code"].as_str().unwrap()), (StatusCode::UNPROCESSABLE_ENTITY, "FIELD_LOCKED"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn resubmit_blocked_until_resubmit_after(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "rejected").await;
    member(&pool, o, u.id, "owner").await;
    sqlx::query("UPDATE organizations SET resubmit_after=now()+interval '1 day' WHERE id=$1").bind(o).execute(&pool).await.unwrap();
    let uri = format!("/orgs/{o}");
    let (s, _, e) = call(&pool, "PATCH", &uri, &ck(&u.token), Some(json!({"intro":"x"}))).await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::CONFLICT, "RESUBMIT_TOO_EARLY"));
    sqlx::query("UPDATE organizations SET resubmit_after=now()-interval '1 day' WHERE id=$1").bind(o).execute(&pool).await.unwrap();
    let (s, _, v) = call(&pool, "PATCH", &uri, &ck(&u.token), Some(json!({"intro":"x"}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["verification_status"], "draft");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_returns_to_draft(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "pending").await;
    member(&pool, o, u.id, "owner").await;
    let (s, _, v) = call(&pool, "POST", &format!("/orgs/{o}/withdraw"), &ck(&u.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["verification_status"], "draft");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn non_member_gets_404(pool: PgPool) {
    let a = user(&pool, "A").await;
    let o = org(&pool, a.id, "verified").await;
    member(&pool, o, a.id, "owner").await;
    let x = user(&pool, "X").await;
    let (s, _, _) = call(&pool, "GET", &format!("/orgs/{o}"), &ck(&x.token), None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn volunteer_cannot_patch(pool: PgPool) {
    let a = user(&pool, "A").await;
    let o = org(&pool, a.id, "needs_changes").await;
    member(&pool, o, a.id, "owner").await;
    let v = user(&pool, "V").await;
    member(&pool, o, v.id, "volunteer").await;
    let (s, _, e) = call(&pool, "PATCH", &format!("/orgs/{o}"), &ck(&v.token), Some(json!({"intro":"x"}))).await;
    assert_eq!((s, e["code"].as_str().unwrap()), (StatusCode::FORBIDDEN, "ORG_ROLE_REQUIRED"));
    // 志工可讀，但看不到聯絡電話
    let (s, _, g) = call(&pool, "GET", &format!("/orgs/{o}"), &ck(&v.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(g.get("contact_phone").is_none());
}
