//! 個人頁外部連結：本人整組覆寫（PUT /me/links）與公開連結頁（GET /public/links/{handle}，可被 CDN 快取、不可讀 cookie）。
use crate::{error::AppError, session::CurrentUser, validate, AppState};
use axum::{
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

const MAX_LINKS: usize = 10;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/links", get(list).put(replace))
        .route("/public/links/{handle}", get(public))
}

/// 只收 https、有 host、無帳密、≤500 字（防 javascript: 與釣魚用的 userinfo）。
pub(crate) fn valid_url(s: &str) -> bool {
    s.chars().count() <= 500
        && reqwest::Url::parse(s).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str().is_some_and(|h| !h.is_empty())
                && u.username().is_empty()
                && u.password().is_none()
        })
}

/// 個人頁用：非 hidden 的連結。
pub(crate) async fn visible(pool: &sqlx::PgPool, uid: uuid::Uuid) -> Result<Vec<Value>, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT title, url FROM user_links WHERE user_id = $1 AND NOT hidden ORDER BY position",
    )
    .bind(uid)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(t, u)| json!({ "title": t, "url": u }))
        .collect())
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> Result<Json<Value>, AppError> {
    let rows: Vec<(uuid::Uuid, String, String, bool)> = sqlx::query_as(
        "SELECT id, title, url, hidden FROM user_links WHERE user_id = $1 ORDER BY position",
    )
    .bind(u.id)
    .fetch_all(&st.pool)
    .await?;
    let links: Vec<Value> = rows
        .into_iter()
        .map(|(id, title, url, hidden)| json!({ "id": id, "title": title, "url": url, "hidden": hidden }))
        .collect();
    Ok(Json(json!({ "links": links })))
}

#[derive(Deserialize)]
struct Link {
    title: String,
    url: String,
    #[serde(default)]
    hidden: bool,
}
#[derive(Deserialize)]
struct Body {
    links: Vec<Link>,
}

async fn replace(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(b): Json<Body>,
) -> Result<Json<Value>, AppError> {
    if b.links.len() > MAX_LINKS {
        return Err(AppError::invalid("/links", "TOO_MANY", "最多 10 筆連結"));
    }
    let mut clean = Vec::with_capacity(b.links.len());
    for (i, l) in b.links.iter().enumerate() {
        let title = validate::text(&l.title, &format!("/links/{i}/title"), false)?;
        if title.is_empty() || title.chars().count() > 40 {
            return Err(AppError::invalid(
                &format!("/links/{i}/title"),
                "INVALID",
                "標題需為 1–40 字",
            ));
        }
        let url = l.url.trim();
        if !valid_url(url) {
            return Err(AppError::invalid(
                &format!("/links/{i}/url"),
                "INVALID_URL",
                "僅接受 https 網址（不含帳號密碼，最長 500 字）",
            ));
        }
        clean.push((title, url.to_string(), l.hidden));
    }
    let mut tx = st.pool.begin().await?;
    // 同一人的並行 PUT 依序執行，否則兩邊都看不到對方未提交的列而雙雙寫入（超過上限）
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('user_links:' || $1::text, 0))")
        .bind(u.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM user_links WHERE user_id = $1")
        .bind(u.id)
        .execute(&mut *tx)
        .await?;
    let titles: Vec<&str> = clean.iter().map(|c| c.0.as_str()).collect();
    let urls: Vec<&str> = clean.iter().map(|c| c.1.as_str()).collect();
    let hidden: Vec<bool> = clean.iter().map(|c| c.2).collect();
    sqlx::query("INSERT INTO user_links (user_id, title, url, position, hidden) SELECT $1, t, u, (ord - 1)::smallint, h FROM unnest($2::text[], $3::text[], $4::bool[]) WITH ORDINALITY AS x(t, u, h, ord)")
        .bind(u.id)
        .bind(&titles)
        .bind(&urls)
        .bind(&hidden)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    list(u, State(st)).await
}

async fn public(
    State(st): State<AppState>,
    Path(handle): Path<String>,
) -> Result<Response, AppError> {
    // 不呼叫 access::viewer：回應會被 CDN 快取，只能含所有人都看得到的內容
    #[allow(clippy::type_complexity)]
    let (uid, handle, name, bio, avatar_key, cover_preset, cover_key): (
        uuid::Uuid,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT id, handle, display_name, bio, avatar_key, cover_preset, cover_image_key FROM users
          WHERE handle = lower($1) AND deleted_at IS NULL AND blocked_at IS NULL",
    )
    .bind(handle)
    .fetch_optional(&st.pool)
    .await?
    .ok_or(AppError::NotFound)?;
    let links = visible(&st.pool, uid).await?;
    let body = json!({ "user": { "display_name": name, "handle": handle, "bio": bio,
                                 "avatar_url": crate::uploads::avatar_url(avatar_key.as_deref()),
                                 "cover_preset": cover_preset,
                                 "cover_image_url": cover_key.as_deref().map(|k| crate::uploads::S3::get().public_url(k)) },
                       "links": links });
    Ok(([(header::CACHE_CONTROL, "public, s-maxage=60")], Json(body)).into_response())
}

#[cfg(test)]
mod tests {
    use super::valid_url;
    #[test]
    fn rejects_unsafe_urls() {
        assert!(valid_url("https://example.com/a?b=1"));
        for s in [
            "http://a.com",
            "https://a:b@x.com",
            "https://u@x.com",
            "javascript:alert(1)",
            "https://",
            "",
        ] {
            assert!(!valid_url(s), "{s}");
        }
        assert!(!valid_url(&format!("https://a.com/{}", "x".repeat(500))));
    }
}
