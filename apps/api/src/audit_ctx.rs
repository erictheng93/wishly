//! 請求脈絡（request id / IP / User-Agent），供稽核寫入共用 helper 帶入 audit_logs。
//! 背景工作不在請求內，一律回傳全 None。
#[derive(Clone, Default)]
pub struct Ctx {
    pub request_id: Option<uuid::Uuid>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

tokio::task_local! {
    pub static CTX: Ctx;
}

pub fn current() -> Ctx {
    CTX.try_with(Clone::clone).unwrap_or_default()
}

/// 每個請求：產生（或接受合法 UUID 的）x-request-id、寫回應標頭與 tracing，並設定脈絡。
pub async fn middleware(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let (parts, body) = req.into_parts();
    let request_id = parts
        .headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| uuid::Uuid::parse_str(v.trim()).ok())
        .unwrap_or_else(uuid::Uuid::new_v4);
    let ctx = Ctx {
        request_id: Some(request_id),
        ip: crate::security::inet(&crate::config::client_ip(&parts, &crate::config::get())),
        user_agent: parts
            .headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.chars().take(300).collect()),
    };
    tracing::Span::current().record("request_id", tracing::field::display(request_id));
    let req = axum::extract::Request::from_parts(parts, body);
    let mut res = CTX.scope(ctx, next.run(req)).await;
    if let Ok(v) = request_id.to_string().parse() {
        res.headers_mut().insert("x-request-id", v);
    }
    res
}

/// 稽核單筆寫入的共用入口：自動帶入請求脈絡（背景工作為 NULL）。
#[allow(clippy::too_many_arguments)]
pub async fn insert<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    actor_type: &str,
    actor_id: Option<uuid::Uuid>,
    action: &str,
    entity: &str,
    entity_id: Option<uuid::Uuid>,
    diff: Option<serde_json::Value>,
) -> Result<(), sqlx::Error> {
    let c = current();
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff, ip, request_id, user_agent) VALUES ($1::actor_type, $2, $3, $4, $5, $6, $7::inet, $8, $9)")
        .bind(actor_type).bind(actor_id).bind(action).bind(entity).bind(entity_id).bind(diff)
        .bind(c.ip).bind(c.request_id).bind(c.user_agent).execute(ex).await?;
    Ok(())
}
