//! audit_logs 只增不改、請求脈絡寫入與保留期清理。
mod common;
use axum::{extract::ConnectInfo, http::StatusCode};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

async fn seed(pool: &PgPool, action: &str, age_days: i32) -> Uuid {
    sqlx::query_scalar("INSERT INTO audit_logs (actor_type, action, entity, created_at) VALUES ('system', $1, 'x', now() - make_interval(days => $2)) RETURNING id")
        .bind(action).bind(age_days).fetch_one(pool).await.unwrap()
}

async fn exists(pool: &PgPool, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM audit_logs WHERE id=$1)")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn append_only_triggers(pool: PgPool) {
    let id = seed(&pool, "a", 0).await;
    assert!(sqlx::query("UPDATE audit_logs SET action='b' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query("DELETE FROM audit_logs WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .is_err());
    assert!(sqlx::query("TRUNCATE audit_logs")
        .execute(&pool)
        .await
        .is_err());
    // purge 開關只放行 DELETE，UPDATE 仍拒絕
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('wishly.audit_purge','on',true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(sqlx::query("UPDATE audit_logs SET action='b' WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .is_err());
    drop(tx);
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('wishly.audit_purge','on',true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("DELETE FROM audit_logs WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(!exists(&pool, id).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn retention_only_purges_old_content_edits(pool: PgPool) {
    let old_edit = seed(&pool, "item.update", 400).await;
    let old_login = seed(&pool, "auth.login", 400).await;
    let old_money = seed(&pool, "topup.paid", 400).await;
    let new_edit = seed(&pool, "item.update", 10).await;
    wishly_api::ratelimit::cleanup(&pool).await.unwrap();
    assert!(!exists(&pool, old_edit).await);
    assert!(exists(&pool, old_login).await);
    assert!(exists(&pool, old_money).await);
    assert!(exists(&pool, new_edit).await);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn staff_action_records_request_context(pool: PgPool) {
    let staff = common::staff(&pool, "staff").await;
    let target = common::user(&pool, "target").await;
    let rid = Uuid::new_v4();
    let mut req = common::request(
        "PATCH",
        &format!("/api/v1/admin/users/{}", target.id),
        &[
            ("authorization", format!("Bearer {}", staff.token)),
            ("x-request-id", rid.to_string()),
            ("user-agent", "audit-test/1.0".into()),
        ],
        Some(json!({"blocked": true, "reason": "abuse"})),
    );
    req.extensions_mut().insert(ConnectInfo(
        "203.0.113.9:1234".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let (s, h, _) = common::call_request(&common::test_app(&pool), req).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h["x-request-id"], rid.to_string());
    let (ip, rq, ua): (Option<String>, Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT host(ip), request_id, user_agent FROM audit_logs WHERE actor_type='staff' AND actor_id=$1",
    )
    .bind(staff.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ip.as_deref(), Some("203.0.113.9"));
    assert_eq!(rq, Some(rid));
    assert_eq!(ua.as_deref(), Some("audit-test/1.0"));

    // 非 UUID 的 x-request-id 不採用：換成新產生的 UUID
    let (_, h, _) = common::call(
        &pool,
        "GET",
        "/health",
        &[("x-request-id", "not-a-uuid".into())],
        None,
    )
    .await;
    assert!(Uuid::parse_str(h["x-request-id"].to_str().unwrap()).is_ok());
    assert_ne!(h["x-request-id"], "not-a-uuid");
}
