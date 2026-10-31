//! Email 通知 outbox（寫 notifications，docs/04 4.10），
//! 背景 worker `spawn_worker` 輪詢到期通知，經 SMTP（預設 Mailpit localhost:1025）寄出。
//! 不通知受捐者「有新捐助」（0026 移除 claim.created / claim.digest）：會轟炸收件匣，也破壞驚喜。
use crate::config::Config;
use lettre::{
    message::header::ContentType, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
};
use serde_json::Value;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// 眾籌（P2-A）：通知某品項的捐贈者（status in `statuses` 的認捐，每人一封）。
/// kind：crowdfund.funded / order.shipped / order.delivered / order.failed / funding.expired。
/// payload 帶 wishlist_id 與 item_id；捐贈者本來就知道自己捐了哪個品項，故可寫品項名稱。
pub async fn enqueue_donors<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    item_id: Uuid,
    kind: &str,
    statuses: &[&str],
) -> Result<(), sqlx::Error> {
    let statuses: Vec<String> = statuses.iter().map(|s| s.to_string()).collect();
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT DISTINCT c.user_id, 'email'::notification_channel, $2, jsonb_build_object('wishlist_id', c.wishlist_id, 'item_id', c.item_id)
           FROM contributions c JOIN users u ON u.id = c.user_id
          WHERE c.item_id = $1 AND c.status::text = ANY($3) AND u.deleted_at IS NULL")
        .bind(item_id).bind(kind).bind(&statuses).execute(ex).await?;
    Ok(())
}

/// 通知單一使用者（同交易內的純 INSERT）；kind / payload 見計畫「新增 kinds 與 payload」。
pub async fn enqueue<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    user_id: Uuid,
    kind: &str,
    payload: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO notifications (user_id, channel, kind, payload) VALUES ($1, 'email'::notification_channel, $2, $3)")
        .bind(user_id).bind(kind).bind(payload).execute(ex).await?;
    Ok(())
}

/// 會員認領確認信：延後 10 分鐘寄，讓連續認領能在 worker 合併成一封。
pub async fn enqueue_claim_confirmation<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    claim_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload, scheduled_at)
         SELECT c.user_id, 'email'::notification_channel, 'claim.confirmation',
                jsonb_build_object('wishlist_id', i.wishlist_id, 'item_id', c.item_id, 'claim_id', c.id, 'expires_at', c.expires_at),
                now() + interval '10 minutes'
           FROM claims c JOIN wishlist_items i ON i.id = c.item_id
          WHERE c.id = $1 AND c.user_id IS NOT NULL")
        .bind(claim_id).execute(ex).await?;
    Ok(())
}

/// 通知清單管理者：個人清單給 owner_id；組織清單給未停用的 owner / coordinator。payload 併入 wishlist_id。
pub async fn enqueue_owner<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    wishlist_id: Uuid,
    kind: &str,
    payload: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT r.uid, 'email'::notification_channel, $2, $3::jsonb || jsonb_build_object('wishlist_id', w.id)
           FROM wishlists w
           JOIN LATERAL (
             SELECT w.owner_id AS uid WHERE w.org_id IS NULL
             UNION
             SELECT m.user_id FROM org_members m JOIN users u ON u.id = m.user_id AND u.deleted_at IS NULL
              WHERE w.org_id IS NOT NULL AND m.org_id = w.org_id AND m.disabled_at IS NULL AND m.role IN ('owner', 'coordinator')
           ) r ON true
          WHERE w.id = $1")
        .bind(wishlist_id).bind(kind).bind(payload).execute(ex).await?;
    Ok(())
}

/// 通知組織的 owner（未停用、帳號未刪除）。
pub async fn enqueue_org_owners<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    org_id: Uuid,
    kind: &str,
    payload: Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT m.user_id, 'email'::notification_channel, $2, $3
           FROM org_members m JOIN users u ON u.id = m.user_id AND u.deleted_at IS NULL
          WHERE m.org_id = $1 AND m.disabled_at IS NULL AND m.role = 'owner'")
        .bind(org_id).bind(kind).bind(payload).execute(ex).await?;
    Ok(())
}

/// 據點暫停 / 關閉：通知進行中的捐助者（reserved / purchased 認領、pledged 認捐，僅會員），每人每據點一筆。
pub async fn enqueue_site_paused<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    wishlist_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT d.user_id, 'email'::notification_channel, 'site.paused', jsonb_build_object('wishlist_id', d.wishlist_id)
           FROM (SELECT DISTINCT i.wishlist_id, r.user_id
                   FROM wishlist_items i
                   JOIN (SELECT item_id, user_id FROM claims WHERE status IN ('reserved', 'purchased') AND user_id IS NOT NULL
                         UNION SELECT item_id, user_id FROM contributions WHERE status = 'pledged') r ON r.item_id = i.id
                   JOIN users u ON u.id = r.user_id AND u.deleted_at IS NULL
                  WHERE i.wishlist_id = ANY($1)) d")
        .bind(wishlist_ids).execute(ex).await?;
    Ok(())
}

/// Q13：建立者修改品項內容後，通知進行中的捐助者（reserved / purchased，含有 email 的舊訪客）與眾籌認捐者（pledged）。
/// 連續修改合併：延後 10 分鐘寄，notifications_item_changed_uk 讓同一人同一品項只留一筆待寄；內容寄信時才讀最新品項。
pub async fn enqueue_item_changed<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    item_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, guest_id, channel, kind, payload, scheduled_at)
         SELECT r.user_id, r.guest_id, 'email'::notification_channel, 'claim.item_changed',
                jsonb_build_object('wishlist_id', i.wishlist_id, 'item_id', i.id, 'crowdfund', i.funding_mode = 'crowdfund'),
                now() + interval '10 minutes'
           FROM wishlist_items i
           JOIN (SELECT user_id, guest_id FROM claims WHERE item_id = $1 AND status IN ('reserved', 'purchased')
                 UNION SELECT user_id, NULL::uuid FROM contributions WHERE item_id = $1 AND status = 'pledged') r ON true
           LEFT JOIN users u ON u.id = r.user_id
           LEFT JOIN guests g ON g.id = r.guest_id
          WHERE i.id = $1
            AND ((u.email IS NOT NULL AND u.deleted_at IS NULL) OR (g.email IS NOT NULL AND g.deleted_at IS NULL))
         ON CONFLICT DO NOTHING")
        .bind(item_id).execute(ex).await?;
    Ok(())
}

/// 好友申請通知收件人（kind friend.request）。同一寄件人 24 小時內只通知一次，避免撤回 / 重送洗信；
/// 寄信時才讀寄件人名稱，申請已被撤回 / 處理則取消。
pub async fn enqueue_friend_request<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    to_user: Uuid,
    from_user: Uuid,
    request_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT $1, 'email'::notification_channel, 'friend.request',
                jsonb_build_object('request_id', $3::uuid, 'from_user', $2::uuid)
          WHERE NOT EXISTS (SELECT 1 FROM notifications
                             WHERE user_id = $1 AND kind = 'friend.request'
                               AND payload->>'from_user' = $2::text
                               AND created_at > now() - interval '24 hours')",
    )
    .bind(to_user)
    .bind(from_user)
    .bind(request_id)
    .execute(ex)
    .await?;
    Ok(())
}

/// Q12：認領購買期限提醒，截止前 3 天與 24 小時各一封（只排還在未來的）。
/// payload 記下排程時的 expires_at；寄信時若捐助已不是 reserved 或期限已改（延長），worker 會取消不寄。
pub async fn enqueue_purchase_reminders<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    claim_ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications (user_id, guest_id, channel, kind, payload, scheduled_at)
         SELECT c.user_id, c.guest_id, 'email'::notification_channel, 'claim.purchase_reminder',
                jsonb_build_object('wishlist_id', i.wishlist_id, 'item_id', c.item_id, 'claim_id', c.id, 'expires_at', c.expires_at),
                c.expires_at - d.before
           FROM claims c JOIN wishlist_items i ON i.id = c.item_id,
                (VALUES (interval '3 days'), (interval '24 hours')) d(before)
          WHERE c.id = ANY($1) AND c.status = 'reserved' AND c.expires_at - d.before > now()")
        .bind(claim_ids).execute(ex).await?;
    Ok(())
}

/// 唯一寄信點。設了 CF_ACCOUNT_ID + CF_EMAIL_API_TOKEN 就走 Cloudflare Email Service REST API
/// （production 啟動時強制要求）；否則 dev 走 SMTP → Mailpit。錯誤字串不含 token。
pub async fn send_mail(cfg: &Config, to: &str, subject: &str, body: &str) -> Result<(), String> {
    if let (Some(acct), Some(tok)) = (&cfg.cf_account_id, &cfg.cf_email_api_token) {
        let url = format!(
            "{}/accounts/{acct}/email/sending/send",
            cfg.cf_api_base.trim_end_matches('/')
        );
        let r = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().map_err(|e| e.without_url().to_string())?
            .post(url).bearer_auth(tok)
            .json(&serde_json::json!({ "from": cfg.mail_from, "to": to, "subject": subject, "text": body }))
            .send().await.map_err(|e| e.without_url().to_string())?;
        let status = r.status();
        let v: Value = r.json().await.unwrap_or(Value::Null);
        // 2xx 且 success != false 才算成功；永久退信也視為失敗
        if status.is_success()
            && v["success"] != false
            && v["result"]["permanent_bounces"]
                .as_array()
                .is_none_or(|a| a.is_empty())
        {
            return Ok(());
        }
        return Err(format!("cloudflare email {status}: {}", v["errors"]));
    }
    let msg = Message::builder()
        .from(cfg.mail_from.parse().map_err(|e| format!("{e}"))?)
        .to(to.parse().map_err(|e| format!("{e}"))?)
        .subject(subject)
        .header(ContentType::TEXT_PLAIN)
        .body(body.to_string())
        .map_err(|e| e.to_string())?;
    AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(cfg.smtp_host.clone())
        .port(cfg.smtp_port)
        .build()
        .send(msg)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn spawn_worker(pool: PgPool) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = tick(&pool).await {
                tracing::warn!(error = %e, "notify worker");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

type PendingNotification = (Uuid, Option<Uuid>, Option<Uuid>, String, String, Value, i16);

/// 寄送失敗最多嘗試次數（含第一次）；超過標 failed。
pub const MAX_ATTEMPTS: i16 = 6;

/// 第 `attempts` 次失敗後到下一次嘗試的等待：1m / 5m / 30m / 2h / 6h。
pub fn backoff(attempts: i16) -> Duration {
    const STEPS: [u64; 5] = [60, 300, 1800, 7200, 21600];
    Duration::from_secs(STEPS[(attempts.max(1) as usize - 1).min(STEPS.len() - 1)])
}

/// 通知類別：決定能否關閉與是否遵守安靜時段。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    /// 帳務、安全、治理：不可關閉、不延後
    Account,
    /// 送禮進度：偏好 `email_gifts`
    Gifts,
    /// 好友與生日：偏好 `email_social`
    Social,
}

pub fn category(kind: &str) -> Category {
    let head = kind.split('.').next().unwrap_or("");
    if matches!(head, "auth" | "org" | "account" | "wallet" | "topup")
        || matches!(
            kind,
            "staff.digest"
                | "cash_refund.updated"
                | "wishlist.moderated"
                | "order.failed"
                | "order.partial_refund"
                | "funding.expired"
                | "funding.released"
                | "item.removed"
        )
    {
        Category::Account
    } else if matches!(head, "friend" | "birthday") {
        Category::Social
    } else {
        Category::Gifts
    }
}

/// 所有會被 render 的 kind（單元測試用它確認四語文案齊全）。新增 kind 時一併加進這裡、`category` 與 `inbox_link`。
pub const KINDS: &[&str] = &[
    "auth.password_changed", "auth.mfa_reset", "auth.identity_linked", "auth.identity_unlinked",
    "auth.mfa_enabled", "auth.recovery_regenerated",
    "org.invite", "org.permit_revoked", "org.permit_expired", "org.permit_expiring", "org.review",
    "org.permit_reviewed", "org.event_hidden",
    "crowdfund.funded", "order.shipped", "order.delivered", "order.failed", "order.partial_refund",
    "funding.expired", "funding.released", "item.removed",
    "claim.confirmation", "claim.item_removed", "claim.item_changed", "claim.purchase_reminder",
    "claim.list_closed", "claim.expired", "claim.cancelled_by_owner", "claim.cancelled_by_donor",
    "item.reopened", "wishlist.closing", "site.paused",
    "owner.funded", "owner.shipped", "owner.delivered", "owner.order_failed", "event.summary",
    "friend.request", "friend.accepted", "birthday.friend", "birthday.nudge",
    "topup.credited", "topup.expired", "cash_refund.updated",
    "wallet.frozen", "wallet.unfrozen", "wallet.chargeback",
    "wishlist.moderated", "account.suspended", "account.restored", "staff.digest",
];

/// 站內通知點擊後的前端相對路徑。
pub fn inbox_link(kind: &str, payload: &Value) -> String {
    let org = payload["org_id"].as_str().unwrap_or("");
    match kind {
        "claim.confirmation" | "claim.cancelled_by_owner" | "claim.purchase_reminder"
        | "claim.expired" | "claim.list_closed" | "claim.item_removed" | "site.paused" => "/me/claims".into(),
        "claim.item_changed" if payload["crowdfund"] == true => "/me/wallet".into(),
        "claim.item_changed" => "/me/claims".into(),
        "crowdfund.funded" | "order.shipped" | "order.delivered" | "order.failed" | "order.partial_refund"
        | "funding.expired" | "funding.released" | "item.removed" | "topup.credited" | "topup.expired"
        | "cash_refund.updated" | "wallet.frozen" | "wallet.unfrozen" | "wallet.chargeback" => "/me/wallet".into(),
        "friend.request" | "friend.accepted" => "/friends".into(),
        "birthday.friend" => match payload["slug"].as_str() {
            Some(s) => format!("/s/{s}"),
            None => "/friends".into(),
        },
        "birthday.nudge" => "/lists/new".into(),
        "auth.password_changed" | "auth.identity_linked" | "auth.identity_unlinked" | "auth.mfa_enabled"
        | "auth.recovery_regenerated" | "account.suspended" | "account.restored" => "/settings".into(),
        "auth.mfa_reset" | "staff.digest" => "/admin".into(),
        "org.permit_revoked" | "org.permit_expired" | "org.permit_expiring" | "org.permit_reviewed" => {
            format!("/orgs/{org}#permits")
        }
        "org.event_hidden" => format!("/orgs/{org}"),
        "org.invite" | "org.review" => "/org/apply".into(),
        _ => "/dashboard".into(),
    }
}

/// 安靜時段（台北 21:00–09:00）：回傳應延後到的下一個 09:00（UTC）；不在時段內回 None。
pub fn quiet_until(now: chrono::DateTime<chrono::Utc>) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::Timelike;
    let local = now + chrono::Duration::hours(8);
    let h = local.hour();
    if (9..21).contains(&h) {
        return None;
    }
    let day = if h >= 21 { local.date_naive() + chrono::Duration::days(1) } else { local.date_naive() };
    let at = day.and_hms_opt(9, 0, 0)?.and_utc() - chrono::Duration::hours(8);
    Some(at)
}

