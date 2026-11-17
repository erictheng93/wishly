//! 系統公告（#96）：後台建立 / 停用、/public/status 帶出生效中的公告。
mod common;
use axum::http::StatusCode;
use common::{call, staff};
use serde_json::{json, Value};
use sqlx::PgPool;

fn bearer(t: &str) -> [(&'static str, String); 1] {
    [("authorization", format!("Bearer {t}"))]
}

async fn status_notices(pool: &PgPool) -> Vec<Value> {
    let (st, _, b) = call(pool, "GET", "/public/status", &[], None).await;
    assert_eq!(st, StatusCode::OK);
    b["notices"].as_array().unwrap().clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_show_disable_and_audit(pool: PgPool) {
    let a = staff(&pool, "admin").await;
    let h = bearer(&a.token);
    assert!(status_notices(&pool).await.is_empty());

    let (st, _, n) = call(
        &pool,
        "POST",
        "/admin/notices",
        &h,
        Some(json!({"title":" 維護 ","body":"今晚","level":"critical","locale":"en"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{n}");
    assert_eq!(n["title"], "維護");
    assert_eq!(n["locale"], "en");
    assert_eq!(n["level"], "critical");
    let id = n["id"].as_str().unwrap().to_string();
    // 較低嚴重度但較新的公告排在後面
    call(
        &pool,
        "POST",
        "/admin/notices",
        &h,
        Some(json!({"title":"info"})),
    )
    .await;
    let shown = status_notices(&pool).await;
    assert_eq!(shown.len(), 2);
    assert_eq!(shown[0]["id"], id);
    assert_eq!(shown[0]["locale"], "en");
    assert!(shown[1]["locale"].is_null());

    // 未來開始 / 已停用都不顯示
    let future = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let (st, _, _) = call(
        &pool,
        "POST",
        "/admin/notices",
        &h,
        Some(json!({"title":"later","starts_at":future})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(status_notices(&pool).await.len(), 2);
    let path = format!("/admin/notices/{id}/disable");
    for _ in 0..2 {
        let (st, _, d) = call(&pool, "POST", &path, &h, None).await;
        assert_eq!(st, StatusCode::OK);
        assert!(d["disabled_at"].is_string());
    }
    assert_eq!(status_notices(&pool).await.len(), 1);
    // 已過期不顯示
    sqlx::query("UPDATE system_notices SET starts_at = now() - interval '2 hours', ends_at = now() - interval '1 hour' WHERE title = 'info'")
        .execute(&pool).await.unwrap();
    assert!(status_notices(&pool).await.is_empty());

    let (st, _, l) = call(&pool, "GET", "/admin/notices", &h, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(l["data"].as_array().unwrap().len(), 3);
    let (st, _, _) = call(
        &pool,
        "POST",
        "/admin/notices/00000000-0000-0000-0000-000000000000/disable",
        &h,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE action IN ('system_notice.create','system_notice.disable') AND entity='system_notices'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(n, 5);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn validation_and_roles(pool: PgPool) {
    let a = staff(&pool, "admin").await;
    let sup = staff(&pool, "sup").await;
    sqlx::query("UPDATE users SET staff_roles = '{support}' WHERE id = $1")
        .bind(sup.id)
        .execute(&pool)
        .await
        .unwrap();
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    for body in [
        json!({"title":"  "}),
        json!({"title":"x","level":"bad"}),
        json!({"title":"x","locale":"fr"}),
        json!({"title":"x","body":"a".repeat(501)}),
        json!({"title":"x","ends_at":past}),
        json!({"title":"x","starts_at":(chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339(),"ends_at":(chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339()}),
    ] {
        let (st, _, r) = call(
            &pool,
            "POST",
            "/admin/notices",
            &bearer(&a.token),
            Some(body.clone()),
        )
        .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{body} {r}");
    }
    let (st, _, _) = call(
        &pool,
        "POST",
        "/admin/notices",
        &bearer(&sup.token),
        Some(json!({"title":"x"})),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = call(&pool, "GET", "/admin/notices", &bearer(&sup.token), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn create_works_in_read_only_mode(pool: PgPool) {
    let a = staff(&pool, "admin").await;
    sqlx::query("UPDATE system_flags SET value = 'true'::jsonb WHERE key = 'read_only'")
        .execute(&pool)
        .await
        .unwrap();
    let (st, _, b) = call(
        &pool,
        "POST",
        "/admin/notices",
        &bearer(&a.token),
        Some(json!({"title":"唯讀中"})),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "{b}");
    let (_, _, s) = call(&pool, "GET", "/public/status", &[], None).await;
    assert_eq!(s["read_only"], true);
    assert_eq!(s["notices"].as_array().unwrap().len(), 1);
}
