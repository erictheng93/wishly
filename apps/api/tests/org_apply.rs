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
    let (s, v) = create(
        &pool,
        &u.token,
        json!({"legal_name":"某某協會","display_name":"某協會","contact_phone":"0912"}),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(v["role"], "owner");
    assert_eq!(v["contact_phone"], "0912");
    assert!(v.get("contact_phone_enc").is_none());
    let (_, _, me) = call(&pool, "GET", "/me", &ck(&u.token), None).await;
    assert_eq!(
        me["orgs"][0],
        json!({"id":v["id"],"name":"某協會","role":"owner","verification_status":"draft"})
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn second_draft_conflicts(pool: PgPool) {
    let u = user(&pool, "A").await;
    let b = json!({"legal_name":"甲","display_name":"甲"});
    assert_eq!(
        create(&pool, &u.token, b.clone()).await.0,
        StatusCode::CREATED
    );
    let (s, v) = create(&pool, &u.token, b).await;
    assert_eq!(
        (s, v["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "ORG_DRAFT_EXISTS")
    );
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
    call(
        &pool,
        "PATCH",
        &format!("/orgs/{id}"),
        &ck(&u.token),
        Some(json!({"tax_id":"12345675"})),
    )
    .await;
    let (s, e) = submit(&pool, &u.token, id).await;
    assert_eq!(
        (s, ptr(&e)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/documents")
    );
    add_docs(&pool, id, u.id, true, false).await;
    let (s, e) = submit(&pool, &u.token, id).await;
    assert_eq!((s, ptr(&e)), (StatusCode::UNPROCESSABLE_ENTITY, "/permits"));
    add_docs(&pool, id, u.id, false, true).await;
    // 上一輪的補件意見：重新送審後要清掉，staff 不會看到過期的備註
    sqlx::query("UPDATE organizations SET review_note='請補統編證明' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, v) = submit(&pool, &u.token, id).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["verification_status"], "pending");
    assert!(!v["submitted_at"].is_null());
    let note: Option<String> =
        sqlx::query_scalar("SELECT review_note FROM organizations WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(note, None);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn tax_id_taken_by_live_org(pool: PgPool) {
    let a = user(&pool, "A").await;
    let o = org(&pool, a.id, "pending").await;
    sqlx::query("UPDATE organizations SET tax_id='12345675' WHERE id=$1")
        .bind(o)
        .execute(&pool)
        .await
        .unwrap();
    let b = user(&pool, "B").await;
    let (s, e) = create(&pool, &b.token, full("12345675")).await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("TAX_ID_TAKEN"))
    );
    let (_, v) = create(
        &pool,
        &b.token,
        json!({"legal_name":"乙","display_name":"乙"}),
    )
    .await;
    let id = oid(&v);
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{id}"),
        &ck(&b.token),
        Some(json!({"tax_id":"12345675"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("TAX_ID_TAKEN"))
    );
    // 舊草稿仍可能在他人送審後衝突，送審索引保留最後防線。
    sqlx::query("UPDATE organizations SET tax_id='12345675' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{id}"),
        &ck(&b.token),
        Some(full("12345675")),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invalid_tax_id_checksum(pool: PgPool) {
    let u = user(&pool, "A").await;
    let (s, e) = create(&pool, &u.token, full("12345678")).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        (ptr(&e), e["errors"][0]["code"].as_str().unwrap()),
        ("/tax_id", "TAX_ID_CHECKSUM")
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn needs_changes_only_flagged_fields_editable(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "needs_changes").await;
    member(&pool, o, u.id, "owner").await;
    sqlx::query("UPDATE organizations SET flagged_fields='{intro}' WHERE id=$1")
        .bind(o)
        .execute(&pool)
        .await
        .unwrap();
    let uri = format!("/orgs/{o}");
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&u.token),
        Some(json!({"intro":"新"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&u.token),
        Some(json!({"legal_name":"改"})),
    )
    .await;
    assert_eq!(
        (s, e["errors"][0]["code"].as_str().unwrap()),
        (StatusCode::UNPROCESSABLE_ENTITY, "FIELD_LOCKED")
    );
}

/// 組織 Logo：沿用 `/uploads/presign`（purpose=cover）→ confirm 取得 key，再以 PATCH 掛上；可編輯規則同其他欄位。
#[sqlx::test(migrations = "../../db/migrations")]
async fn logo_follows_patch_rules(pool: PgPool) {
    let u = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    let key = || json!({ "logo_key": format!("covers/{}.png", Uuid::new_v4()) });
    let patch = |o: Uuid, tok: String, b: Value| {
        let pool = pool.clone();
        async move {
            let (s, _, v) = call(&pool, "PATCH", &format!("/orgs/{o}"), &ck(&tok), Some(b)).await;
            (s, v)
        }
    };
    // 上傳管線的型別 / 大小檢查
    for b in [
        json!({"purpose":"cover","content_type":"image/gif","content_length":10}),
        json!({"purpose":"cover","content_type":"image/png","content_length":6 * 1024 * 1024}),
    ] {
        let (s, _, e) = call(&pool, "POST", "/uploads/presign", &ck(&u.token), Some(b)).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    }
    // 建立時不收 Logo；草稿可掛上，key 須為圖片管線的 covers/ key
    let (s, _) = create(
        &pool,
        &u.token,
        json!({"legal_name":"甲","display_name":"甲","logo_key":"covers/x.png"}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (_, d) = create(
        &pool,
        &u.token,
        json!({"legal_name":"甲","display_name":"甲"}),
    )
    .await;
    let draft = oid(&d);
    member(&pool, draft, c.id, "coordinator").await;
    let (s, v) = patch(draft, u.token.clone(), key()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["logo_url"].is_string());
    let (s, e) = patch(draft, u.token.clone(), json!({"logo_key":"items/x.png"})).await;
    assert_eq!(
        (s, ptr(&e)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/logo_key")
    );
    assert_eq!(
        patch(draft, c.token.clone(), key()).await.0,
        StatusCode::FORBIDDEN
    );
    // needs_changes：未標記 → FIELD_LOCKED；標記 logo_key → 可改
    let nc = org(&pool, u.id, "needs_changes").await;
    member(&pool, nc, u.id, "owner").await;
    let (s, e) = patch(nc, u.token.clone(), key()).await;
    assert_eq!(
        (s, e["errors"][0]["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("FIELD_LOCKED"))
    );
    sqlx::query("UPDATE organizations SET flagged_fields='{logo_key}' WHERE id=$1")
        .bind(nc)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(patch(nc, u.token.clone(), key()).await.0, StatusCode::OK);
    // pending 不可改；verified 可改、不重審、可移除
    let p = org(&pool, u.id, "pending").await;
    member(&pool, p, u.id, "owner").await;
    assert_eq!(
        patch(p, u.token.clone(), key()).await.0,
        StatusCode::CONFLICT
    );
    let ok = org(&pool, u.id, "verified").await;
    member(&pool, ok, u.id, "owner").await;
    let (s, v) = patch(ok, u.token.clone(), key()).await;
    assert_eq!(
        (s, v["verification_status"].as_str()),
        (StatusCode::OK, Some("verified"))
    );
    let (s, v) = patch(ok, u.token.clone(), json!({"logo_key":null})).await;
    assert_eq!((s, v["logo_url"].is_null()), (StatusCode::OK, true));
}

/// 上傳者刪帳號時，仍被組織引用的 Logo 不可被退役。
#[sqlx::test(migrations = "../../db/migrations")]
async fn org_logo_survives_uploader_media_retire(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "verified").await;
    member(&pool, o, u.id, "owner").await;
    let key = format!("covers/{}.png", Uuid::new_v4());
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{o}"),
        &ck(&u.token),
        Some(json!({ "logo_key": key })),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    sqlx::query("UPDATE user_media SET retire_requested_at=now() WHERE object_key=$1")
        .bind(&key)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::uploads::drain_media(&pool).await;
    let deleted: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM user_media WHERE object_key=$1")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!deleted, "org logo was retired while still referenced");
}

/// 已通過的組織：owner 可維護簡介與聯絡方式，不重審；身分欄位仍鎖；非 owner 被拒；pending 不可改。
#[sqlx::test(migrations = "../../db/migrations")]
async fn verified_org_edits_profile_only(pool: PgPool) {
    let u = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    let o = org(&pool, u.id, "verified").await;
    member(&pool, o, u.id, "owner").await;
    member(&pool, o, c.id, "coordinator").await;
    let uri = format!("/orgs/{o}");
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&u.token),
        Some(json!({"intro":"新簡介","website":"https://example.org","contact_name":"新聯絡人",
            "contact_title":"幹事","contact_phone":"0911222333","public_email":"new@example.org","public_phone":"02-1234"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        (
            v["verification_status"].as_str(),
            v["intro"].as_str(),
            v["contact_phone"].as_str()
        ),
        (Some("verified"), Some("新簡介"), Some("0911222333"))
    );
    // 稽核只記欄位名，不含電話 / Email 內容
    let diff: Value = sqlx::query_scalar(
        "SELECT diff FROM audit_logs WHERE action='org.update' AND entity_id=$1",
    )
    .bind(o)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !diff.to_string().contains("0911222333") && !diff.to_string().contains("new@example.org")
    );
    for b in [
        json!({"legal_name":"改"}),
        json!({"tax_id":"04595257"}),
        json!({"intro":"x","display_name":"改"}),
    ] {
        let (s, _, e) = call(&pool, "PATCH", &uri, &ck(&u.token), Some(b)).await;
        assert_eq!(
            (s, e["errors"][0]["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("FIELD_LOCKED")),
            "{e}"
        );
    }
    let (s, _, _) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&c.token),
        Some(json!({"intro":"x"})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (status, intro): (String, String) =
        sqlx::query_as("SELECT verification_status, intro FROM organizations WHERE id=$1")
            .bind(o)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), intro.as_str()), ("verified", "新簡介"));
    let p = org(&pool, u.id, "pending").await;
    member(&pool, p, u.id, "owner").await;
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{p}"),
        &ck(&u.token),
        Some(json!({"intro":"x"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("ORG_NOT_EDITABLE"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn resubmit_blocked_until_resubmit_after(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "rejected").await;
    member(&pool, o, u.id, "owner").await;
    sqlx::query("UPDATE organizations SET resubmit_after=now()+interval '1 day' WHERE id=$1")
        .bind(o)
        .execute(&pool)
        .await
        .unwrap();
    let uri = format!("/orgs/{o}");
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&u.token),
        Some(json!({"intro":"x"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "RESUBMIT_TOO_EARLY")
    );
    sqlx::query("UPDATE organizations SET resubmit_after=now()-interval '1 day' WHERE id=$1")
        .bind(o)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _, v) = call(
        &pool,
        "PATCH",
        &uri,
        &ck(&u.token),
        Some(json!({"intro":"x"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["verification_status"], "draft");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn withdraw_returns_to_draft(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "pending").await;
    member(&pool, o, u.id, "owner").await;
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/orgs/{o}/withdraw"),
        &ck(&u.token),
        None,
    )
    .await;
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
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{o}"),
        &ck(&v.token),
        Some(json!({"intro":"x"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::FORBIDDEN, "ORG_ROLE_REQUIRED")
    );
    // 志工可讀，但看不到聯絡電話
    let (s, _, g) = call(&pool, "GET", &format!("/orgs/{o}"), &ck(&v.token), None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(g.get("contact_phone").is_none());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reopen_rejected_collides_with_existing_draft(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "rejected").await;
    member(&pool, o, u.id, "owner").await;
    org(&pool, u.id, "draft").await; // 同一人已有另一份草稿
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{o}"),
        &ck(&u.token),
        Some(json!({"intro":"x"})),
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str().unwrap()),
        (StatusCode::CONFLICT, "ORG_DRAFT_EXISTS")
    );
    let st: String =
        sqlx::query_scalar("SELECT verification_status FROM organizations WHERE id=$1")
            .bind(o)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(st, "rejected");
}

/// PATCH 的 lock() 必須 FOR UPDATE：審核端先鎖列並改成 rejected，申請端要等到提交後才看到新狀態。
#[sqlx::test(migrations = "../../db/migrations")]
async fn patch_waits_on_org_row_lock_then_sees_rejection(pool: PgPool) {
    let u = user(&pool, "A").await;
    let o = org(&pool, u.id, "draft").await;
    member(&pool, o, u.id, "owner").await;
    let app = test_app(&pool);
    let mut tx = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM organizations WHERE id=$1 FOR UPDATE")
        .bind(o)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE organizations SET verification_status='rejected', resubmit_after=now()+interval '30 days' WHERE id=$1")
        .bind(o).execute(&mut *tx).await.unwrap();
    let (a2, uri, tok) = (app.clone(), format!("/orgs/{o}"), u.token.clone());
    let h = tokio::spawn(async move {
        call_app(&a2, "PATCH", &uri, &ck(&tok), Some(json!({"intro":"x"}))).await
    });
    wait_for_blocked(&pool, pid, 1).await;
    tx.commit().await.unwrap();
    let (s, _, e) = h.await.unwrap();
    assert_eq!(
        (s, e["code"].as_str().unwrap_or("")),
        (StatusCode::CONFLICT, "RESUBMIT_TOO_EARLY"),
        "owner PATCH overwrote the concurrent rejection: {e}"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn draft_tax_id_does_not_reserve_and_submit_still_checks_uniqueness(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    let (s, oa) = create(&pool, &a.token, full("12345675")).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, ob) = create(&pool, &b.token, full("12345675")).await;
    assert_eq!(s, StatusCode::CREATED);
    for (o, u) in [(oid(&oa), &a), (oid(&ob), &b)] {
        add_docs(&pool, o, u.id, true, true).await;
    }
    assert_eq!(submit(&pool, &a.token, oid(&oa)).await.0, StatusCode::OK);
    let (s, e) = submit(&pool, &b.token, oid(&ob)).await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::CONFLICT, Some("TAX_ID_TAKEN"))
    );
    sqlx::query("UPDATE organizations SET verification_status='needs_changes', flagged_fields='{tax_id}' WHERE id=$1").bind(oid(&oa)).execute(&pool).await.unwrap();
    let (s, _, e) = call(
        &pool,
        "PATCH",
        &format!("/orgs/{}", oid(&oa)),
        &ck(&a.token),
        Some(json!({"tax_id":"12345675"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "self must not conflict: {e}");
}
