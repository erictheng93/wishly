//! 組織文件加密保存與勸募許可（需 MinIO 在跑）。
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::*;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const PDF: &[u8] = b"%PDF-1.4 secret-registration-content";

async fn up(pool: &PgPool, tok: &str, org: Uuid, ct: &str, body: Vec<u8>) -> (StatusCode, Value) {
    upk(pool, tok, org, "registration", ct, body).await
}
async fn upk(
    pool: &PgPool,
    tok: &str,
    org: Uuid,
    kind: &str,
    ct: &str,
    body: Vec<u8>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/orgs/{org}/documents?kind={kind}"))
        .header("cookie", format!("ws_session={tok}"))
        .header("content-type", ct)
        .body(Body::from(body))
        .unwrap();
    let (s, _, v) = call_request(&test_app(pool), req).await;
    (s, v)
}
async fn raw_get(
    pool: &PgPool,
    tok: &str,
    org: Uuid,
    doc: &str,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let req = Request::builder()
        .uri(format!("/api/v1/orgs/{org}/documents/{doc}"))
        .header("cookie", format!("ws_session={tok}"))
        .body(Body::empty())
        .unwrap();
    let res = test_app(pool).oneshot(req).await.unwrap();
    let (s, h) = (res.status(), res.headers().clone());
    (
        s,
        h,
        res.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}
fn ptr(v: &Value) -> &str {
    v["errors"][0]["pointer"].as_str().unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn upload_seals_bytes_at_rest(pool: PgPool) {
    let o = user(&pool, "O").await;
    let org = org(&pool, o.id, "draft").await;
    member(&pool, org, o.id, "owner").await;
    let (s, v) = up(&pool, &o.token, org, "application/pdf", PDF.to_vec()).await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let id = v["id"].as_str().unwrap();
    let key: String = sqlx::query_scalar("SELECT object_key FROM org_documents WHERE id=$1::uuid")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let stored = wishly_api::uploads::get_object(&key).await.unwrap();
    assert_ne!(stored, PDF);
    assert!(!stored.windows(6).any(|w| w == b"secret"));
    let (s, h, body) = raw_get(&pool, &o.token, org, id).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, PDF);
    assert_eq!(h["cache-control"], "no-store");
    assert_eq!(h["content-disposition"], "attachment");
    assert_eq!(h["x-content-type-options"], "nosniff");
    let n: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action='org.document.download'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(n, 1);
    let (_, _, show) = call(&pool, "GET", &format!("/orgs/{org}"), &ck(&o.token), None).await;
    assert_eq!(show["documents"][0]["id"], id);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn rejects_wrong_type_and_oversize(pool: PgPool) {
    let o = user(&pool, "O").await;
    let org = org(&pool, o.id, "draft").await;
    member(&pool, org, o.id, "owner").await;
    let (s, v) = up(&pool, &o.token, org, "text/html", b"<b>x</b>".to_vec()).await;
    assert_eq!(
        (s, ptr(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/content_type")
    );
    let (s, v) = up(
        &pool,
        &o.token,
        org,
        "application/pdf",
        vec![0; 10 * 1024 * 1024 + 1],
    )
    .await;
    assert_eq!(
        (s, ptr(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/content_length")
    );
    let (s, _) = up(
        &pool,
        &o.token,
        org,
        "application/pdf",
        vec![0; 10 * 1024 * 1024 + 2],
    )
    .await;
    assert_eq!(s, StatusCode::PAYLOAD_TOO_LARGE);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn download_denied_to_volunteer_and_strangers(pool: PgPool) {
    let o = user(&pool, "O").await;
    let vol = user(&pool, "V").await;
    let co = user(&pool, "C").await;
    let x = user(&pool, "X").await;
    let st = staff(&pool, "S").await;
    let org = org(&pool, o.id, "draft").await;
    member(&pool, org, o.id, "owner").await;
    member(&pool, org, vol.id, "volunteer").await;
    member(&pool, org, co.id, "coordinator").await;
    let (_, v) = up(&pool, &o.token, org, "application/pdf", PDF.to_vec()).await;
    let id = v["id"].as_str().unwrap();
    assert_eq!(
        raw_get(&pool, &vol.token, org, id).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        raw_get(&pool, &x.token, org, id).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(raw_get(&pool, &co.token, org, id).await.0, StatusCode::OK);
    assert_eq!(raw_get(&pool, &st.token, org, id).await.0, StatusCode::OK);
    // 非 owner 不可上傳
    assert_eq!(
        up(&pool, &co.token, org, "application/pdf", PDF.to_vec())
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn permit_requires_document_of_same_org(pool: PgPool) {
    let o = user(&pool, "O").await;
    let o2 = user(&pool, "P").await;
    let (a, b) = (
        org(&pool, o.id, "draft").await,
        org(&pool, o2.id, "draft").await,
    );
    member(&pool, a, o.id, "owner").await;
    member(&pool, b, o2.id, "owner").await;
    let (_, other) = up(&pool, &o2.token, b, "application/pdf", PDF.to_vec()).await;
    let body = |d: &Value| json!({"permit_no":"P-9","valid_from":"2026-01-01","valid_to":"2026-12-31","document_id":d});
    let (s, _, v) = call(
        &pool,
        "POST",
        &format!("/orgs/{a}/permits"),
        &ck(&o.token),
        Some(body(&other["id"])),
    )
    .await;
    assert_eq!(
        (s, ptr(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/document_id")
    );
    let (_, mine) = up(&pool, &o.token, a, "application/pdf", PDF.to_vec()).await;
    let (s, _, p) = call(
        &pool,
        "POST",
        &format!("/orgs/{a}/permits"),
        &ck(&o.token),
        Some(body(&mine["id"])),
    )
    .await;
    assert_eq!(
        (s, p["status"].as_str()),
        (StatusCode::CREATED, Some("pending")),
        "{p}"
    );
    let (_, _, l) = call(
        &pool,
        "GET",
        &format!("/orgs/{a}/permits"),
        &ck(&o.token),
        None,
    )
    .await;
    assert_eq!(l[0]["id"], p["id"]);
    // 被許可引用的文件不可刪
    let (s, _, v) = call(
        &pool,
        "DELETE",
        &format!("/orgs/{a}/documents/{}", mine["id"].as_str().unwrap()),
        &ck(&o.token),
        None,
    )
    .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (StatusCode::CONFLICT, Some("DOCUMENT_IN_USE"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn permit_dates_validated_and_delete_removes(pool: PgPool) {
    let o = user(&pool, "O").await;
    let org = org(&pool, o.id, "draft").await;
    member(&pool, org, o.id, "owner").await;
    let (_, d) = up(&pool, &o.token, org, "image/png", vec![1, 2, 3]).await;
    let (s, _, v) = call(&pool, "POST", &format!("/orgs/{org}/permits"), &ck(&o.token),
        Some(json!({"permit_no":"P","valid_from":"2026-05-01","valid_to":"2026-04-01","document_id":d["id"]}))).await;
    assert_eq!(
        (s, ptr(&v)),
        (StatusCode::UNPROCESSABLE_ENTITY, "/valid_to")
    );
    let (s, _, _) = call(
        &pool,
        "DELETE",
        &format!("/orgs/{org}/documents/{}", d["id"].as_str().unwrap()),
        &ck(&o.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert_eq!(
        raw_get(&pool, &o.token, org, d["id"].as_str().unwrap())
            .await
            .0,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn status_gates_uploads_and_permits(pool: PgPool) {
    let o = user(&pool, "O").await;
    let pb = |d: &Value| json!({"permit_no":"P","valid_from":"2026-01-01","valid_to":"2026-12-31","document_id":d});
    for st in ["pending", "rejected", "verified"] {
        let org = org(&pool, o.id, st).await;
        member(&pool, org, o.id, "owner").await;
        let (s, v) = upk(
            &pool,
            &o.token,
            org,
            "registration",
            "application/pdf",
            PDF.to_vec(),
        )
        .await;
        assert_eq!(
            (s, v["code"].as_str()),
            (StatusCode::CONFLICT, Some("ORG_NOT_EDITABLE")),
            "{st} registration"
        );
        let (s, v) = upk(
            &pool,
            &o.token,
            org,
            "permit",
            "application/pdf",
            PDF.to_vec(),
        )
        .await;
        let doc = if st == "verified" {
            assert_eq!(s, StatusCode::CREATED, "{v}");
            v["id"].clone()
        } else {
            assert_eq!(
                (s, v["code"].as_str()),
                (StatusCode::CONFLICT, Some("ORG_NOT_EDITABLE")),
                "{st} permit doc"
            );
            // 直接塞一份文件，驗證 permit 建立本身也被擋
            let d: Uuid = sqlx::query_scalar("INSERT INTO org_documents(org_id,kind,object_key,content_type,size_bytes,uploaded_by) VALUES($1,'permit','k','application/pdf',1,$2) RETURNING id")
                .bind(org).bind(o.id).fetch_one(&pool).await.unwrap();
            json!(d)
        };
        let (s, _, v) = call(
            &pool,
            "POST",
            &format!("/orgs/{org}/permits"),
            &ck(&o.token),
            Some(pb(&doc)),
        )
        .await;
        if st == "verified" {
            assert_eq!(s, StatusCode::CREATED, "{v}");
        } else {
            assert_eq!(
                (s, v["code"].as_str()),
                (StatusCode::CONFLICT, Some("ORG_NOT_EDITABLE")),
                "{st} permit"
            );
        }
    }
}