async fn set_status(pool: &PgPool, id: Uuid, status: &str, err: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE notifications SET status = $2::notification_status, last_error = $3 WHERE id = $1")
        .bind(id).bind(status).bind(err).execute(pool).await?;
    Ok(())
}

/// 處理一批到期通知；回傳領取筆數。順序固定（見計畫 Task 1）：
/// 領取（租約 6 分鐘）→ 安靜時段 → 寄前重驗 → 驚喜把關 → 合併 → render 後寫站內信 → 偏好靜音 → 寄送 / 退避。
pub async fn tick(pool: &PgPool) -> Result<usize, sqlx::Error> {
    tick_at(pool, chrono::Utc::now()).await
}

/// `tick` 的可注入時鐘版本（只影響安靜時段判斷），測試用。
pub async fn tick_at(pool: &PgPool, now: chrono::DateTime<chrono::Utc>) -> Result<usize, sqlx::Error> {
    // 1. 領取：租約須大於一批 20 封 × 15 秒的寄送上限
    let rows: Vec<PendingNotification> = sqlx::query_as(
        "UPDATE notifications SET attempts = attempts + 1, scheduled_at = now() + interval '6 minutes'
          WHERE id IN (SELECT id FROM notifications WHERE status = 'pending' AND scheduled_at <= now()
                        ORDER BY scheduled_at LIMIT 20 FOR UPDATE SKIP LOCKED)
         RETURNING id, user_id, guest_id, channel::text, kind, payload, attempts")
        .fetch_all(pool).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let n = rows.len();
    let mut bundled = std::collections::HashSet::new();
    for row in rows {
        if bundled.contains(&row.0) {
            continue;
        }
        handle(pool, now, row, &mut bundled).await?;
    }
    Ok(n)
}

async fn handle(
    pool: &PgPool,
    now: chrono::DateTime<chrono::Utc>,
    (id, uid, gid, channel, kind, mut payload, attempts): PendingNotification,
    bundled: &mut std::collections::HashSet<Uuid>,
) -> Result<(), sqlx::Error> {
    let cfg = crate::config::get();
    let cat = category(&kind);
    // 2. 安靜時段：延後到 09:00，不算一次嘗試
    if cat != Category::Account && kind != "wishlist.closing" {
        if let Some(at) = quiet_until(now) {
            sqlx::query("UPDATE notifications SET scheduled_at = $2, attempts = attempts - 1 WHERE id = $1")
                .bind(id).bind(at).execute(pool).await?;
            return Ok(());
        }
    }
    // 會員收件人：帳號已刪除就不處理；同時取語言與偏好（偏好在步驟 7 才套用）
    let user: Option<(String, bool, bool)> = match uid {
        Some(u) => {
            let r: Option<(String, bool, bool)> = sqlx::query_as(
                "SELECT locale, COALESCE(notification_prefs->>'email_gifts', 'true') <> 'false',
                        COALESCE(notification_prefs->>'email_social', 'true') <> 'false'
                   FROM users WHERE id = $1 AND deleted_at IS NULL")
                .bind(u).fetch_optional(pool).await?;
            if r.is_none() {
                return set_status(pool, id, "cancelled", "no recipient").await;
            }
            r
        }
        None => None,
    };
    // 3. 寄前重驗
    if kind == "claim.purchase_reminder" {
        // 寄送當下：認領仍待購且期限沒變才寄；清單已結束（Q14）時改用不提「延長」的文案
        let ended: Option<bool> = sqlx::query_scalar(
            "SELECT w.status IN ('closed', 'archived') OR (w.closes_at IS NOT NULL AND w.closes_at <= now())
               FROM claims c JOIN wishlist_items i ON i.id = c.item_id JOIN wishlists w ON w.id = i.wishlist_id
              WHERE c.id = ($1->>'claim_id')::uuid AND c.status = 'reserved' AND c.expires_at = ($1->>'expires_at')::timestamptz")
            .bind(&payload).fetch_optional(pool).await?;
        let Some(ended) = ended else {
            return set_status(pool, id, "cancelled", "claim no longer due").await;
        };
        if ended {
            payload["ended"] = Value::Bool(true);
        }
    } else if kind == "claim.confirmation" {
        // 延後 10 分鐘寄：這段期間認領已取消 / 釋出就不寄
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM claims WHERE id = ($1->>'claim_id')::uuid AND status IN ('reserved', 'purchased'))")
            .bind(&payload).fetch_one(pool).await?;
        if !ok {
            return set_status(pool, id, "cancelled", "claim no longer active").await;
        }
    } else if kind == "friend.request" {
        // 申請可能已被撤回 / 接受 / 拒絕，或寄件人已刪除帳號：不寄
        let from_name: Option<String> = sqlx::query_scalar(
            "SELECT o.display_name FROM friend_requests r JOIN users o ON o.id = r.from_user AND o.deleted_at IS NULL
              WHERE r.id = ($1->>'request_id')::uuid")
            .bind(&payload).fetch_optional(pool).await?;
        let Some(from_name) = from_name else {
            return set_status(pool, id, "cancelled", "friend request no longer pending").await;
        };
        payload["from_name"] = Value::String(from_name);
    } else if kind == "friend.accepted" {
        let from_name: Option<String> = sqlx::query_scalar(
            "SELECT display_name FROM users WHERE id = ($1->>'from_user')::uuid AND deleted_at IS NULL")
            .bind(&payload).fetch_optional(pool).await?;
        let Some(from_name) = from_name else {
            return set_status(pool, id, "cancelled", "friend no longer exists").await;
        };
        payload["from_name"] = Value::String(from_name);
    } else if kind == "birthday.friend" {
        // 寄前重驗（D65）：清單仍開著生日通知且仍看得到、壽星生日仍公開給好友、收件人仍是好友（selected 仍在名單）。偏好於步驟 7 靜音
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT o.display_name, w.slug::text FROM wishlists w
               JOIN users o ON o.id = w.owner_id AND o.deleted_at IS NULL AND o.blocked_at IS NULL
                            AND o.birthday IS NOT NULL AND o.birthday_visibility::text <> 'private'
               JOIN users r ON r.id = $2 AND r.deleted_at IS NULL AND r.blocked_at IS NULL
              WHERE w.id = ($1->>'wishlist_id')::uuid AND w.owner_id = ($1->>'owner_id')::uuid
                AND w.birthday_notify AND w.deleted_at IS NULL AND w.status = 'active' AND w.moderation_status = 'ok'
                AND w.visibility::text <> 'private'
                AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST(r.id, o.id) AND f.user_b = GREATEST(r.id, o.id))
                AND (w.visibility::text <> 'selected' OR EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = r.id))")
            .bind(&payload).bind(uid).fetch_optional(pool).await?;
        let Some((owner_name, slug)) = row else {
            return set_status(pool, id, "cancelled", "birthday list no longer visible").await;
        };
        payload["owner_name"] = owner_name.into();
        payload["slug"] = slug.into();
    } else if kind == "birthday.nudge" {
        let ok: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND birthday IS NOT NULL AND birthday_visibility::text <> 'private')")
            .bind(uid).fetch_one(pool).await?;
        if !ok {
            return set_status(pool, id, "cancelled", "birthday nudge no longer wanted").await;
        }
    } else if kind == "claim.item_changed" {
        // 延後 10 分鐘寄：這段期間取消 / 撤回 / 轉投 / 品項被刪的就不寄
        let still: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM claims WHERE item_id = ($1->>'item_id')::uuid AND status IN ('reserved', 'purchased')
                              AND (user_id = $2 OR guest_id = $3))
                 OR EXISTS (SELECT 1 FROM contributions WHERE item_id = ($1->>'item_id')::uuid AND status = 'pledged' AND user_id = $2)")
            .bind(&payload).bind(uid).bind(gid).fetch_one(pool).await?;
        if !still {
            return set_status(pool, id, "cancelled", "no longer supporting item").await;
        }
    }
    // 4. 驚喜把關：受贈者端通知在驚喜鎖定中一律不寄、不進站內信
    if kind.starts_with("owner.")
        || matches!(kind.as_str(), "claim.cancelled_by_donor" | "item.reopened" | "event.summary")
    {
        let locked: Option<bool> = sqlx::query_scalar(
            "SELECT surprise_locked(surprise_mode, event_date) FROM wishlists WHERE id = ($1->>'wishlist_id')::uuid")
            .bind(&payload).fetch_optional(pool).await?;
        if locked == Some(true) {
            return set_status(pool, id, "cancelled", "surprise locked").await;
        }
    }
    // 5. 合併：同收件人短時間內的同類通知併成一封，其餘標為 sent（last_error 記主列 id）
    if matches!(kind.as_str(), "claim.confirmation" | "claim.purchase_reminder" | "claim.expired" | "item.reopened") {
        let merged: Vec<(Uuid, Option<String>)> = sqlx::query_as(
            "WITH m AS (
               SELECT n.id, i.title FROM notifications n
                 LEFT JOIN wishlist_items i ON i.id = (n.payload->>'item_id')::uuid
                WHERE n.user_id IS NOT DISTINCT FROM $1 AND n.guest_id IS NOT DISTINCT FROM $2
                  AND n.kind = $3 AND n.status = 'pending' AND n.id <> $4
                  AND n.scheduled_at <= now() + interval '6 hours'
                  AND CASE n.kind
                        WHEN 'claim.purchase_reminder' THEN EXISTS (
                          SELECT 1 FROM claims c WHERE c.id = (n.payload->>'claim_id')::uuid
                             AND c.status = 'reserved' AND c.expires_at = (n.payload->>'expires_at')::timestamptz)
                        WHEN 'claim.confirmation' THEN EXISTS (
                          SELECT 1 FROM claims c WHERE c.id = (n.payload->>'claim_id')::uuid AND c.status IN ('reserved', 'purchased'))
                        WHEN 'item.reopened' THEN NOT EXISTS (
                          SELECT 1 FROM wishlists w WHERE w.id = (n.payload->>'wishlist_id')::uuid AND surprise_locked(w.surprise_mode, w.event_date))
                        ELSE true END),
             u AS (
               UPDATE notifications SET status = 'sent', sent_at = now(), last_error = 'bundled:' || $4::text
                 FROM m WHERE notifications.id = m.id RETURNING notifications.id)
             SELECT m.id, m.title FROM m JOIN u ON u.id = m.id")
            .bind(uid).bind(gid).bind(&kind).bind(id).fetch_all(pool).await?;
        if !merged.is_empty() {
            // 寫回 DB：被併入的列已標 sent，主列重試時要靠這份清單；重試時可能再併入新列，所以累加
            let mut items: Vec<Value> = payload.get("more_items").and_then(Value::as_array).cloned().unwrap_or_default();
            items.extend(merged.iter().filter_map(|(_, t)| t.clone()).map(Value::String));
            sqlx::query("UPDATE notifications SET payload = jsonb_set(payload, '{more_items}', $2) WHERE id = $1")
                .bind(id).bind(Value::Array(items.clone())).execute(pool).await?;
            payload["more_items"] = items.into();
            bundled.extend(merged.iter().map(|(i, _)| *i));
        }
    }
    // 6. render
    let title: String = match payload
        .get("wishlist_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<Uuid>().ok())
    {
        Some(w) => sqlx::query_scalar("SELECT title FROM wishlists WHERE id = $1")
            .bind(w)
            .fetch_optional(pool)
            .await?
            .unwrap_or_default(),
        None => String::new(),
    };
    // 眾籌通知：寄信時才查品項名稱（品項可能已被刪除，仍保留列）
    if let Some(it) = payload
        .get("item_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<Uuid>().ok())
    {
        let t: Option<String> = sqlx::query_scalar("SELECT title FROM wishlist_items WHERE id = $1")
            .bind(it)
            .fetch_optional(pool)
            .await?;
        if let Some(t) = t {
            payload["item_title"] = t.into();
        }
    }
    // 退訂連結：Gifts → 'c'、Social → 'b'（會員）；訪客 'g'；Account 不附
    let api = cfg.api_base_url.trim_end_matches('/').to_string();
    let unsub = match (uid, gid, cat) {
        (Some(u), _, Category::Gifts) => Some(crate::account::unsub_token('c', u)),
        (Some(u), _, Category::Social) => Some(crate::account::unsub_token('b', u)),
        (None, Some(g), _) => Some(crate::account::unsub_token('g', g)),
        _ => None,
    };
    if let Some(t) = unsub {
        payload["unsubscribe_url"] = format!("{api}/api/v1/unsubscribe?token={t}").into();
    }
    let lang = match &user {
        Some((locale, ..)) => Lang::parse(locale),
        None => Lang::parse(payload.get("locale").and_then(Value::as_str).unwrap_or_default()),
    };
    let (subject, body) = render_lang(lang, &kind, &title, &payload);
    // 站內信先於 email 判斷：沒有 email、偏好關閉的使用者一樣看得到
    if uid.is_some() {
        let inbox_title = subject
            .strip_prefix("【Wishly】")
            .or_else(|| subject.strip_prefix("[Wishly] "))
            .unwrap_or(&subject);
        sqlx::query("UPDATE notifications SET inbox_at = COALESCE(inbox_at, now()), inbox_title = $2, inbox_link = $3 WHERE id = $1")
            .bind(id).bind(inbox_title).bind(inbox_link(&kind, &payload)).execute(pool).await?;
    }
    // 7. 偏好：關閉的類別只靜音 email，站內信保留
    if let Some((_, gifts, social)) = &user {
        if (cat == Category::Gifts && !gifts) || (cat == Category::Social && !social) {
            return set_status(pool, id, "cancelled", "muted").await;
        }
    }
    // 8. 寄送
    let security = matches!(
        kind.as_str(),
        "auth.identity_linked" | "auth.identity_unlinked" | "auth.password_changed" | "auth.mfa_reset"
            | "auth.mfa_enabled" | "auth.recovery_regenerated"
    );
    let to: Option<String> = if channel != "email" {
        None
    } else if let Some(u) = uid {
        sqlx::query_scalar("SELECT CASE WHEN $2 THEN $3 ELSE email END FROM users WHERE id = $1 AND deleted_at IS NULL")
            .bind(u).bind(security).bind(payload.get("to").and_then(Value::as_str)).fetch_optional(pool).await?.flatten()
    } else if let Some(g) = gid {
        sqlx::query_scalar("SELECT email FROM guests WHERE id = $1 AND deleted_at IS NULL")
            .bind(g).fetch_optional(pool).await?.flatten()
    } else {
        None
    };
    let Some(to) = to else {
        return set_status(pool, id, "cancelled", "no email recipient").await;
    };
    if to.parse::<lettre::Address>().is_err() {
        return set_status(pool, id, "failed", "bad address").await;
    }
    match send_mail(&cfg, &to, &subject, &body).await {
        Ok(()) => {
            sqlx::query("UPDATE notifications SET status = 'sent', sent_at = now(), last_error = NULL WHERE id = $1").bind(id).execute(pool).await?;
        }
        Err(e) if attempts >= MAX_ATTEMPTS => {
            tracing::error!(notification = %id, kind = %kind, error = %e, "notification failed permanently");
            set_status(pool, id, "failed", &e).await?;
        }
        Err(e) => {
            sqlx::query("UPDATE notifications SET last_error = $2, scheduled_at = now() + make_interval(secs => $3) WHERE id = $1")
                .bind(id).bind(&e).bind(backoff(attempts).as_secs_f64()).execute(pool).await?;
        }
    }
    Ok(())
}

/// 台北時間字串（含「（台北時間）」）；沒有時間就寫「期限」
fn at_str(l: Lang, at: Option<String>) -> String {
    match at {
        Some(a) => l.pick(format!("{a}（台北時間）"), format!("{a} (Taipei time)"), format!("{a} (giờ Đài Bắc)"), format!("{a} (waktu Taipei)")),
        None => l.pick("期限".into(), "the deadline".into(), "hạn chót".into(), "batas waktu".into()),
    }
}

/// 信件語言。DB `users.locale` 與 Accept-Language 都以 zh-TW / en / vi 為準，其他值一律退回繁中。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Lang {
    #[default]
    Zh,
    En,
    Vi,
    Id,
}

