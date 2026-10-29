//! 新手導覽（D66）：個人資料 → 建立清單 → 加品項 → 發佈（受捐人）→ 捐助（捐助人）。
//! 步驟完成與否一律由既有資料推導，換裝置也一致；只存「完成 / 跳過」的時間（users.onboarding_done_at）。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{extract::State, routing::get, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/me/onboarding", get(status).patch(patch))
}

/// 步驟順序即導覽順序；前端以第一個未完成的步驟為「目前步驟」
const STEPS: [&str; 5] = ["profile", "wishlist", "item", "publish", "give"];

async fn status(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(load(&st.pool, u.id).await?))
}

async fn load(pool: &sqlx::PgPool, uid: Uuid) -> Result<Value, AppError> {
    // 個人清單才算（組織據點不是「我的心願」）；已刪除的不算，已封存仍算走過
    let (done, profile, wishlist, item, publish, give, list_id): (bool, bool, bool, bool, bool, bool, Option<Uuid>) = sqlx::query_as(
        "SELECT u.onboarding_done_at IS NOT NULL,
                u.handle IS NOT NULL OR u.bio IS NOT NULL OR u.birthday IS NOT NULL,
                EXISTS (SELECT 1 FROM wishlists w WHERE w.owner_id = u.id AND w.org_id IS NULL AND w.deleted_at IS NULL),
                EXISTS (SELECT 1 FROM wishlist_items i JOIN wishlists w ON w.id = i.wishlist_id
                         WHERE w.owner_id = u.id AND w.org_id IS NULL AND w.deleted_at IS NULL AND i.deleted_at IS NULL),
                EXISTS (SELECT 1 FROM wishlists w WHERE w.owner_id = u.id AND w.org_id IS NULL AND w.deleted_at IS NULL AND w.status <> 'draft'),
                EXISTS (SELECT 1 FROM claims c WHERE c.user_id = u.id) OR EXISTS (SELECT 1 FROM contributions c WHERE c.user_id = u.id),
                (SELECT w.id FROM wishlists w WHERE w.owner_id = u.id AND w.org_id IS NULL AND w.deleted_at IS NULL AND w.status <> 'archived'
                  ORDER BY w.created_at DESC LIMIT 1)
           FROM users u WHERE u.id = $1 AND u.deleted_at IS NULL",
    )
    .bind(uid)
    .fetch_optional(pool)
    .await?
    .ok_or(AppError::Unauthorized)?;
    let flags = [profile, wishlist, item, publish, give];
    Ok(json!({
        "done": done,
        "steps": STEPS.iter().zip(flags).map(|(k, v)| json!({ "key": k, "done": v })).collect::<Vec<_>>(),
        "current": STEPS.iter().zip(flags).find(|(_, v)| !v).map(|(k, _)| *k),
        // 「加品項 / 發佈」要帶去的清單：最近建立、未封存的一份
        "wishlist_id": list_id,
    }))
}

#[derive(Deserialize)]
struct Patch {
    /// true = 完成或跳過；false = 重新開始（設定頁）
    done: bool,
}

async fn patch(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(p): Json<Patch>,
) -> Result<Json<Value>, AppError> {
    // 重複完成不覆寫第一次的時間
    sqlx::query(
        "UPDATE users SET onboarding_done_at = CASE WHEN $2 THEN COALESCE(onboarding_done_at, now()) END WHERE id = $1",
    )
    .bind(u.id)
    .bind(p.done)
    .execute(&st.pool)
    .await?;
    Ok(Json(load(&st.pool, u.id).await?))
}
