//! 帳號刪除 / 去識別化（docs/04 4.9，D40）。P2 錢包前置檢查不在 MVP。
use crate::{error::AppError, session::CurrentUser, AppState};
use axum::{
    extract::{Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/me", delete(delete_me))
        .route("/me/export", get(export))
        .route("/unsubscribe", get(unsubscribe_redirect).post(unsubscribe))
}

// ---------- 一鍵退訂（4.13）：token = base64url("u|g:<id>:<exp>") . hex(HMAC-SHA256) ----------
fn mac(msg: &str) -> Hmac<Sha256> {
    let mut m = <Hmac<Sha256> as Mac>::new_from_slice(crate::config::get().unsub_secret.as_bytes())
        .unwrap();
    m.update(msg.as_bytes());
    m
}

/// kind: 'u'（建立者）| 'g'（訪客）；有效一年
pub fn unsub_token(kind: char, id: Uuid) -> String {
    let msg = format!(
        "{kind}:{id}:{}",
        chrono::Utc::now().timestamp() + 365 * 86400
    );
    format!(
        "{}.{}",
        B64.encode(&msg),
        hex::encode(mac(&msg).finalize().into_bytes())
    )
}

fn verify(token: &str) -> Option<(char, Uuid)> {
    let (m, sig) = token.split_once('.')?;
    let msg = String::from_utf8(B64.decode(m).ok()?).ok()?;
    mac(&msg).verify_slice(&hex::decode(sig).ok()?).ok()?;
    let mut it = msg.split(':');
    let (k, id, exp) = (
        it.next()?.chars().next()?,
        it.next()?.parse().ok()?,
        it.next()?.parse::<i64>().ok()?,
    );
    (exp > chrono::Utc::now().timestamp() && matches!(k, 'u' | 'g')).then_some((k, id))
}

#[derive(Deserialize)]
struct UnsubQ {
    token: Option<String>,
}

/// GET 不變更狀態（信件掃描器預取無害）：只 302 到前端確認頁，由頁面 POST 才退訂。token 驗證留給 POST。
async fn unsubscribe_redirect(Query(q): Query<UnsubQ>) -> Response {
    // token 只含 base64url / '.' / hex；含其他字元者視為無效，不帶入 URL（避免注入）
    let t = q
        .token
        .filter(|t| {
            t.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        })
        .unwrap_or_default();
    let loc = format!("{}/unsubscribe?token={t}", crate::config::get().app_url);
    (StatusCode::FOUND, [(header::LOCATION, loc)]).into_response()
}

#[derive(Deserialize)]
struct UnsubBody {
    token: Option<String>,
}

/// POST {token}：HMAC 驗證（verify_slice 常數時間）；成功 204、冪等；無效 / 過期 422 INVALID_TOKEN
async fn unsubscribe(
    State(st): State<AppState>,
    Json(b): Json<UnsubBody>,
) -> Result<StatusCode, AppError> {
    let Some((k, id)) = b.token.as_deref().and_then(verify) else {
        return Err(AppError::problem(
            422,
            "INVALID_TOKEN",
            "退訂連結無效或已過期",
        ));
    };
    let mut tx = st.pool.begin().await?;
    if k == 'u' {
        sqlx::query("UPDATE users SET notification_prefs = jsonb_set(COALESCE(notification_prefs, '{}'::jsonb), '{email_claims}', 'false'::jsonb) WHERE id = $1 AND deleted_at IS NULL").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending' AND kind IN ('claim.created', 'claim.digest')").bind(id).execute(&mut *tx).await?;
    } else {
        sqlx::query("UPDATE guests SET email = NULL WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE notifications SET status = 'cancelled' WHERE guest_id = $1 AND status = 'pending'").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------- GET /me/export ----------
async fn export(State(st): State<AppState>, u: CurrentUser) -> Result<Response, AppError> {
    // 每帳號每日 3 次（以 audit_logs 計）
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_logs WHERE actor_type = 'user' AND actor_id = $1 AND action = 'account.export' AND created_at > now() - interval '1 day'")
        .bind(u.id).fetch_one(&st.pool).await?;
    if n >= 3 {
        return Err(AppError::Problem {
            status: 429,
            code: "RATE_LIMITED",
            detail: "匯出每日最多 3 次".into(),
            errors: None,
            retry_after: Some(3600),
        });
    }
    let user: Value = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'display_name', display_name, 'email', email, 'bio', bio, 'birthday', birthday, 'created_at', created_at) FROM users WHERE id = $1")
        .bind(u.id).fetch_one(&st.pool).await?;
    let identities: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('provider', provider::text) FROM auth_identities WHERE user_id = $1 ORDER BY created_at")
        .bind(u.id).fetch_all(&st.pool).await?;
    // 驚喜鎖定期間不含品項捐助數；只匯出本人的捐助（不含他人資料）
    let wishlists: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', w.id, 'slug', w.slug::text, 'title', w.title, 'type', w.type::text, 'status', w.status::text,
           'event_date', w.event_date, 'surprise_mode', w.surprise_mode, 'created_at', w.created_at,
           'items', (SELECT COALESCE(jsonb_agg(jsonb_build_object('id', i.id, 'title', i.title, 'qty_needed', i.qty_needed,
               'qty_claimed', CASE WHEN surprise_locked(w.surprise_mode, w.event_date) THEN NULL ELSE i.qty_claimed END)
               ORDER BY i.sort_order, i.id), '[]'::jsonb) FROM wishlist_items i WHERE i.wishlist_id = w.id AND i.deleted_at IS NULL))
         FROM wishlists w WHERE w.owner_id = $1 AND w.org_id IS NULL AND w.deleted_at IS NULL ORDER BY w.id").bind(u.id).fetch_all(&st.pool).await?;
    let claims: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'item_id', item_id, 'qty', qty, 'status', status::text, 'note', note, 'is_anonymous', is_anonymous, 'created_at', created_at)
           FROM claims WHERE user_id = $1 ORDER BY id").bind(u.id).fetch_all(&st.pool).await?;
    // P2-A：錢包、點數流水與本人的認捐（不含他人資料）
    let wallet: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'balance', balance, 'status', status::text, 'created_at', created_at) FROM point_wallets WHERE user_id = $1")
        .bind(u.id).fetch_optional(&st.pool).await?;
    let ledger: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', l.id, 'delta', l.delta, 'balance_after', l.balance_after, 'entry_type', l.entry_type::text, 'ref_type', l.ref_type,
                'ref_id', l.ref_id, 'note', l.note, 'created_at', l.created_at)
           FROM point_ledger l JOIN point_wallets w ON w.id = l.wallet_id WHERE w.user_id = $1 ORDER BY l.seq").bind(u.id).fetch_all(&st.pool).await?;
    let contributions: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'item_id', item_id, 'wishlist_id', wishlist_id, 'points', points, 'refunded_points', refunded_points, 'status', status::text,
                'message', message, 'is_anonymous', is_anonymous, 'visibility', visibility::text, 'captured_at', captured_at, 'released_at', released_at, 'created_at', created_at)
           FROM contributions WHERE user_id = $1 ORDER BY id").bind(u.id).fetch_all(&st.pool).await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id) VALUES ('user', $1, 'account.export', 'users', $1)").bind(u.id).execute(&st.pool).await?;
    let now = chrono::Utc::now();
    let mut res = Json(json!({ "exported_at": now, "user": user, "identities": identities, "wishlists": wishlists, "claims": claims,
        "wallet": wallet, "ledger": ledger, "contributions": contributions })).into_response();
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"wishly-export-{}.json\"",
            now.format("%Y-%m-%d")
        ))
        .unwrap(),
    );
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(res)
}

