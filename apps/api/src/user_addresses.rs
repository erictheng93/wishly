//! 帳號層級地址（0069）：預設收件地址（shipping）與居住地址（home）。
//! 欄位級加密（sealed）；回應一律遮罩，明文只會經個人頁的 home 可見性出現。
use crate::{
    error::AppError,
    session::CurrentUser,
    wishlists::{address_fields, mask_address, masked, obj},
    AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

type R<T> = Result<T, AppError>;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me/addresses", get(list))
        .route("/me/addresses/{kind}", put(put_kind).delete(remove))
}

fn open(b: &[u8]) -> R<String> {
    crate::sealed::open_str(b)
        .ok_or_else(|| AppError::problem(500, "INTERNAL_ERROR", "地址無法解密"))
}

async fn list(u: CurrentUser, State(st): State<AppState>) -> R<Json<Value>> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, Option<Vec<u8>>, Option<Vec<u8>>, Vec<u8>)> = sqlx::query_as(
        "SELECT kind, recipient_name_enc, phone_enc, address_enc FROM user_addresses WHERE user_id = $1",
    )
    .bind(u.id)
    .fetch_all(&st.pool)
    .await?;
    let (mut shipping, mut home) = (Value::Null, Value::Null);
    for (kind, n, p, a) in rows {
        let addr = open(&a)?;
        if kind == "home" {
            home = json!({ "address": mask_address(&addr) });
        } else {
            let m = masked(
                &open(&n.unwrap_or_default())?,
                &open(&p.unwrap_or_default())?,
                &addr,
            );
            shipping = json!({ "recipient_name": m["recipient_name"], "phone": m["phone"], "address": m["address"] });
        }
    }
    Ok(Json(json!({ "shipping": shipping, "home": home })))
}

async fn upsert(
    pool: &PgPool,
    uid: Uuid,
    kind: &str,
    name: Option<&str>,
    phone: Option<&str>,
    addr: &str,
) -> R<()> {
    let seal = |s: &str| crate::sealed::seal(s.as_bytes());
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO user_addresses (user_id, kind, recipient_name_enc, phone_enc, address_enc) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (user_id, kind) DO UPDATE SET recipient_name_enc = EXCLUDED.recipient_name_enc, phone_enc = EXCLUDED.phone_enc,
                   address_enc = EXCLUDED.address_enc, updated_at = now()")
        .bind(uid).bind(kind).bind(name.map(seal)).bind(phone.map(seal)).bind(seal(addr))
        .execute(&mut *tx).await?;
    // 地址內容不進稽核
    crate::orgs::audit_user(
        &mut *tx,
        uid,
        "profile.address_update",
        "users",
        Some(uid),
        json!({ "kind": kind, "changed": true }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn put_kind(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(kind): Path<String>,
    Json(b): Json<Value>,
) -> R<Json<Value>> {
    match kind.as_str() {
        "shipping" => put_shipping(u, st, b).await,
        "home" => put_home(u, st, b).await,
        _ => Err(AppError::NotFound),
    }
}

async fn put_shipping(u: CurrentUser, st: AppState, b: Value) -> R<Json<Value>> {
    let (name, phone, addr) = address_fields(obj(&b)?, true)?;
    upsert(&st.pool, u.id, "shipping", Some(&name), Some(&phone), &addr).await?;
    let m = masked(&name, &phone, &addr);
    Ok(Json(
        json!({ "recipient_name": m["recipient_name"], "phone": m["phone"], "address": m["address"] }),
    ))
}

async fn put_home(u: CurrentUser, st: AppState, b: Value) -> R<Json<Value>> {
    let (_, _, addr) = address_fields(obj(&b)?, false)?;
    upsert(&st.pool, u.id, "home", None, None, &addr).await?;
    Ok(Json(json!({ "address": mask_address(&addr) })))
}

async fn remove(
    u: CurrentUser,
    State(st): State<AppState>,
    Path(kind): Path<String>,
) -> R<StatusCode> {
    if !matches!(kind.as_str(), "shipping" | "home") {
        return Err(AppError::NotFound);
    }
    let mut tx = st.pool.begin().await?;
    let n = sqlx::query("DELETE FROM user_addresses WHERE user_id = $1 AND kind = $2")
        .bind(u.id)
        .bind(&kind)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if n > 0 {
        crate::orgs::audit_user(
            &mut *tx,
            u.id,
            "profile.address_delete",
            "users",
            Some(u.id),
            json!({ "kind": kind, "changed": true }),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