impl Lang {
    pub fn parse(tag: &str) -> Lang {
        let t = tag.trim().to_ascii_lowercase();
        if t.starts_with("en") {
            Lang::En
        } else if t.starts_with("vi") {
            Lang::Vi
        } else if t.starts_with("id") || t == "in" || t.starts_with("in-") {
            // "in" 是 Indonesian 的舊 ISO 639-1 代碼
            Lang::Id
        } else {
            Lang::Zh
        }
    }
    /// 寫進 users.locale 的值
    pub fn code(self) -> &'static str {
        match self {
            Lang::Zh => "zh-TW",
            Lang::En => "en",
            Lang::Vi => "vi",
            Lang::Id => "id",
        }
    }
    fn pick(self, zh: String, en: String, vi: String, id: String) -> String {
        match self {
            Lang::Zh => zh,
            Lang::En => en,
            Lang::Vi => vi,
            Lang::Id => id,
        }
    }
}

tokio::task_local! {
    /// 目前請求的語言（由 lib.rs 的 middleware 依 Accept-Language 設定），OTP 信等匿名寄信用
    pub static REQUEST_LANG: Lang;
}

pub fn request_lang() -> Lang {
    REQUEST_LANG.try_with(|l| *l).unwrap_or_default()
}

/// 純函式：(主旨, 內文)，繁中。刻意只用數量，不帶品項 / 捐助者。
pub fn render(kind: &str, title: &str, payload: &Value) -> (String, String) {
    render_lang(Lang::Zh, kind, title, payload)
}