/// Q10：刪帳前重新驗證，防被盜的 session 直接刪帳。有密碼者驗密碼（每人 15 分鐘 5 次）；
/// 只有 Google / LINE 者要求 session 在 10 分鐘內建立。在開交易前做：argon2 慢，不可持有列鎖。
async fn reauth(pool: &sqlx::PgPool, u: &CurrentUser, pw: Option<&str>) -> Result<(), AppError> {
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT password_hash FROM auth_identities WHERE user_id=$1 AND provider='email' AND password_hash IS NOT NULL",
    )
    .bind(u.id)
    .fetch_optional(pool)
    .await?;
    let Some(hash) = hash else {
        let fresh: bool = sqlx::query_scalar(
            "SELECT created_at > now() - interval '10 minutes' FROM sessions WHERE id=$1",
        )
        .bind(u.session_id)
        .fetch_one(pool)
        .await?;
        return if fresh {
            Ok(())
        } else {
            Err(AppError::problem(
                403,
                "REAUTH_REQUIRED",
                "為了安全，請重新登入後再刪除帳號",
            ))
        };
    };
    let pw = pw
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::invalid("/password", "REQUIRED", "請輸入密碼"))?
        .to_owned();
    // 驗證前就計數：超限時連正確密碼也不驗，暴力猜測拿不到判定
    crate::ratelimit::check(pool, &format!("delete-me-pw:{}", u.id), 5, 900).await?;
    let ok = crate::auth_ext::argon_run(move || {
        pw.chars().count() <= 128 && crate::auth_ext::verify_pw(&pw, &hash)
    })
    .await?;
    if ok {
        Ok(())
    } else {
        Err(AppError::problem(401, "INVALID_CREDENTIALS", "密碼不正確"))
    }
}

