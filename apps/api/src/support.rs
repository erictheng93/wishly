//! 支持品項：指向另一份公開清單，不計入完成度（設計見 docs/superpowers/specs/2026-11-01-support-item-design.md）。
use crate::error::AppError;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;

/// 清單可公開提供（別名 `w` 清單、`o` 擁有者）；與公開頁同一套規則。
pub const TARGET_LIVE_SQL: &str = concat!(
    "w.deleted_at IS NULL AND w.visibility = 'public' AND w.status = 'active' AND w.moderation_status = 'ok' \
     AND (w.closes_at IS NULL OR w.closes_at > now()) AND o.deleted_at IS NULL AND o.blocked_at IS NULL \
     AND (w.type <> 'relief' OR (",
    crate::public::live_site!(),
    "))"
);

fn err(code: &'static str, detail: &'static str) -> AppError {
    AppError::Problem {
        status: 422,
        code,
        detail: detail.into(),
        errors: Some(json!([{ "pointer": "/support_slug", "code": code, "detail": detail }])),
        retry_after: None,
    }
}

/// 以 slug 找支持對象；不存在、非公開、已下架一律同一個錯誤，避免探測他人清單。
pub async fn resolve_target(tx: &mut Tx<'_>, slug: &str, uid: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Uuid, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT w.id, can_manage_wishlist(w.owner_id, w.org_id, $2) FROM wishlists w
         JOIN users o ON o.id = w.owner_id WHERE w.slug = $1 AND {TARGET_LIVE_SQL}"
    )))
    .bind(slug)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        None => Err(err("SUPPORT_TARGET_INVALID", "找不到可支持的清單。")),
        Some((_, true)) => Err(err("SUPPORT_TARGET_SELF", "不能支持自己管理的清單。")),
        Some((id, false)) => Ok(id),
    }
}