/// payload 的 RFC 3339 時間轉台北時間字串
fn taipei(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .and_then(|t| t.parse::<chrono::DateTime<chrono::Utc>>().ok())
        .map(|t| {
            (t + chrono::Duration::hours(8))
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
}

pub fn render_lang(lang: Lang, kind: &str, title: &str, payload: &Value) -> (String, String) {
    let l = lang;
    let app = crate::config::get().app_url;
    if kind == "auth.password_changed" {
        return (
            l.pick("【Wishly】密碼已變更".into(), "[Wishly] Your password was changed".into(), "[Wishly] Mật khẩu đã được thay đổi".into(), "[Wishly] Kata sandi kamu sudah diubah".into()),
            l.pick(
                format!("你的帳號 Email 登入密碼剛剛已變更，其他裝置已登出。\n若非本人操作，請立即使用「忘記密碼」重設。\n帳號設定：{app}/settings\n這是帳號安全通知。"),
                format!("The password for your account's email login was just changed, and your other devices have been signed out.\nIf this wasn't you, reset it right away with \"Forgot password\".\nAccount settings: {app}/settings\nThis is an account security notice."),
                format!("Mật khẩu đăng nhập bằng email của tài khoản bạn vừa được thay đổi, các thiết bị khác đã bị đăng xuất.\nNếu không phải bạn thực hiện, hãy đặt lại ngay bằng \"Quên mật khẩu\".\nCài đặt tài khoản: {app}/settings\nĐây là thông báo bảo mật tài khoản."),
                format!("Kata sandi login email akun kamu baru saja diubah, dan perangkat lain sudah dikeluarkan.\nKalau bukan kamu yang melakukannya, segera atur ulang lewat \"Lupa kata sandi\".\nPengaturan akun: {app}/settings\nIni adalah pemberitahuan keamanan akun."),
            ),
        );
    }
    if kind == "auth.mfa_reset" {
        return (
            l.pick("【Wishly】兩步驟驗證已被重設".into(), "[Wishly] Your two-step verification was reset".into(), "[Wishly] Xác minh hai bước của bạn đã bị đặt lại".into(), "[Wishly] Verifikasi dua langkah kamu diatur ulang".into()),
            l.pick(
                format!("另一位營運人員剛剛重設了你的兩步驟驗證，你的所有裝置都已登出；下次進後台要重新設定驗證器。\n若你沒有提出這個要求，請立即聯絡其他營運人員並檢查稽核紀錄。\n後台：{app}/admin\n這是帳號安全通知。"),
                format!("Another staff member just reset your two-step verification, and all your devices have been signed out. You'll set up an authenticator again next time you open the console.\nIf you didn't ask for this, contact other staff right away and check the audit log.\nConsole: {app}/admin\nThis is an account security notice."),
                format!("Một nhân viên vận hành khác vừa đặt lại xác minh hai bước của bạn, mọi thiết bị đã bị đăng xuất. Lần tới vào trang quản trị bạn cần thiết lập lại ứng dụng xác thực.\nNếu bạn không yêu cầu việc này, hãy liên hệ ngay nhân viên khác và kiểm tra nhật ký kiểm toán.\nTrang quản trị: {app}/admin\nĐây là thông báo bảo mật tài khoản."),
                format!("Staf lain baru saja mengatur ulang verifikasi dua langkah kamu, dan semua perangkat kamu sudah dikeluarkan. Saat membuka konsol berikutnya, kamu perlu menyiapkan aplikasi autentikator lagi.\nKalau kamu tidak memintanya, segera hubungi staf lain dan periksa log audit.\nKonsol: {app}/admin\nIni adalah pemberitahuan keamanan akun."),
            ),
        );
    }
    if kind == "org.invite" {
        let (org, url) = (
            payload["org_display_name"].as_str().unwrap_or(""),
            payload["url"].as_str().unwrap_or(&app),
        );
        return (
            l.pick(format!("【Wishly】邀請你加入「{org}」"), format!("[Wishly] You're invited to join \"{org}\""), format!("[Wishly] Bạn được mời tham gia \"{org}\""), format!("[Wishly] Kamu diundang bergabung dengan \"{org}\"")),
            l.pick(
                format!("「{org}」邀請你加入組織。\n請用此 Email 登入後接受邀請（7 天內有效）：{url}\n"),
                format!("\"{org}\" has invited you to join the organization.\nSign in with this email and accept within 7 days: {url}\n"),
                format!("\"{org}\" mời bạn tham gia tổ chức.\nHãy đăng nhập bằng email này và chấp nhận trong 7 ngày: {url}\n"),
                format!("\"{org}\" mengundangmu bergabung dengan organisasi.\nMasuk dengan email ini dan terima dalam 7 hari: {url}\n"),
            ),
        );
    }
    if matches!(kind, "auth.identity_linked" | "auth.identity_unlinked") {
        let linked = kind == "auth.identity_linked";
        let provider = match payload["provider"].as_str() {
            Some("google") => "Google".to_string(),
            Some("line") => "LINE".to_string(),
            Some("email") => "Email".to_string(),
            _ => l.pick(
                "第三方".into(),
                "third-party".into(),
                "bên thứ ba".into(),
                "pihak ketiga".into(),
            ),
        };
        let action_zh = if linked { "新增" } else { "解除" };
        let action_en = if linked { "added" } else { "removed" };
        let action_vi = if linked { "thêm" } else { "gỡ" };
        let action_id = if linked { "ditambahkan" } else { "dihapus" };
        return (
            l.pick("【Wishly】登入方式已變更".into(), "[Wishly] Sign-in method changed".into(), "[Wishly] Phương thức đăng nhập đã thay đổi".into(), "[Wishly] Metode masuk diubah".into()),
            l.pick(
                format!("你的帳號已{action_zh} {provider} 登入方式。\n若非本人操作，請立即檢查帳號設定及重設密碼。\n帳號設定：{app}/settings\n這是帳號安全通知。"),
                format!("The {provider} sign-in method was {action_en} on your account.\nIf this wasn't you, check your account settings and reset your password right away.\nAccount settings: {app}/settings\nThis is an account security notice."),
                format!("Phương thức đăng nhập {provider} đã được {action_vi} cho tài khoản của bạn.\nNếu không phải bạn thực hiện, hãy kiểm tra cài đặt tài khoản và đặt lại mật khẩu ngay.\nCài đặt tài khoản: {app}/settings\nĐây là thông báo bảo mật tài khoản."),
                format!("Metode masuk {provider} sudah {action_id} di akun kamu.\nKalau bukan kamu yang melakukannya, segera periksa pengaturan akun dan atur ulang kata sandi.\nPengaturan akun: {app}/settings\nIni adalah pemberitahuan keamanan akun."),
            ),
        );
    }
    if kind == "org.permit_revoked" {
        let name = payload["display_name"].as_str().unwrap_or("");
        let no = payload["permit_no"].as_str().unwrap_or("");
        let note = payload["note"]
            .as_str()
            .filter(|n| !n.is_empty())
            .unwrap_or("-");
        let url = format!(
            "{app}/orgs/{}#permits",
            payload["org_id"].as_str().unwrap_or("")
        );
        return (
            l.pick(format!("【Wishly】勸募許可 {no} 已撤銷"), format!("[Wishly] Fundraising permit {no} revoked"), format!("[Wishly] Giấy phép gây quỹ {no} đã bị thu hồi"), format!("[Wishly] Izin penggalangan dana {no} dicabut")),
            l.pick(
                format!("組織「{name}」的勸募許可 {no} 已被撤銷，使用此許可的救災據點已暫停。\n請到組織後台的勸募許可區上傳新許可送審，通過後在據點編輯頁換用新許可再恢復。\n備註：{note}\n勸募許可區：{url}\n"),
                format!("The fundraising permit {no} of \"{name}\" has been revoked, and relief sites using it are paused.\nIn your organization dashboard, upload a new permit in the Fundraising permits section; once approved, switch your sites to the new permit on each site's edit page and resume them.\nNote: {note}\nPermits section: {url}\n"),
                format!("Giấy phép gây quỹ {no} của \"{name}\" đã bị thu hồi, các điểm cứu trợ dùng giấy phép này đã tạm dừng.\nTrong trang quản trị tổ chức, hãy tải lên giấy phép mới ở mục Giấy phép gây quỹ; sau khi được duyệt, đổi các điểm sang giấy phép mới ở trang chỉnh sửa điểm rồi mở lại.\nGhi chú: {note}\nMục giấy phép: {url}\n"),
                format!("Izin penggalangan dana {no} milik \"{name}\" dicabut, dan lokasi bantuan yang memakainya dijeda.\nDi dasbor organisasi, unggah izin baru di bagian Izin penggalangan dana; setelah disetujui, ganti izin lokasi di halaman edit lokasi lalu lanjutkan.\nCatatan: {note}\nBagian izin: {url}\n"),
            ),
        );
    }
    if kind == "org.permit_expired" {
        let name = payload["display_name"].as_str().unwrap_or("");
        let no = payload["permit_no"].as_str().unwrap_or("");
        let url = format!(
            "{app}/orgs/{}#permits",
            payload["org_id"].as_str().unwrap_or("")
        );
        return (
            l.pick(format!("【Wishly】勸募許可 {no} 已到期"), format!("[Wishly] Fundraising permit {no} expired"), format!("[Wishly] Giấy phép gây quỹ {no} đã hết hạn"), format!("[Wishly] Izin penggalangan dana {no} kedaluwarsa")),
            l.pick(
                format!("組織「{name}」的勸募許可 {no} 已到期，使用此許可的救災據點已暫停。\n請到組織後台的勸募許可區上傳新許可送審，通過後在據點編輯頁換用新許可再恢復。\n勸募許可區：{url}\n"),
                format!("The fundraising permit {no} of \"{name}\" has expired, and relief sites using it are paused.\nIn your organization dashboard, upload a new permit in the Fundraising permits section; once approved, switch your sites to the new permit on each site's edit page and resume them.\nPermits section: {url}\n"),
                format!("Giấy phép gây quỹ {no} của \"{name}\" đã hết hạn, các điểm cứu trợ dùng giấy phép này đã tạm dừng.\nTrong trang quản trị tổ chức, hãy tải lên giấy phép mới ở mục Giấy phép gây quỹ; sau khi được duyệt, đổi các điểm sang giấy phép mới ở trang chỉnh sửa điểm rồi mở lại.\nMục giấy phép: {url}\n"),
                format!("Izin penggalangan dana {no} milik \"{name}\" kedaluwarsa, dan lokasi bantuan yang memakainya dijeda.\nDi dasbor organisasi, unggah izin baru di bagian Izin penggalangan dana; setelah disetujui, ganti izin lokasi di halaman edit lokasi lalu lanjutkan.\nBagian izin: {url}\n"),
            ),
        );
    }
    if kind == "org.permit_expiring" {
        let name = payload["display_name"].as_str().unwrap_or("");
        let no = payload["permit_no"].as_str().unwrap_or("");
        let to = payload["valid_to"].as_str().unwrap_or("");
        let days = payload["days"].as_i64().unwrap_or(0);
        let url = format!(
            "{app}/orgs/{}#permits",
            payload["org_id"].as_str().unwrap_or("")
        );
        return (
            l.pick(format!("【Wishly】勸募許可 {no} 將於 {days} 天內到期"), format!("[Wishly] Fundraising permit {no} expires within {days} days"), format!("[Wishly] Giấy phép gây quỹ {no} sẽ hết hạn trong {days} ngày"), format!("[Wishly] Izin penggalangan dana {no} kedaluwarsa dalam {days} hari")),
            l.pick(
                format!("組織「{name}」的勸募許可 {no} 將於 {to} 到期，到期後使用此許可的救災據點會自動暫停。\n請到組織後台的勸募許可區上傳新許可送審，通過後在據點編輯頁換用新許可。\n勸募許可區：{url}\n"),
                format!("The fundraising permit {no} of \"{name}\" expires on {to}; relief sites using it will be paused automatically.\nIn your organization dashboard, upload a new permit in the Fundraising permits section; once approved, switch your sites to it on each site's edit page.\nPermits section: {url}\n"),
                format!("Giấy phép gây quỹ {no} của \"{name}\" sẽ hết hạn vào {to}; các điểm cứu trợ dùng giấy phép này sẽ tự động tạm dừng.\nTrong trang quản trị tổ chức, hãy tải lên giấy phép mới ở mục Giấy phép gây quỹ; sau khi được duyệt, đổi các điểm sang giấy phép mới ở trang chỉnh sửa điểm.\nMục giấy phép: {url}\n"),
                format!("Izin penggalangan dana {no} milik \"{name}\" kedaluwarsa pada {to}; lokasi bantuan yang memakainya akan dijeda otomatis.\nDi dasbor organisasi, unggah izin baru di bagian Izin penggalangan dana; setelah disetujui, ganti izin lokasi di halaman edit lokasi.\nBagian izin: {url}\n"),
            ),
        );
    }
    if kind == "org.review" {
        let name = payload["display_name"].as_str().unwrap_or("");
        let note = payload["note"]
            .as_str()
            .filter(|n| !n.is_empty())
            .unwrap_or("-");
        let d = payload["decision"].as_str().unwrap_or("");
        let (zh, en, vi, id) = match d {
            "verified" => (
                "已通過審核",
                "has been approved",
                "đã được duyệt",
                "telah disetujui",
            ),
            "needs_changes" => (
                "需要補件或修正",
                "needs changes",
                "cần bổ sung hoặc chỉnh sửa",
                "perlu perbaikan",
            ),
            _ if payload["revoked"].as_bool() == Some(true) => (
                "驗證已撤銷",
                "verification has been revoked",
                "đã bị thu hồi xác minh",
                "verifikasinya dicabut",
            ),
            _ => (
                "未通過審核",
                "was not approved",
                "không được duyệt",
                "tidak disetujui",
            ),
        };
        return (
            l.pick(format!("【Wishly】組織「{name}」{zh}"), format!("[Wishly] Organization \"{name}\" {en}"), format!("[Wishly] Tổ chức \"{name}\" {vi}"), format!("[Wishly] Organisasi \"{name}\" {id}")),
            l.pick(
                format!("你的組織「{name}」{zh}。\n審核備註：{note}\n組織申請頁面：{app}/org/apply\n"),
                format!("Your organization \"{name}\" {en}.\nReview note: {note}\nOrganization application: {app}/org/apply\n"),
                format!("Tổ chức \"{name}\" của bạn {vi}.\nGhi chú xét duyệt: {note}\nTrang đăng ký tổ chức: {app}/org/apply\n"),
                format!("Organisasi \"{name}\" kamu {id}.\nCatatan peninjauan: {note}\nHalaman pendaftaran organisasi: {app}/org/apply\n"),
            ),
        );
    }
    let base = app;
    let manage = format!("{base}/me/claims");
    let dash = format!("{base}/dashboard");
    let item = payload
        .get("item_title")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            l.pick(
                "品項".into(),
                "item".into(),
                "món quà".into(),
                "barang".into(),
            )
        });
    let wallet = format!("{base}/me/wallet");
    let (subject, body) = match kind {
        "crowdfund.funded" => (
            l.pick(format!("【Wishly】《{item}》集資達標了"), format!("[Wishly] \"{item}\" has reached its goal"), format!("[Wishly] \"{item}\" đã đạt mục tiêu góp quà"), format!("[Wishly] Patungan \"{item}\" sudah mencapai target")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）已集資達標，營運團隊會開始代購。\n查看我的點數與贊助：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\"), which you backed, has reached its goal. Our team will start purchasing it.\nView my points and contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") mà bạn ủng hộ đã đạt mục tiêu, đội ngũ của chúng tôi sẽ bắt đầu mua hộ.\nXem điểm và các khoản tài trợ của tôi: {wallet}\n"),
                format!("\"{item}\" (wishlist \"{title}\") yang kamu dukung sudah mencapai target. Tim kami akan mulai membelikannya.\nLihat poin dan patunganku: {wallet}\n"),
            ),
        ),
        "order.shipped" => (
            l.pick(format!("【Wishly】《{item}》已出貨"), format!("[Wishly] \"{item}\" has shipped"), format!("[Wishly] \"{item}\" đã được gửi đi"), format!("[Wishly] \"{item}\" sudah dikirim")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）已出貨。\n查看我的點數與贊助：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\"), which you backed, has shipped.\nView my points and contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") mà bạn ủng hộ đã được gửi đi.\nXem điểm và các khoản tài trợ của tôi: {wallet}\n"),
                format!("\"{item}\" (wishlist \"{title}\") yang kamu dukung sudah dikirim.\nLihat poin dan patunganku: {wallet}\n"),
            ),
        ),
        "order.delivered" => (
            l.pick(format!("【Wishly】《{item}》已送達"), format!("[Wishly] \"{item}\" has been delivered"), format!("[Wishly] \"{item}\" đã được giao"), format!("[Wishly] \"{item}\" sudah sampai")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）已送達受贈者，謝謝你的心意。\n查看我的點數與贊助：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\"), which you backed, has been delivered to the recipient. Thank you for your kindness.\nView my points and contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") mà bạn ủng hộ đã được giao đến người nhận. Cảm ơn tấm lòng của bạn.\nXem điểm và các khoản tài trợ của tôi: {wallet}\n"),
                format!("\"{item}\" (wishlist \"{title}\") yang kamu dukung sudah sampai ke penerima. Terima kasih atas niat baikmu.\nLihat poin dan patunganku: {wallet}\n"),
            ),
        ),
        "order.failed" => (
            l.pick(format!("【Wishly】《{item}》代購未成功，點數已退回"), format!("[Wishly] \"{item}\" could not be purchased; points refunded"), format!("[Wishly] Không mua được \"{item}\", điểm đã được hoàn lại"), format!("[Wishly] \"{item}\" gagal dibeli, poin dikembalikan")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）代購未成功，你贊助的點數已全數退回錢包。\n查看我的點數：{wallet}\n"),
                format!("The purchase of \"{item}\" (list \"{title}\"), which you backed, was unsuccessful. All the points you contributed have been refunded to your wallet.\nView my points: {wallet}\n"),
                format!("Việc mua \"{item}\" (danh sách \"{title}\") mà bạn ủng hộ không thành công. Toàn bộ điểm bạn đã tài trợ đã được hoàn vào ví.\nXem điểm của tôi: {wallet}\n"),
                format!("Pembelian \"{item}\" (wishlist \"{title}\") yang kamu dukung tidak berhasil. Semua poin yang kamu pakai untuk patungan sudah dikembalikan ke dompetmu.\nLihat poinku: {wallet}\n"),
            ),
        ),
        "funding.expired" => (
            l.pick(format!("【Wishly】《{item}》募集期限已到"), format!("[Wishly] The funding period for \"{item}\" has ended"), format!("[Wishly] \"{item}\" đã hết hạn góp quà"), format!("[Wishly] Masa penggalangan \"{item}\" sudah berakhir")),
            l.pick(
                format!("「{item}」（清單《{title}》）在期限內未達標。你的點數會再保留 7 天：期間可到錢包撤回，或轉投到同一份清單的其他品項；逾期未處理會自動退回錢包。\n處理我的贊助：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\") did not reach its goal in time. Your points are held for another 7 days: you can withdraw them in your wallet or move them to another item on the same list; if you do nothing they are refunded to your wallet automatically.\nManage my contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") chưa đạt mục tiêu trong thời hạn. Điểm của bạn được giữ thêm 7 ngày: bạn có thể rút về ví hoặc chuyển sang món khác trong cùng danh sách; nếu không xử lý, điểm sẽ tự động hoàn vào ví.\nQuản lý khoản tài trợ của tôi: {wallet}\n"),
                format!("\"{item}\" (wishlist \"{title}\") tidak mencapai target tepat waktu. Poinmu ditahan 7 hari lagi: kamu bisa menariknya ke dompet atau memindahkannya ke barang lain di wishlist yang sama; kalau tidak diurus, poin otomatis dikembalikan ke dompetmu.\nKelola patunganku: {wallet}\n"),
            ),
        ),
        "item.removed" => (
            l.pick(format!("【Wishly】《{item}》已被移除，點數已退回"), format!("[Wishly] \"{item}\" was removed; points refunded"), format!("[Wishly] \"{item}\" đã bị gỡ, điểm đã được hoàn lại"), format!("[Wishly] \"{item}\" dihapus, poin dikembalikan")),
            l.pick(
                format!("清單《{title}》的建立者移除了「{item}」（或封存了清單），你贊助的點數已退回錢包。\n查看我的點數：{wallet}\n"),
                format!("The owner of list \"{title}\" removed \"{item}\" (or archived the list), and the points you contributed have been refunded to your wallet.\nView my points: {wallet}\n"),
                format!("Chủ danh sách \"{title}\" đã gỡ \"{item}\" (hoặc lưu trữ danh sách), điểm bạn đã tài trợ đã được hoàn vào ví.\nXem điểm của tôi: {wallet}\n"),
                format!("Pemilik wishlist \"{title}\" menghapus \"{item}\" (atau mengarsipkan wishlist), dan poin yang kamu pakai untuk patungan sudah dikembalikan ke dompetmu.\nLihat poinku: {wallet}\n"),
            ),
        ),
        "claim.confirmation" => {
            let by = at_str(l, taipei(payload, "expires_at"));
            (
                l.pick(format!("【Wishly】已記下：由你送「{item}」"), format!("[Wishly] Noted: you're giving \"{item}\""), format!("[Wishly] Đã ghi nhận: bạn sẽ tặng \"{item}\""), format!("[Wishly] Tercatat: kamu yang kasih \"{item}\"")),
                l.pick(
                    format!("已記下由你送「{item}」（清單《{title}》），謝謝你的心意。\n請在 {by}前標記已購買，逾期會釋出給其他朋友。\n我的送禮：{manage}\n"),
                    format!("It's recorded that you're giving \"{item}\" (list \"{title}\"). Thank you for your kindness.\nPlease mark it as purchased by {by}; otherwise it will be released to other friends.\nMy gifts: {manage}\n"),
                    format!("Đã ghi nhận bạn sẽ tặng \"{item}\" (danh sách \"{title}\"). Cảm ơn tấm lòng của bạn.\nVui lòng đánh dấu đã mua trước {by}; quá hạn sẽ được nhường cho bạn bè khác.\nQuà tôi tặng: {manage}\n"),
                    format!("Tercatat: kamu yang kasih \"{item}\" (wishlist \"{title}\"). Terima kasih atas niat baikmu.\nMohon tandai sudah dibeli sebelum {by}; kalau tidak, barang ini dilepas untuk teman lain.\nHadiah dariku: {manage}\n"),
                ),
            )
        }
        "claim.cancelled_by_owner" => (
            l.pick(format!("【Wishly】你對「{item}」的認領已被取消"), format!("[Wishly] Your claim on \"{item}\" was cancelled"), format!("[Wishly] Lượt nhận \"{item}\" của bạn đã bị hủy"), format!("[Wishly] Klaimmu atas \"{item}\" dibatalkan")),
            l.pick(
                format!("清單《{title}》的建立者取消了你對「{item}」的認領。若你已經買了，請自行處理這份禮物。\n我的送禮：{manage}\n"),
                format!("The owner of list \"{title}\" cancelled your claim on \"{item}\". If you've already bought it, please handle the gift yourself.\nMy gifts: {manage}\n"),
                format!("Chủ danh sách \"{title}\" đã hủy lượt nhận \"{item}\" của bạn. Nếu bạn đã mua, vui lòng tự xử lý món quà.\nQuà tôi tặng: {manage}\n"),
                format!("Pemilik wishlist \"{title}\" membatalkan klaimmu atas \"{item}\". Kalau kamu sudah membelinya, mohon urus hadiahnya sendiri.\nHadiah dariku: {manage}\n"),
            ),
        ),
        // 寄給擁有者：不提是誰取消，也不帶品項名稱
        "claim.cancelled_by_donor" => (
            l.pick(format!("【Wishly】《{title}》有一份禮物取消了"), format!("[Wishly] A gift on \"{title}\" was cancelled"), format!("[Wishly] Một món quà trong \"{title}\" đã bị hủy"), format!("[Wishly] Satu hadiah di \"{title}\" dibatalkan")),
            l.pick(
                format!("《{title}》有一份已購買的禮物被取消了，品項已重新開放認領。\n{dash}\n"),
                format!("A gift that had been marked as purchased on \"{title}\" was cancelled, and the item is open for claiming again.\n{dash}\n"),
                format!("Một món quà đã mua trong \"{title}\" đã bị hủy, món này được mở lại để nhận.\n{dash}\n"),
                format!("Satu hadiah yang sudah dibeli di \"{title}\" dibatalkan, dan barangnya dibuka lagi untuk diklaim.\n{dash}\n"),
            ),
        ),
        "funding.released" => (
            l.pick(format!("【Wishly】《{item}》的點數已退回錢包"), format!("[Wishly] Points for \"{item}\" refunded to your wallet"), format!("[Wishly] Điểm cho \"{item}\" đã hoàn vào ví"), format!("[Wishly] Poin untuk \"{item}\" dikembalikan ke dompet")),
            l.pick(
                format!("「{item}」（清單《{title}》）的 7 天選擇期已結束，你的點數已自動退回錢包。\n查看我的點數：{wallet}\n"),
                format!("The 7-day choice period for \"{item}\" (list \"{title}\") has ended, and your points have been refunded to your wallet automatically.\nView my points: {wallet}\n"),
                format!("Thời gian 7 ngày lựa chọn của \"{item}\" (danh sách \"{title}\") đã kết thúc, điểm của bạn đã tự động hoàn vào ví.\nXem điểm của tôi: {wallet}\n"),
                format!("Masa pilih 7 hari untuk \"{item}\" (wishlist \"{title}\") sudah berakhir, dan poinmu otomatis dikembalikan ke dompet.\nLihat poinku: {wallet}\n"),
            ),
        ),
        "order.partial_refund" => (
            l.pick(format!("【Wishly】《{item}》實際花費較低，差額已退回"), format!("[Wishly] \"{item}\" cost less; the difference was refunded"), format!("[Wishly] \"{item}\" tốn ít hơn, phần chênh lệch đã hoàn lại"), format!("[Wishly] \"{item}\" lebih murah, selisihnya dikembalikan")),
            l.pick(
                format!("「{item}」（清單《{title}》）的實際花費低於目標，差額已依比例退回你的錢包。\n查看我的點數：{wallet}\n"),
                format!("The actual cost of \"{item}\" (list \"{title}\") was lower than the goal, and the difference has been refunded to your wallet in proportion.\nView my points: {wallet}\n"),
                format!("Chi phí thực tế của \"{item}\" (danh sách \"{title}\") thấp hơn mục tiêu, phần chênh lệch đã được hoàn vào ví của bạn theo tỷ lệ.\nXem điểm của tôi: {wallet}\n"),
                format!("Biaya sebenarnya \"{item}\" (wishlist \"{title}\") lebih rendah dari target, dan selisihnya sudah dikembalikan ke dompetmu secara proporsional.\nLihat poinku: {wallet}\n"),
            ),
        ),
        "owner.funded" => (
            l.pick(format!("【Wishly】你的《{item}》集資達標了"), format!("[Wishly] Your \"{item}\" has reached its goal"), format!("[Wishly] \"{item}\" của bạn đã đạt mục tiêu"), format!("[Wishly] \"{item}\" milikmu sudah mencapai target")),
            l.pick(
                format!("你清單《{title}》的「{item}」已集資達標，營運團隊會開始代購。\n{dash}\n"),
                format!("\"{item}\" on your list \"{title}\" has reached its funding goal. Our team will start purchasing it.\n{dash}\n"),
                format!("\"{item}\" trong danh sách \"{title}\" của bạn đã đạt mục tiêu góp quà, đội ngũ của chúng tôi sẽ bắt đầu mua hộ.\n{dash}\n"),
                format!("\"{item}\" di wishlist \"{title}\" milikmu sudah mencapai target patungan. Tim kami akan mulai membelikannya.\n{dash}\n"),
            ),
        ),
        "owner.shipped" => (
            l.pick(format!("【Wishly】你的《{item}》已出貨"), format!("[Wishly] Your \"{item}\" has shipped"), format!("[Wishly] \"{item}\" của bạn đã được gửi đi"), format!("[Wishly] \"{item}\" milikmu sudah dikirim")),
            l.pick(
                format!("你清單《{title}》的「{item}」已出貨。\n{dash}\n"),
                format!("\"{item}\" on your list \"{title}\" has shipped.\n{dash}\n"),
                format!("\"{item}\" trong danh sách \"{title}\" của bạn đã được gửi đi.\n{dash}\n"),
                format!("\"{item}\" di wishlist \"{title}\" milikmu sudah dikirim.\n{dash}\n"),
            ),
        ),
        "owner.delivered" => (
            l.pick(format!("【Wishly】你的《{item}》已送達"), format!("[Wishly] Your \"{item}\" has been delivered"), format!("[Wishly] \"{item}\" của bạn đã được giao"), format!("[Wishly] \"{item}\" milikmu sudah sampai")),
            l.pick(
                format!("你清單《{title}》的「{item}」已送達。\n{dash}\n"),
                format!("\"{item}\" on your list \"{title}\" has been delivered.\n{dash}\n"),
                format!("\"{item}\" trong danh sách \"{title}\" của bạn đã được giao.\n{dash}\n"),
                format!("\"{item}\" di wishlist \"{title}\" milikmu sudah sampai.\n{dash}\n"),
            ),
        ),
        "owner.order_failed" => (
            l.pick(format!("【Wishly】你的《{item}》代購未成功"), format!("[Wishly] The purchase of your \"{item}\" was unsuccessful"), format!("[Wishly] Việc mua \"{item}\" của bạn không thành công"), format!("[Wishly] Pembelian \"{item}\" milikmu tidak berhasil")),
            l.pick(
                format!("你清單《{title}》的「{item}」代購未成功。\n{dash}\n"),
                format!("The purchase of \"{item}\" on your list \"{title}\" was unsuccessful.\n{dash}\n"),
                format!("Việc mua \"{item}\" trong danh sách \"{title}\" của bạn không thành công.\n{dash}\n"),
                format!("Pembelian \"{item}\" di wishlist \"{title}\" milikmu tidak berhasil.\n{dash}\n"),
            ),
        ),
        // 只帶人數，不帶任何捐助者身分
        "event.summary" => {
            let n = payload["givers"].as_i64().unwrap_or(0);
            (
                l.pick(format!("【Wishly】有 {n} 位朋友為《{title}》送上心意"), format!("[Wishly] {n} friends sent their love for \"{title}\""), format!("[Wishly] {n} người bạn đã gửi tấm lòng cho \"{title}\""), format!("[Wishly] {n} teman memberi hadiah untuk \"{title}\"")),
                l.pick(
                    format!("你的清單《{title}》活動日已過，共有 {n} 位朋友為你送上心意。\n{dash}\n"),
                    format!("The event day for your list \"{title}\" has passed, and {n} friends sent their love.\n{dash}\n"),
                    format!("Ngày sự kiện của danh sách \"{title}\" đã qua, có {n} người bạn đã gửi tấm lòng đến bạn.\n{dash}\n"),
                    format!("Hari acara wishlist \"{title}\" sudah lewat, dan {n} teman memberi hadiah untukmu.\n{dash}\n"),
                ),
            )
        }
        "site.paused" => (
            l.pick(format!("【Wishly】據點《{title}》已暫停"), format!("[Wishly] Site \"{title}\" is paused"), format!("[Wishly] Điểm \"{title}\" đã tạm dừng"), format!("[Wishly] Lokasi \"{title}\" dijeda")),
            l.pick(
                format!("你支持的據點《{title}》已暫停。請先不要寄出物資，到「我的送禮」確認最新狀況。\n我的送禮：{manage}\n"),
                format!("The relief site \"{title}\" you're supporting is paused. Please hold off on sending supplies and check the latest status in My gifts.\nMy gifts: {manage}\n"),
                format!("Điểm cứu trợ \"{title}\" bạn ủng hộ đã tạm dừng. Vui lòng chưa gửi vật phẩm và xem tình hình mới nhất trong Quà tôi tặng.\nQuà tôi tặng: {manage}\n"),
                format!("Lokasi bantuan \"{title}\" yang kamu dukung dijeda. Mohon jangan kirim barang dulu dan cek kabar terbaru di Hadiah dariku.\nHadiah dariku: {manage}\n"),
            ),
        ),
        "friend.accepted" => {
            let from = payload["from_name"].as_str().unwrap_or_default();
            (
                l.pick(format!("【Wishly】{from} 接受了你的好友邀請"), format!("[Wishly] {from} accepted your friend request"), format!("[Wishly] {from} đã chấp nhận lời mời kết bạn của bạn"), format!("[Wishly] {from} menerima permintaan pertemananmu")),
                l.pick(
                    format!("{from} 接受了你的好友邀請，你們現在是好友了。\n我的好友：{base}/friends\n"),
                    format!("{from} accepted your friend request, and you're now friends.\nMy friends: {base}/friends\n"),
                    format!("{from} đã chấp nhận lời mời kết bạn của bạn, hai bạn giờ là bạn bè.\nBạn bè của tôi: {base}/friends\n"),
                    format!("{from} menerima permintaan pertemananmu, kalian sekarang berteman.\nTemanku: {base}/friends\n"),
                ),
            )
        }
        "topup.credited" => {
            let a = payload["amount"].as_i64().unwrap_or(0);
            (
                l.pick(format!("【Wishly】儲值 NT${a} 已入帳"), format!("[Wishly] Top-up of NT${a} credited"), format!("[Wishly] Đã nạp NT${a} vào ví"), format!("[Wishly] Top-up NT${a} sudah masuk")),
                l.pick(
                    format!("你的儲值 NT${a} 已入帳，可以到錢包查看。\n查看我的點數：{wallet}\n"),
                    format!("Your top-up of NT${a} has been credited. You can check it in your wallet.\nView my points: {wallet}\n"),
                    format!("Khoản nạp NT${a} của bạn đã được ghi vào ví. Bạn có thể xem trong ví.\nXem điểm của tôi: {wallet}\n"),
                    format!("Top-up NT${a} kamu sudah masuk. Kamu bisa melihatnya di dompet.\nLihat poinku: {wallet}\n"),
                ),
            )
        }
        "topup.expired" => {
            let a = payload["amount"].as_i64().unwrap_or(0);
            (
                l.pick("【Wishly】儲值訂單已失效".into(), "[Wishly] Top-up order expired".into(), "[Wishly] Đơn nạp tiền đã hết hiệu lực".into(), "[Wishly] Pesanan top-up kedaluwarsa".into()),
                l.pick(
                    format!("你的儲值 NT${a} 繳費期限已過，訂單已失效。若仍要儲值，請重新建立訂單。\n我的錢包：{wallet}\n"),
                    format!("The payment deadline for your NT${a} top-up has passed and the order has expired. To top up, please create a new order.\nMy wallet: {wallet}\n"),
                    format!("Hạn thanh toán khoản nạp NT${a} của bạn đã qua và đơn đã hết hiệu lực. Nếu vẫn muốn nạp, vui lòng tạo đơn mới.\nVí của tôi: {wallet}\n"),
                    format!("Batas pembayaran top-up NT${a} kamu sudah lewat dan pesanan kedaluwarsa. Kalau masih mau top-up, buat pesanan baru.\nDompetku: {wallet}\n"),
                ),
            )
        }
        "cash_refund.updated" => {
            let a = payload["amount"].as_i64().unwrap_or(0);
            let note = payload["note"].as_str().filter(|n| !n.is_empty()).unwrap_or("-");
            let done = payload["status"].as_str() == Some("completed");
            let (sz, se, sv, si, rz, re, rv, ri) = if done {
                ("已完成", "completed", "đã hoàn tất", "selesai", "已完成退款", "has been completed", "đã được hoàn tất", "sudah selesai")
            } else {
                ("未通過", "was not approved", "không được duyệt", "tidak disetujui", "未通過審核", "was not approved", "không được duyệt", "tidak disetujui")
            };
            (
                l.pick(format!("【Wishly】現金退款申請{sz}"), format!("[Wishly] Your cash refund request {se}"), format!("[Wishly] Yêu cầu hoàn tiền mặt {sv}"), format!("[Wishly] Permintaan pengembalian dana tunai {si}")),
                l.pick(
                    format!("你申請的現金退款 NT${a} {rz}。\n備註：{note}\n我的錢包：{wallet}\n"),
                    format!("Your cash refund request of NT${a} {re}.\nNote: {note}\nMy wallet: {wallet}\n"),
                    format!("Yêu cầu hoàn tiền mặt NT${a} của bạn {rv}.\nGhi chú: {note}\nVí của tôi: {wallet}\n"),
                    format!("Permintaan pengembalian dana tunai NT${a} kamu {ri}.\nCatatan: {note}\nDompetku: {wallet}\n"),
                ),
            )
        }
        "wallet.frozen" => (
            l.pick("【Wishly】你的錢包已凍結".into(), "[Wishly] Your wallet was frozen".into(), "[Wishly] Ví của bạn đã bị đóng băng".into(), "[Wishly] Dompetmu dibekukan".into()),
            l.pick(
                format!("你的點數錢包已被凍結，暫時無法使用點數。如有疑問請依服務條款聯絡客服。\n服務條款與客服：{base}/terms\n"),
                format!("Your points wallet has been frozen and points can't be used for now. If you have questions, contact support as described in the terms.\nTerms and support: {base}/terms\n"),
                format!("Ví điểm của bạn đã bị đóng băng, tạm thời không dùng được điểm. Nếu có thắc mắc, vui lòng liên hệ hỗ trợ theo điều khoản.\nĐiều khoản và hỗ trợ: {base}/terms\n"),
                format!("Dompet poinmu dibekukan dan poin tidak bisa dipakai untuk sementara. Kalau ada pertanyaan, hubungi dukungan sesuai ketentuan layanan.\nKetentuan dan dukungan: {base}/terms\n"),
            ),
        ),
        "wallet.unfrozen" => (
            l.pick("【Wishly】你的錢包已解除凍結".into(), "[Wishly] Your wallet was unfrozen".into(), "[Wishly] Ví của bạn đã được mở khóa".into(), "[Wishly] Dompetmu dibuka kembali".into()),
            l.pick(
                format!("你的點數錢包已解除凍結，可以正常使用。\n我的錢包：{wallet}\n"),
                format!("Your points wallet has been unfrozen and works normally again.\nMy wallet: {wallet}\n"),
                format!("Ví điểm của bạn đã được mở khóa và dùng bình thường trở lại.\nVí của tôi: {wallet}\n"),
                format!("Dompet poinmu sudah dibuka kembali dan bisa dipakai seperti biasa.\nDompetku: {wallet}\n"),
            ),
        ),
        "wallet.chargeback" => {
            let (a, d) = (payload["amount"].as_i64().unwrap_or(0), payload["debt"].as_i64().unwrap_or(0));
            (
                l.pick(format!("【Wishly】銀行爭議扣回 {a} 點"), format!("[Wishly] {a} points were charged back"), format!("[Wishly] Đã thu hồi {a} điểm do tranh chấp ngân hàng"), format!("[Wishly] {a} poin ditarik karena sengketa bank")),
                l.pick(
                    format!("因銀行爭議，你的錢包被扣回 {a} 點；目前欠款 {d} 點。如有疑問請聯絡客服。\n服務條款與客服：{base}/terms\n"),
                    format!("Because of a bank dispute, {a} points were deducted from your wallet; your outstanding debt is {d} points. Contact support if you have questions.\nTerms and support: {base}/terms\n"),
                    format!("Do tranh chấp ngân hàng, ví của bạn bị trừ {a} điểm; hiện còn nợ {d} điểm. Nếu có thắc mắc, vui lòng liên hệ hỗ trợ.\nĐiều khoản và hỗ trợ: {base}/terms\n"),
                    format!("Karena sengketa bank, {a} poin ditarik dari dompetmu; utang saat ini {d} poin. Hubungi dukungan kalau ada pertanyaan.\nKetentuan dan dukungan: {base}/terms\n"),
                ),
            )
        }
        "wishlist.moderated" => {
            let hidden = payload["hidden"].as_bool().unwrap_or(true);
            let reason = payload["reason"].as_str().filter(|r| !r.is_empty()).unwrap_or("-");
            if hidden {
                (
                    l.pick(format!("【Wishly】清單《{title}》已被下架"), format!("[Wishly] Your list \"{title}\" was taken down"), format!("[Wishly] Danh sách \"{title}\" đã bị gỡ"), format!("[Wishly] Wishlist \"{title}\" diturunkan")),
                    l.pick(
                        format!("你的清單《{title}》已被下架。\n原因：{reason}\n如有疑問請聯絡客服：{base}/terms\n"),
                        format!("Your list \"{title}\" has been taken down.\nReason: {reason}\nIf you have questions, contact support: {base}/terms\n"),
                        format!("Danh sách \"{title}\" của bạn đã bị gỡ.\nLý do: {reason}\nNếu có thắc mắc, vui lòng liên hệ hỗ trợ: {base}/terms\n"),
                        format!("Wishlist \"{title}\" milikmu diturunkan.\nAlasan: {reason}\nKalau ada pertanyaan, hubungi dukungan: {base}/terms\n"),
                    ),
                )
            } else {
                (
                    l.pick(format!("【Wishly】清單《{title}》已恢復"), format!("[Wishly] Your list \"{title}\" was restored"), format!("[Wishly] Danh sách \"{title}\" đã được khôi phục"), format!("[Wishly] Wishlist \"{title}\" dipulihkan")),
                    l.pick(
                        format!("你的清單《{title}》已恢復上架。\n{dash}\n"),
                        format!("Your list \"{title}\" has been restored.\n{dash}\n"),
                        format!("Danh sách \"{title}\" của bạn đã được khôi phục.\n{dash}\n"),
                        format!("Wishlist \"{title}\" milikmu sudah dipulihkan.\n{dash}\n"),
                    ),
                )
            }
        }
        "account.suspended" => (
            l.pick("【Wishly】你的帳號已被停權".into(), "[Wishly] Your account was suspended".into(), "[Wishly] Tài khoản của bạn đã bị đình chỉ".into(), "[Wishly] Akunmu ditangguhkan".into()),
            l.pick(
                format!("你的帳號已被停權，暫時無法使用。如有疑問請依服務條款聯絡客服。\n服務條款與客服：{base}/terms\n"),
                format!("Your account has been suspended and can't be used for now. If you have questions, contact support as described in the terms.\nTerms and support: {base}/terms\n"),
                format!("Tài khoản của bạn đã bị đình chỉ và tạm thời không dùng được. Nếu có thắc mắc, vui lòng liên hệ hỗ trợ theo điều khoản.\nĐiều khoản và hỗ trợ: {base}/terms\n"),
                format!("Akunmu ditangguhkan dan tidak bisa dipakai untuk sementara. Kalau ada pertanyaan, hubungi dukungan sesuai ketentuan layanan.\nKetentuan dan dukungan: {base}/terms\n"),
            ),
        ),
        "account.restored" => (
            l.pick("【Wishly】你的帳號已恢復".into(), "[Wishly] Your account was restored".into(), "[Wishly] Tài khoản của bạn đã được khôi phục".into(), "[Wishly] Akunmu dipulihkan".into()),
            l.pick(
                format!("你的帳號已恢復，可以正常使用。\n{dash}\n"),
                format!("Your account has been restored and works normally again.\n{dash}\n"),
                format!("Tài khoản của bạn đã được khôi phục và dùng bình thường trở lại.\n{dash}\n"),
                format!("Akunmu sudah dipulihkan dan bisa dipakai seperti biasa.\n{dash}\n"),
            ),
        ),
        "org.permit_reviewed" => {
            let name = payload["display_name"].as_str().unwrap_or("");
            let no = payload["permit_no"].as_str().unwrap_or("");
            let note = payload["note"].as_str().filter(|n| !n.is_empty()).unwrap_or("-");
            let url = format!("{base}/orgs/{}#permits", payload["org_id"].as_str().unwrap_or(""));
            let ok = payload["approved"].as_bool().unwrap_or(false);
            let (z, e, v, i) = if ok {
                ("已核准", "approved", "đã được duyệt", "disetujui")
            } else {
                ("未通過審核", "not approved", "không được duyệt", "tidak disetujui")
            };
            (
                l.pick(format!("【Wishly】勸募許可 {no} {z}"), format!("[Wishly] Fundraising permit {no} {e}"), format!("[Wishly] Giấy phép gây quỹ {no} {v}"), format!("[Wishly] Izin penggalangan dana {no} {i}")),
                l.pick(
                    format!("組織「{name}」的勸募許可 {no} {z}。\n審核備註：{note}\n勸募許可區：{url}\n"),
                    format!("The fundraising permit {no} of \"{name}\" was {e}.\nReview note: {note}\nPermits section: {url}\n"),
                    format!("Giấy phép gây quỹ {no} của \"{name}\" {v}.\nGhi chú xét duyệt: {note}\nMục giấy phép: {url}\n"),
                    format!("Izin penggalangan dana {no} milik \"{name}\" {i}.\nCatatan peninjauan: {note}\nBagian izin: {url}\n"),
                ),
            )
        }
        "org.event_hidden" => {
            let name = payload["display_name"].as_str().unwrap_or("");
            let ev = payload["event_title"].as_str().unwrap_or("");
            let reason = payload["reason"].as_str().filter(|r| !r.is_empty()).unwrap_or("-");
            let url = format!("{base}/orgs/{}", payload["org_id"].as_str().unwrap_or(""));
            (
                l.pick(format!("【Wishly】事件「{ev}」已下架"), format!("[Wishly] Event \"{ev}\" was taken down"), format!("[Wishly] Sự kiện \"{ev}\" đã bị gỡ"), format!("[Wishly] Acara \"{ev}\" diturunkan")),
                l.pick(
                    format!("組織「{name}」的事件「{ev}」已下架，所屬救災據點已暫停。\n原因：{reason}\n組織後台：{url}\n"),
                    format!("The event \"{ev}\" of \"{name}\" has been taken down, and its relief sites are paused.\nReason: {reason}\nOrganization dashboard: {url}\n"),
                    format!("Sự kiện \"{ev}\" của \"{name}\" đã bị gỡ, các điểm cứu trợ thuộc sự kiện đã tạm dừng.\nLý do: {reason}\nTrang quản trị tổ chức: {url}\n"),
                    format!("Acara \"{ev}\" milik \"{name}\" diturunkan, dan lokasi bantuan di bawahnya dijeda.\nAlasan: {reason}\nDasbor organisasi: {url}\n"),
                ),
            )
        }
        "auth.mfa_enabled" => (
            l.pick("【Wishly】兩步驟驗證已啟用".into(), "[Wishly] Two-step verification enabled".into(), "[Wishly] Đã bật xác minh hai bước".into(), "[Wishly] Verifikasi dua langkah diaktifkan".into()),
            l.pick(
                format!("你的帳號剛剛啟用了兩步驟驗證。\n若非本人操作，請立即檢查帳號設定。\n帳號設定：{base}/settings\n這是帳號安全通知。"),
                format!("Two-step verification was just enabled on your account.\nIf this wasn't you, check your account settings right away.\nAccount settings: {base}/settings\nThis is an account security notice."),
                format!("Xác minh hai bước vừa được bật cho tài khoản của bạn.\nNếu không phải bạn thực hiện, hãy kiểm tra cài đặt tài khoản ngay.\nCài đặt tài khoản: {base}/settings\nĐây là thông báo bảo mật tài khoản."),
                format!("Verifikasi dua langkah baru saja diaktifkan di akunmu.\nKalau bukan kamu yang melakukannya, segera periksa pengaturan akun.\nPengaturan akun: {base}/settings\nIni adalah pemberitahuan keamanan akun."),
            ),
        ),
        "auth.recovery_regenerated" => (
            l.pick("【Wishly】復原碼已重新產生".into(), "[Wishly] Recovery codes regenerated".into(), "[Wishly] Mã khôi phục đã được tạo lại".into(), "[Wishly] Kode pemulihan dibuat ulang".into()),
            l.pick(
                format!("你的帳號剛剛重新產生了兩步驟驗證復原碼，舊的復原碼已失效。\n若非本人操作，請立即檢查帳號設定。\n帳號設定：{base}/settings\n這是帳號安全通知。"),
                format!("The two-step verification recovery codes on your account were just regenerated, and the old codes no longer work.\nIf this wasn't you, check your account settings right away.\nAccount settings: {base}/settings\nThis is an account security notice."),
                format!("Mã khôi phục xác minh hai bước của tài khoản bạn vừa được tạo lại, các mã cũ không còn dùng được.\nNếu không phải bạn thực hiện, hãy kiểm tra cài đặt tài khoản ngay.\nCài đặt tài khoản: {base}/settings\nĐây là thông báo bảo mật tài khoản."),
                format!("Kode pemulihan verifikasi dua langkah di akunmu baru saja dibuat ulang, dan kode lama tidak berlaku lagi.\nKalau bukan kamu yang melakukannya, segera periksa pengaturan akun.\nPengaturan akun: {base}/settings\nIni adalah pemberitahuan keamanan akun."),
            ),
        ),
        "staff.digest" => {
            let g = |k: &str| payload[k].as_i64().unwrap_or(0);
            let (r, f, o, p, d, x) = (g("reports"), g("refunds"), g("orgs"), g("permits"), g("orders"), g("failed"));
            (
                l.pick("【Wishly】今日營運待辦".into(), "[Wishly] Today's operations to-do".into(), "[Wishly] Việc vận hành cần xử lý hôm nay".into(), "[Wishly] Tugas operasional hari ini".into()),
                l.pick(
                    format!("待處理檢舉 {r}、現金退款 {f}、待審組織 {o}、待審許可 {p}、待處理訂單 {d}、寄送失敗通知 {x}。\n後台：{base}/admin\n"),
                    format!("Open reports {r}, cash refunds {f}, organizations to review {o}, permits to review {p}, orders to handle {d}, failed notifications {x}.\nConsole: {base}/admin\n"),
                    format!("Báo cáo chờ xử lý {r}, hoàn tiền mặt {f}, tổ chức chờ duyệt {o}, giấy phép chờ duyệt {p}, đơn cần xử lý {d}, thông báo gửi lỗi {x}.\nTrang quản trị: {base}/admin\n"),
                    format!("Laporan menunggu {r}, pengembalian dana tunai {f}, organisasi perlu ditinjau {o}, izin perlu ditinjau {p}, pesanan perlu diurus {d}, notifikasi gagal terkirim {x}.\nKonsol: {base}/admin\n"),
                ),
            )
        }
        "claim.item_removed" => (
            l.pick("【Wishly】你要送的品項已被移除".into(), "[Wishly] An item you were giving was removed".into(), "[Wishly] Món quà bạn định tặng đã bị gỡ".into(), "[Wishly] Barang yang mau kamu kasih dihapus".into()),
            l.pick(
                format!("清單《{title}》的建立者移除了你要送的品項，這筆認領已取消。\n查看其他品項：{manage}\n"),
                format!("The owner of list \"{title}\" removed an item you were going to give, so your claim has been cancelled.\nSee other items: {manage}\n"),
                format!("Chủ danh sách \"{title}\" đã gỡ món quà bạn định tặng, lượt nhận tặng này đã bị hủy.\nXem các món khác: {manage}\n"),
                format!("Pemilik wishlist \"{title}\" menghapus barang yang mau kamu kasih, jadi klaim ini dibatalkan.\nLihat barang lainnya: {manage}\n"),
            ),
        ),
        "friend.request" => {
            let from = payload["from_name"].as_str().unwrap_or_default();
            (
                l.pick(format!("【Wishly】{from} 想加你為好友"), format!("[Wishly] {from} wants to be your friend"), format!("[Wishly] {from} muốn kết bạn với bạn"), format!("[Wishly] {from} ingin berteman denganmu")),
                l.pick(
                    format!("{from} 向你送出了好友申請。\n接受或婉拒：{base}/friends\n"),
                    format!("{from} sent you a friend request.\nAccept or decline: {base}/friends\n"),
                    format!("{from} đã gửi lời mời kết bạn cho bạn.\nChấp nhận hoặc từ chối: {base}/friends\n"),
                format!("{from} mengirim permintaan pertemanan kepadamu.\nTerima atau tolak: {base}/friends\n"),
            ),
            )
        }
        "claim.item_changed" => {
            // 眾籌認捐在錢包管理，一般捐助在「我的捐助」
            let link = if payload["crowdfund"] == true { &wallet } else { &manage };
            (
                l.pick(format!("【Wishly】你要送的《{item}》內容有更新"), format!("[Wishly] \"{item}\" you're giving was updated"), format!("[Wishly] \"{item}\" bạn định tặng đã được cập nhật"), format!("[Wishly] \"{item}\" yang mau kamu kasih sudah diperbarui")),
                l.pick(
                    format!("清單《{title}》的建立者修改了「{item}」的內容（例如數量、規格或說明）。購買前請再確認一次數量與規格。\n管理我的送禮：{link}\n"),
                    format!("The owner of list \"{title}\" updated \"{item}\" (for example its quantity, spec or description). Please re-check the quantity and spec before buying.\nManage my gifts: {link}\n"),
                    format!("Chủ danh sách \"{title}\" đã cập nhật \"{item}\" (ví dụ số lượng, quy cách hoặc mô tả). Vui lòng kiểm tra lại số lượng và quy cách trước khi mua.\nQuản lý quà tôi tặng: {link}\n"),
                format!("Pemilik wishlist \"{title}\" mengubah \"{item}\" (misalnya jumlah, spesifikasi, atau deskripsi). Mohon cek lagi jumlah dan spesifikasinya sebelum membeli.\nKelola hadiah dariku: {link}\n"),
            ),
            )
        }
        "wishlist.closing" => {
            // 以台北時間顯示；解析失敗就只講「24 小時內」
            let at = taipei(payload, "closes_at");
            let (at_zh, at_en, at_vi, at_id) = match at {
                Some(a) => (format!("{a}（台北時間）"), format!("{a} (Taipei time)"), format!("{a} (giờ Đài Bắc)"), format!("{a} (waktu Taipei)")),
                None => ("24 小時內".into(), "within 24 hours".into(), "trong vòng 24 giờ".into(), "dalam 24 jam".into()),
            };
            (
                l.pick(format!("【Wishly】《{title}》即將結束"), format!("[Wishly] \"{title}\" is about to close"), format!("[Wishly] \"{title}\" sắp kết thúc"), format!("[Wishly] \"{title}\" akan segera ditutup")),
                l.pick(
                    format!("你的清單《{title}》將於 {at_zh}自動結束，屆時朋友不能再認領。\n要延長或取消期限：{base}/dashboard\n"),
                    format!("Your list \"{title}\" will close automatically {at_en}, after which friends can no longer claim.\nTo extend or cancel the deadline: {base}/dashboard\n"),
                    format!("Danh sách \"{title}\" của bạn sẽ tự động kết thúc vào {at_vi}, sau đó bạn bè không thể nhận tặng nữa.\nĐể gia hạn hoặc hủy thời hạn: {base}/dashboard\n"),
                format!("Wishlist \"{title}\" milikmu akan ditutup otomatis pada {at_id}, setelah itu teman tidak bisa mengklaim lagi.\nUntuk memperpanjang atau membatalkan batas waktu: {base}/dashboard\n"),
            ),
            )
        }
        "claim.purchase_reminder" => {
            let at = taipei(payload, "expires_at").unwrap_or_default();
            (
                l.pick(format!("【Wishly】還沒買《{item}》嗎？"), format!("[Wishly] Haven't bought \"{item}\" yet?"), format!("[Wishly] Bạn chưa mua \"{item}\"?"), format!("[Wishly] Belum beli \"{item}\"?")),
                if payload["ended"] == true {
                    l.pick(
                        format!("清單《{title}》已結束。如果你已經買好或送出了「{item}」，請在 {at}（台北時間）前標記已購買或已送達；逾期未標記的認領會釋出。\n管理我的送禮：{manage}\n"),
                        format!("The list \"{title}\" has ended. If you've already bought or given \"{item}\", please mark it as purchased or delivered before {at} (Taipei time); unmarked claims will be released after that.\nManage my gifts: {manage}\n"),
                        format!("Danh sách \"{title}\" đã kết thúc. Nếu bạn đã mua hoặc trao \"{item}\", vui lòng đánh dấu đã mua hoặc đã trao trước {at} (giờ Đài Bắc); các lượt nhận tặng chưa đánh dấu sẽ được nhường lại sau thời điểm đó.\nQuản lý quà tôi tặng: {manage}\n"),
                format!("Wishlist \"{title}\" sudah berakhir. Kalau kamu sudah membeli atau memberikan \"{item}\", mohon tandai sebagai sudah dibeli atau sudah sampai sebelum {at} (waktu Taipei); klaim yang belum ditandai akan dilepas setelah itu.\nKelola hadiah dariku: {manage}\n"),
            )
                } else {
                l.pick(
                    format!("還沒買嗎？你在《{title}》認領的「{item}」請在 {at}（台北時間）前標記已購買，或延長 7 天；逾期會自動釋出給其他朋友。\n管理我的送禮：{manage}\n"),
                    format!("Haven't bought it yet? Please mark \"{item}\" from \"{title}\" as purchased before {at} (Taipei time), or extend by 7 days. Otherwise it will be released to other friends.\nManage my gifts: {manage}\n"),
                    format!("Bạn chưa mua? Vui lòng đánh dấu đã mua \"{item}\" trong \"{title}\" trước {at} (giờ Đài Bắc), hoặc gia hạn thêm 7 ngày; quá hạn sẽ tự động nhường cho bạn bè khác.\nQuản lý quà tôi tặng: {manage}\n"),
                format!("Belum beli? Mohon tandai \"{item}\" dari \"{title}\" sebagai sudah dibeli sebelum {at} (waktu Taipei), atau perpanjang 7 hari. Kalau tidak, barang ini akan dilepas untuk teman lain.\nKelola hadiah dariku: {manage}\n"),
            )
                },
            )
        }
        "claim.list_closed" => {
            let at = taipei(payload, "expires_at").unwrap_or_default();
            (
                l.pick(format!("【Wishly】《{title}》已結束，請標記你要送的禮物"), format!("[Wishly] \"{title}\" has ended: please mark the gifts you're giving"), format!("[Wishly] \"{title}\" đã kết thúc: vui lòng đánh dấu quà bạn sẽ tặng"), format!("[Wishly] \"{title}\" sudah berakhir: mohon tandai hadiah yang mau kamu kasih")),
                l.pick(
                    format!("清單《{title}》已結束。如果你已經買好或送出了「{item}」，請到「我的送禮」標記已購買或已送達；{at}（台北時間）前沒有標記的認領會釋出。\n管理我的送禮：{manage}\n"),
                    format!("The list \"{title}\" has ended. If you've already bought or given \"{item}\", please mark it as purchased or delivered in My gifts; claims not marked before {at} (Taipei time) will be released.\nManage my gifts: {manage}\n"),
                    format!("Danh sách \"{title}\" đã kết thúc. Nếu bạn đã mua hoặc trao \"{item}\", vui lòng đánh dấu đã mua hoặc đã giao trong Quà tôi tặng; các món chưa đánh dấu trước {at} (giờ Đài Bắc) sẽ được nhường lại.\nQuản lý quà tôi tặng: {manage}\n"),
                format!("Wishlist \"{title}\" sudah berakhir. Kalau kamu sudah membeli atau memberikan \"{item}\", mohon tandai sebagai sudah dibeli atau sudah sampai di Hadiah dariku; klaim yang belum ditandai sebelum {at} (waktu Taipei) akan dilepas.\nKelola hadiah dariku: {manage}\n"),
            ),
            )
        }
        "claim.expired" if payload["ended"] == true => (
            l.pick(format!("【Wishly】「{item}」已超過購買期限"), format!("[Wishly] \"{item}\" passed its purchase deadline"), format!("[Wishly] \"{item}\" đã quá hạn mua"), format!("[Wishly] \"{item}\" melewati batas waktu pembelian")),
            l.pick(
                format!("清單《{title}》已結束，你認領的「{item}」在期限前沒有標記已購買，這筆認領已取消。\n管理我的送禮：{manage}\n"),
                format!("The list \"{title}\" has ended and \"{item}\" wasn't marked as purchased before the deadline, so your claim has been cancelled.\nManage my gifts: {manage}\n"),
                format!("Danh sách \"{title}\" đã kết thúc và \"{item}\" chưa được đánh dấu đã mua trước hạn, nên lượt nhận của bạn đã bị hủy.\nQuản lý quà tôi tặng: {manage}\n"),
                format!("Wishlist \"{title}\" sudah berakhir dan \"{item}\" tidak ditandai sebagai sudah dibeli sebelum batas waktu, jadi klaimmu dibatalkan.\nKelola hadiah dariku: {manage}\n"),
            ),
        ),
        "claim.expired" => (
            l.pick(format!("【Wishly】「{item}」已超過購買期限"), format!("[Wishly] \"{item}\" passed its purchase deadline"), format!("[Wishly] \"{item}\" đã quá hạn mua"), format!("[Wishly] \"{item}\" melewati batas waktu pembelian")),
            l.pick(
                format!("你在《{title}》認領的「{item}」已超過購買期限，已釋出給其他朋友。\n管理我的送禮：{manage}\n"),
                format!("The purchase deadline for \"{item}\" you claimed on \"{title}\" has passed, so it has been released to other friends.\nManage my gifts: {manage}\n"),
                format!("\"{item}\" bạn nhận trong \"{title}\" đã quá hạn mua và đã được nhường cho bạn bè khác.\nQuản lý quà tôi tặng: {manage}\n"),
                format!("Batas waktu pembelian \"{item}\" yang kamu klaim di \"{title}\" sudah lewat, jadi barang ini dilepas untuk teman lain.\nKelola hadiah dariku: {manage}\n"),
            ),
        ),
        // 寄給清單擁有者：不提捐助者是誰（驚喜模式也安全）
        "item.reopened" => (
            l.pick(format!("【Wishly】《{title}》有品項重新開放認領"), format!("[Wishly] An item on \"{title}\" is open again"), format!("[Wishly] Có món trong \"{title}\" được mở lại"), format!("[Wishly] Ada barang di \"{title}\" yang dibuka lagi")),
            l.pick(
                format!("《{title}》有品項重新開放認領。\n{base}/dashboard\n"),
                format!("An item on \"{title}\" is open for claiming again.\n{base}/dashboard\n"),
                format!("Có món trong \"{title}\" được mở lại để nhận.\n{base}/dashboard\n"),
                format!("Ada barang di \"{title}\" yang dibuka lagi untuk diklaim.\n{base}/dashboard\n"),
            ),
        ),
        "birthday.friend" => {
            // 驚喜模式：只提生日與清單，不含任何認捐 / 捐助資訊
            let who = payload["owner_name"].as_str().unwrap_or("");
            let slug = payload["slug"].as_str().unwrap_or("");
            let (m, d) = payload["birthday"].as_str().and_then(|b| b.split_once('-')).map(|(m, d)| (m.trim_start_matches('0'), d.trim_start_matches('0'))).unwrap_or(("", ""));
            (
                l.pick(format!("【Wishly】{who} 的生日快到了"), format!("[Wishly] {who}'s birthday is coming up"), format!("[Wishly] Sắp đến sinh nhật của {who}"), format!("[Wishly] Ulang tahun {who} sudah dekat")),
                l.pick(
                    format!("{who} 的生日是 {m} 月 {d} 日，快到了！\n看看生日清單《{title}》：{base}/s/{slug}\n"),
                    format!("{who}'s birthday is on {m}/{d} and it's coming up!\nTake a look at their birthday list \"{title}\": {base}/s/{slug}\n"),
                    format!("Sinh nhật của {who} vào ngày {d}/{m}, sắp đến rồi!\nXem danh sách sinh nhật \"{title}\" của họ: {base}/s/{slug}\n"),
                    format!("Ulang tahun {who} jatuh pada {d}/{m}, sebentar lagi!\nLihat wishlist ulang tahunnya \"{title}\": {base}/s/{slug}\n"),
                ),
            )
        }
        "birthday.nudge" => (
            l.pick("【Wishly】你的生日快到了".into(), "[Wishly] Your birthday is coming up".into(), "[Wishly] Sắp đến sinh nhật của bạn".into(), "[Wishly] Ulang tahunmu sudah dekat".into()),
            l.pick(
                format!("你的生日快到了，開一份生日清單並通知朋友吧。\n{base}/lists/new\n"),
                format!("Your birthday is coming up. Create a birthday list and let your friends know.\n{base}/lists/new\n"),
                format!("Sinh nhật của bạn sắp đến. Hãy tạo danh sách sinh nhật và báo cho bạn bè nhé.\n{base}/lists/new\n"),
                format!("Ulang tahunmu sudah dekat. Buat wishlist ulang tahun dan beri tahu teman-temanmu.\n{base}/lists/new\n"),
            ),
        ),
        _ => (l.pick("【Wishly】通知".into(), "[Wishly] Notification".into(), "[Wishly] Thông báo".into(), "[Wishly] Pemberitahuan".into()), format!("{base}\n")),
    };
    // 合併通知（worker 步驟 5）：主旨加件數，內文列出其餘品項
    let (subject, body) = match payload.get("more_items").and_then(Value::as_array).filter(|m| !m.is_empty()) {
        Some(more) => {
            let n = more.len() + 1;
            let suffix = l.pick(format!("（共 {n} 件）"), format!(" ({n} items)"), format!(" ({n} món)"), format!(" ({n} barang)"));
            let head = l.pick("其他品項：".into(), "Other items:".into(), "Các món khác:".into(), "Barang lainnya:".into());
            let list: String = more.iter().filter_map(Value::as_str).map(|t| format!("- {t}\n")).collect();
            (format!("{subject}{suffix}"), format!("{body}\n{head}\n{list}"))
        }
        None => (subject, body),
    };
    match payload.get("unsubscribe_url").and_then(Value::as_str) {
        Some(u) => {
            let foot = l.pick(
                format!("退訂此類通知：{u}"),
                format!("Unsubscribe from these notifications: {u}"),
                format!("Hủy nhận loại thông báo này: {u}"),
                format!("Berhenti menerima pemberitahuan semacam ini: {u}"),
            );
            (subject, format!("{body}\n{foot}\n"))
        }
        None => (subject, body),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    #[test]
    fn quiet_hours_taipei() {
        // 台北 21:00 = UTC 13:00；09:00 = UTC 01:00
        let q = |h, m| quiet_until(Utc.with_ymd_and_hms(2026, 10, 31, h, m, 0).unwrap());
        assert_eq!(q(12, 59), None); // 台北 20:59
        assert_eq!(q(13, 0), Some(Utc.with_ymd_and_hms(2026, 11, 1, 1, 0, 0).unwrap()));
        assert_eq!(q(20, 0), Some(Utc.with_ymd_and_hms(2026, 11, 1, 1, 0, 0).unwrap())); // 台北 04:00 前一天夜裡也順延到隔天早上
        assert_eq!(q(0, 30), Some(Utc.with_ymd_and_hms(2026, 10, 31, 1, 0, 0).unwrap())); // 台北 08:30
        assert_eq!(q(1, 0), None); // 台北 09:00
    }

    #[test]
    fn backoff_steps() {
        let s: Vec<u64> = (1..=5).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(s, [60, 300, 1800, 7200, 21600]);
        assert_eq!(MAX_ATTEMPTS, 6);
    }

    #[test]
    fn every_kind_has_copy_in_all_languages() {
        let p = json!({"givers": 2, "amount": 100, "debt": 5, "status": "completed", "from_name": "A", "hidden": true});
        let fallback = |l| render_lang(l, "no.such.kind", "", &p).0;
        for kind in KINDS {
            for l in [Lang::Zh, Lang::En, Lang::Vi, Lang::Id] {
                let (s, b) = render_lang(l, kind, "T", &p);
                assert_ne!(s, fallback(l), "{kind} {l:?} 落到預設主旨");
                assert!(!b.trim().is_empty(), "{kind} {l:?}");
            }
            assert!(inbox_link(kind, &p).starts_with('/'), "{kind}");
        }
        assert!(!KINDS.contains(&"event.reminder"));
    }

    #[test]
    fn categories() {
        use Category::*;
        for k in ["auth.mfa_reset", "org.review", "staff.digest", "account.suspended", "wallet.frozen", "topup.credited",
                  "cash_refund.updated", "wishlist.moderated", "order.failed", "order.partial_refund", "funding.expired",
                  "funding.released", "item.removed"] {
            assert_eq!(category(k), Account, "{k}");
        }
        for k in ["friend.request", "friend.accepted", "birthday.friend", "birthday.nudge"] {
            assert_eq!(category(k), Social, "{k}");
        }
        for k in ["claim.confirmation", "owner.funded", "event.summary", "site.paused", "wishlist.closing", "crowdfund.funded", "item.reopened"] {
            assert_eq!(category(k), Gifts, "{k}");
        }
        for k in KINDS {
            category(k); // 全部可分類
        }
    }
}