async fn delete_me(
    State(st): State<AppState>,
    u: CurrentUser,
    Json(b): Json<Value>,
) -> Result<Response, AppError> {
    if b.get("confirm").and_then(Value::as_str) != Some("DELETE") {
        return Err(AppError::invalid(
            "/confirm",
            "INVALID",
            "confirm 必須為 \"DELETE\"",
        ));
    }
    reauth(&st.pool, &u, b.get("password").and_then(Value::as_str)).await?;
    let id = u.id;
    let mut tx = crate::uploads::media_tx(&st.pool, id).await?;
    crate::admin::lock_staff_changes(&mut tx).await?;
    let staff: bool = sqlx::query_scalar("SELECT is_staff FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if staff {
        crate::admin::protect_last_staff(&mut tx, id).await?;
    }
    // P2-A：還有進行中的認捐（pledged）或點數餘額就不能刪帳號（否則點數會憑空消失）。先鎖錢包，與認捐 / 營運發點互斥。
    // captured（已達標、採購中）的認捐不擋：點數已花出，帳號匿名化後 donor_name 一併去識別。
    // 鎖序同 wallet_ops::lock_topup（topups → 錢包）：已鎖住儲值、正等錢包的爭議 / 拒付先完成，下面才看得到凍結或欠款。
    sqlx::query("SELECT t.id FROM topups t JOIN point_wallets w ON w.id = t.wallet_id WHERE w.user_id = $1 ORDER BY t.id FOR UPDATE OF t")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT id FROM point_wallets WHERE user_id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let held: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM contributions WHERE user_id = $1 AND status = 'pledged'), (SELECT coalesce(sum(balance), 0)::bigint FROM point_wallets WHERE user_id = $1)")
        .bind(id).fetch_one(&mut *tx).await?;
    if held.0 > 0 || held.1 > 0 {
        return Err(AppError::problem(409, "ACCOUNT_HAS_POINTS", format!("帳號還有 {} 筆進行中的贊助、{} 點餘額。請先到錢包撤回贊助，並聯絡客服處理剩餘點數後再刪除帳號。", held.0, held.1)));
    }
    // F17 / F18：未結案的現金退款（被拒會退點回錢包）、拒付欠款、爭議中凍結的錢包，都要先由客服結案，
    // 否則點數或欠款會落在已匿名、無法登入的帳號上。
    let (refunds, debts, frozen): (i64, i64, bool) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM cash_refund_requests r JOIN point_wallets w ON w.id = r.wallet_id WHERE w.user_id = $1 AND r.status IN ('requested', 'processing')),
                (SELECT count(*) FROM wallet_debts d JOIN point_wallets w ON w.id = d.wallet_id WHERE w.user_id = $1 AND d.status = 'open'),
                EXISTS (SELECT 1 FROM point_wallets WHERE user_id = $1 AND status = 'frozen')")
        .bind(id).fetch_one(&mut *tx).await?;
    if refunds > 0 || debts > 0 || frozen {
        return Err(AppError::problem(409, "ACCOUNT_HAS_POINTS", format!("帳號還有 {refunds} 筆處理中的現金退款、{debts} 筆待處理欠款{}。請聯絡客服結案後再刪除帳號。", if frozen { "，且錢包已凍結" } else { "" })));
    }
    // 尚未付款的儲值（ATM / 超商代碼可能隔天才付）入帳後會落在已匿名的帳號上：等付款或到期後再刪
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM topups t JOIN point_wallets w ON w.id = t.wallet_id WHERE w.user_id = $1 AND t.status = 'pending' AND t.expires_at > now()")
        .bind(id).fetch_one(&mut *tx).await?;
    if pending > 0 {
        return Err(AppError::problem(
            409,
            "ACCOUNT_HAS_POINTS",
            format!("帳號還有 {pending} 筆尚未付款的儲值，請完成付款或等待到期後再刪除帳號。"),
        ));
    }
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM user_activity_days WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM otp_challenges WHERE link_user_id=$1 OR email = (SELECT email FROM users WHERE id = $1)").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM auth_identities WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    // 組織據點不屬於個人：轉給其他 owner / coordinator（owner 優先、最早加入）；本人的成員資格停用；組織已無人可管理就關閉全部據點（保持公開）。
    // 鎖序 organizations(FOR UPDATE，依 id) → wishlists：與撤銷驗證、據點上線一致，必須先於任何 wishlists 更新。
    // 鎖本人所屬的每個組織（不只擁有據點的）：同組織的管理者同時刪帳號會被序列化，後者看得到前者已停用的成員資格。
    let site_orgs: Vec<Uuid> = sqlx::query_scalar(
        "SELECT org_id FROM org_members WHERE user_id = $1 AND disabled_at IS NULL
         UNION SELECT org_id FROM wishlists WHERE owner_id = $1 AND org_id IS NOT NULL AND deleted_at IS NULL")
        .bind(id).fetch_all(&mut *tx).await?;
    sqlx::query("SELECT id FROM organizations WHERE id = ANY($1) ORDER BY id FOR UPDATE").bind(&site_orgs).execute(&mut *tx).await?;
    let mut site_slugs: Vec<String> = sqlx::query_scalar(
        "WITH s AS (SELECT id, org_id FROM wishlists WHERE owner_id = $1 AND org_id IS NOT NULL AND deleted_at IS NULL ORDER BY id FOR UPDATE)
         UPDATE wishlists w SET owner_id = r.uid FROM s
           JOIN LATERAL (SELECT m.user_id AS uid FROM org_members m JOIN users mu ON mu.id = m.user_id AND mu.deleted_at IS NULL AND mu.blocked_at IS NULL
                          WHERE m.org_id = s.org_id AND m.disabled_at IS NULL AND m.role IN ('owner', 'coordinator') AND m.user_id <> $1
                          ORDER BY (m.role = 'owner') DESC, m.created_at, m.user_id LIMIT 1) r ON true
          WHERE w.id = s.id RETURNING w.slug::text")
        .bind(id).fetch_all(&mut *tx).await?;
    sqlx::query("UPDATE org_members SET disabled_at = now() WHERE user_id = $1 AND disabled_at IS NULL").bind(id).execute(&mut *tx).await?;
    // 最後一位 owner 離開時，最早加入的有效 coordinator 升為 owner（組織不會變成無 owner；已持有 org 列鎖）
    sqlx::query(
        "UPDATE org_members SET role = 'owner' WHERE (org_id, user_id) IN (
           SELECT DISTINCT ON (m.org_id) m.org_id, m.user_id FROM org_members m
             JOIN users mu ON mu.id = m.user_id AND mu.deleted_at IS NULL AND mu.blocked_at IS NULL
            WHERE m.org_id = ANY($1) AND m.disabled_at IS NULL AND m.role = 'coordinator'
              AND NOT EXISTS (SELECT 1 FROM org_members o JOIN users ou ON ou.id = o.user_id AND ou.deleted_at IS NULL AND ou.blocked_at IS NULL
                               WHERE o.org_id = m.org_id AND o.disabled_at IS NULL AND o.role = 'owner')
            ORDER BY m.org_id, m.created_at, m.user_id)")
        .bind(&site_orgs).execute(&mut *tx).await?;
    let closed: Vec<(Uuid, String, bool)> = sqlx::query_as(
        "WITH o AS (SELECT w.id, w.status FROM wishlists w
                     WHERE w.org_id = ANY($1) AND w.deleted_at IS NULL AND w.status IN ('draft', 'active')
                       AND NOT EXISTS (SELECT 1 FROM org_members m JOIN users mu ON mu.id = m.user_id AND mu.deleted_at IS NULL AND mu.blocked_at IS NULL
                                        WHERE m.org_id = w.org_id AND m.disabled_at IS NULL AND m.role IN ('owner', 'coordinator'))
                     ORDER BY w.id FOR UPDATE OF w)
         UPDATE wishlists SET status = 'closed', site_status = 'closed', closed_at = COALESCE(wishlists.closed_at, now())
           FROM o WHERE wishlists.id = o.id RETURNING wishlists.id, wishlists.slug::text, o.status = 'active'")
        .bind(&site_orgs).fetch_all(&mut *tx).await?;
    for (w, slug, _) in &closed {
        crate::dashboard::notify(&mut *tx, *w).await?;
        site_slugs.push(slug.clone());
    }
    // Retain replaced/issued image keys; the worker rechecks sharing under the media lock.
    let media: Vec<String> = sqlx::query_scalar(
        "SELECT object_key FROM user_media WHERE user_id=$1
         UNION SELECT cover_image_key FROM wishlists WHERE owner_id=$1 AND org_id IS NULL AND cover_image_key IS NOT NULL
         UNION SELECT i.image_key FROM wishlist_items i JOIN wishlists w ON w.id=i.wishlist_id WHERE w.owner_id=$1 AND w.org_id IS NULL AND i.image_key IS NOT NULL
         UNION SELECT avatar_key FROM users WHERE id=$1 AND avatar_key IS NOT NULL")
        .bind(id).fetch_all(&mut *tx).await?;
    let media: Vec<String> = media
        .into_iter()
        .filter(|k| {
            (crate::uploads::valid_key(k) && !k.starts_with("catalog/"))
                || crate::uploads::avatar_url(Some(k)).is_some()
        })
        .collect();
    for key in &media {
        sqlx::query(
            "INSERT INTO user_media (user_id, object_key) VALUES ($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(key)
        .execute(&mut *tx)
        .await?;
    }
    // 刪除要求與刪帳號同一交易提交：之後就算程序重啟，worker 也會接著刪完
    crate::uploads::request_retire(&mut tx, &media).await?;
    let rows: Vec<(Uuid, String, bool)> = sqlx::query_as(
        "WITH o AS (SELECT id, status FROM wishlists WHERE owner_id = $1 AND org_id IS NULL AND status <> 'archived' FOR UPDATE)
         UPDATE wishlists SET status = 'archived', visibility = 'private', closed_at = COALESCE(wishlists.closed_at, now())
           FROM o WHERE wishlists.id = o.id RETURNING wishlists.id, wishlists.slug::text, o.status <> 'closed'")
        .bind(id).fetch_all(&mut *tx).await?;
    let mut graced: Vec<Uuid> = rows.iter().filter(|r| r.2).map(|r| r.0).collect();
    graced.extend(closed.iter().filter(|c| c.2).map(|c| c.0)); // 這次才結束的清單，他人捐助要給結束寬限
    let (archived, mut slugs): (Vec<Uuid>, Vec<String>) = rows.into_iter().map(|r| (r.0, r.1)).unzip();
    slugs.extend(site_slugs); // 據點換了擁有者 / 已關閉：公開快取副本一併清掉
    crate::wishlists::release_pledged(&mut tx, &archived).await?; // 別人捐在這些清單上、尚未達標的點數退回各自錢包
                                                                  // F-08：取消他人清單上的 reserved 捐助並回補（purchased / delivered 已履行，保留）。鎖序同 claims::apply：先 item（依 id）後 claim。
    sqlx::query("SELECT id FROM wishlist_items WHERE id IN (SELECT item_id FROM claims WHERE user_id = $1 AND status = 'reserved') ORDER BY id FOR UPDATE")
        .bind(id).execute(&mut *tx).await?;
    let wids: Vec<Uuid> = sqlx::query_scalar(
        "WITH x AS (UPDATE claims SET status = 'cancelled', cancelled_at = now(), expires_at = NULL
                     WHERE user_id = $1 AND status = 'reserved' RETURNING id, item_id, qty),
         r AS (UPDATE wishlist_items i SET qty_claimed = i.qty_claimed - s.q
                FROM (SELECT item_id, sum(qty)::int AS q FROM x GROUP BY item_id) s
                WHERE i.id = s.item_id RETURNING i.wishlist_id),
         a AS (INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id, diff)
                SELECT 'user'::actor_type, $1, 'claim.account_delete_cancel', 'claims', id, jsonb_build_object('qty', qty) FROM x)
         SELECT DISTINCT wishlist_id FROM r").bind(id).fetch_all(&mut *tx).await?;
    for w in wids {
        crate::dashboard::notify(&mut *tx, w).await?;
    }
    for w in &graced {
        crate::claims::grace_on_close(&mut tx, *w).await?; // 在所有 item 鎖之後（鎖序見 grace_on_close）
    }
    sqlx::query("UPDATE claims SET claimer_name = '已刪除的使用者' WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE contributions SET donor_name = '已刪除的使用者' WHERE user_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE notifications SET status = 'cancelled' WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE notifications SET payload=payload - 'to' WHERE user_id=$1 AND kind IN ('auth.identity_linked','auth.identity_unlinked')")
        .bind(id).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE wishlists SET cover_image_key=NULL, cover_image_status='none' WHERE owner_id=$1 AND org_id IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE wishlist_items SET image_key=NULL, image_status='none' WHERE wishlist_id IN (SELECT id FROM wishlists WHERE owner_id=$1 AND org_id IS NULL)")
        .bind(id).execute(&mut *tx).await?;
    let (at,): (chrono::DateTime<chrono::Utc>,) = sqlx::query_as(
        "UPDATE users SET display_name = '已刪除的使用者', email = NULL, avatar_key = NULL, bio = NULL, birthday = NULL, notification_prefs = '{}'::jsonb,
                deleted_at = now(), anonymized_at = now() WHERE id = $1 AND deleted_at IS NULL RETURNING anonymized_at")
        .bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO audit_logs (actor_type, actor_id, action, entity, entity_id) VALUES ('user', $1, 'account.delete', 'users', $1)")
        .bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    crate::uploads::delete_objects(st.pool.clone(), media); // COMMIT 後才刪：交易失敗不會留下指向已刪物件的資料
    crate::cdn::purge_wishlists(slugs); // 已封存為 private：邊緣快取的公開副本立即清掉
    let mut res = Json(json!({ "deleted": true, "anonymized_at": at })).into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("ws_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax"),
    );
    Ok(res)
}
