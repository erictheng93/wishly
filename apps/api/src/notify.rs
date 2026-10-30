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

const MAX_ATTEMPTS: i16 = 3;

/// 處理一批到期通知；回傳處理筆數。領取時 attempts+1 並把 scheduled_at 推後 1 分鐘當租約（失敗即自動重試）。
pub async fn tick(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let rows: Vec<PendingNotification> = sqlx::query_as(
        "UPDATE notifications SET attempts = attempts + 1, scheduled_at = now() + interval '1 minute'
          WHERE id IN (SELECT id FROM notifications WHERE status = 'pending' AND scheduled_at <= now()
                        ORDER BY scheduled_at LIMIT 20 FOR UPDATE SKIP LOCKED)
         RETURNING id, user_id, guest_id, channel::text, kind, payload, attempts")
        .fetch_all(pool).await?;
    if rows.is_empty() {
        return Ok(0);
    }
    let cfg = crate::config::get();
    let n = rows.len();
    for (id, uid, gid, channel, kind, mut payload, attempts) in rows {
        if kind == "claim.purchase_reminder" {
            // 寄送當下：認領仍待購且期限沒變才寄；清單已結束（Q14）時改用不提「延長」的文案
            let ended: Option<bool> = sqlx::query_scalar(
                "SELECT w.status IN ('closed', 'archived') OR (w.closes_at IS NOT NULL AND w.closes_at <= now())
                   FROM claims c JOIN wishlist_items i ON i.id = c.item_id JOIN wishlists w ON w.id = i.wishlist_id
                  WHERE c.id = ($1->>'claim_id')::uuid AND c.status = 'reserved' AND c.expires_at = ($1->>'expires_at')::timestamptz")
                .bind(&payload).fetch_optional(pool).await?;
            let Some(ended) = ended else {
                sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'claim no longer due' WHERE id = $1").bind(id).execute(pool).await?;
                continue;
            };
            if ended {
                payload["ended"] = Value::Bool(true);
            }
        } else if kind == "friend.request" {
            // 申請可能已被撤回 / 接受 / 拒絕，或寄件人已刪除帳號：不寄
            let from_name: Option<String> = sqlx::query_scalar(
                "SELECT o.display_name FROM friend_requests r JOIN users o ON o.id = r.from_user AND o.deleted_at IS NULL
                  WHERE r.id = ($1->>'request_id')::uuid")
                .bind(&payload).fetch_optional(pool).await?;
            let Some(from_name) = from_name else {
                sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'friend request no longer pending' WHERE id = $1").bind(id).execute(pool).await?;
                continue;
            };
            payload["from_name"] = Value::String(from_name);
        } else if kind == "birthday.friend" {
            // 寄前重驗（D65）：清單仍開著生日通知且仍看得到、壽星生日仍公開給好友、收件人仍是好友（selected 仍在名單）且沒關通知
            let row: Option<(String, String)> = sqlx::query_as(
                "SELECT o.display_name, w.slug::text FROM wishlists w
                   JOIN users o ON o.id = w.owner_id AND o.deleted_at IS NULL AND o.blocked_at IS NULL
                                AND o.birthday IS NOT NULL AND o.birthday_visibility::text <> 'private'
                   JOIN users r ON r.id = $2 AND r.deleted_at IS NULL AND r.blocked_at IS NULL
                                AND COALESCE(r.notification_prefs->>'email_birthdays', 'true') <> 'false'
                  WHERE w.id = ($1->>'wishlist_id')::uuid AND w.owner_id = ($1->>'owner_id')::uuid
                    AND w.birthday_notify AND w.deleted_at IS NULL AND w.status = 'active' AND w.moderation_status = 'ok'
                    AND w.visibility::text <> 'private'
                    AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST(r.id, o.id) AND f.user_b = GREATEST(r.id, o.id))
                    AND (w.visibility::text <> 'selected' OR EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = w.id AND a.user_id = r.id))")
                .bind(&payload).bind(uid).fetch_optional(pool).await?;
            let Some((owner_name, slug)) = row else {
                sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'birthday list no longer visible' WHERE id = $1").bind(id).execute(pool).await?;
                continue;
            };
            payload["owner_name"] = owner_name.into();
            payload["slug"] = slug.into();
        } else if kind == "birthday.nudge" {
            let ok: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND deleted_at IS NULL AND birthday IS NOT NULL AND birthday_visibility::text <> 'private'
                                  AND COALESCE(notification_prefs->>'email_birthdays', 'true') <> 'false')")
                .bind(uid).fetch_one(pool).await?;
            if !ok {
                sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'birthday nudge no longer wanted' WHERE id = $1").bind(id).execute(pool).await?;
                continue;
            }
        } else if kind == "claim.item_changed" {
            // 延後 10 分鐘寄：這段期間取消 / 撤回 / 轉投 / 品項被刪的就不寄
            let still: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM claims WHERE item_id = ($1->>'item_id')::uuid AND status IN ('reserved', 'purchased')
                                  AND (user_id = $2 OR guest_id = $3))
                     OR EXISTS (SELECT 1 FROM contributions WHERE item_id = ($1->>'item_id')::uuid AND status = 'pledged' AND user_id = $2)")
                .bind(&payload).bind(uid).bind(gid).fetch_one(pool).await?;
            if !still {
                sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'no longer supporting item' WHERE id = $1").bind(id).execute(pool).await?;
                continue;
            }
        }
        let security = matches!(
            kind.as_str(),
            "auth.identity_linked"
                | "auth.identity_unlinked"
                | "auth.password_changed"
                | "auth.mfa_reset"
        );
        let to: Option<String> = if channel != "email" {
            None
        } else if let Some(u) = uid {
            sqlx::query_scalar("SELECT CASE WHEN $2 THEN $3 ELSE email END FROM users WHERE id = $1 AND deleted_at IS NULL")
                .bind(u).bind(security).bind(payload.get("to").and_then(Value::as_str)).fetch_optional(pool).await?.flatten()
        } else if let Some(g) = gid {
            sqlx::query_scalar("SELECT email FROM guests WHERE id = $1 AND deleted_at IS NULL")
                .bind(g)
                .fetch_optional(pool)
                .await?
                .flatten()
        } else {
            None
        };
        let Some(to) = to else {
            sqlx::query("UPDATE notifications SET status = 'cancelled', last_error = 'no email recipient' WHERE id = $1").bind(id).execute(pool).await?;
            continue;
        };
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
        // 寄信時才產生：退訂連結；訪客確認信的一次性恢復權杖（每次寄信換發，30 天）
        let mut payload = payload;
        // 眾籌通知：寄信時才查品項名稱（品項可能已被刪除，仍保留列）
        if let Some(it) = payload
            .get("item_id")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Uuid>().ok())
        {
            let t: Option<String> =
                sqlx::query_scalar("SELECT title FROM wishlist_items WHERE id = $1")
                    .bind(it)
                    .fetch_optional(pool)
                    .await?;
            if let Some(t) = t {
                payload["item_title"] = t.into();
            }
        }
        let api = cfg.api_base_url.clone();
        // 只有訪客信附退訂。會員信都是點數、訂單、自己清單的服務通知；唯一可退訂的「有新捐助」已移除（D58），附了也停不掉
        if let (None, Some(g)) = (uid, gid) {
            payload["unsubscribe_url"] = format!(
                "{}/api/v1/unsubscribe?token={}",
                api.trim_end_matches('/'),
                crate::account::unsub_token('g', g)
            )
            .into();
        }
        if let (Some(u), "birthday.friend" | "birthday.nudge") = (uid, kind.as_str()) {
            payload["unsubscribe_url"] = format!(
                "{}/api/v1/unsubscribe?token={}",
                api.trim_end_matches('/'),
                crate::account::unsub_token('b', u)
            )
            .into();
        }
        if let (Some(g), "claim.confirmation") = (gid, kind.as_str()) {
            let (tok, h) = crate::guest::new_token();
            sqlx::query("INSERT INTO guest_recovery_tokens (guest_id, token_hash, expires_at) VALUES ($1, $2, now() + interval '30 days')")
                .bind(g).bind(h).execute(pool).await?;
            payload["recovery_token"] = tok.into();
        }
        let lang = match uid {
            Some(u) => Lang::parse(
                &sqlx::query_scalar::<_, String>("SELECT locale FROM users WHERE id = $1")
                    .bind(u)
                    .fetch_optional(pool)
                    .await?
                    .unwrap_or_default(),
            ),
            None => Lang::parse(
                payload
                    .get("locale")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
        };
        let (subject, body) = render_lang(lang, &kind, &title, &payload);
        if to.parse::<lettre::Address>().is_err() {
            sqlx::query("UPDATE notifications SET status = 'failed', last_error = 'bad address' WHERE id = $1").bind(id).execute(pool).await?;
            continue;
        }
        let sent = send_mail(&cfg, &to, &subject, &body).await;
        match sent {
            Ok(()) => {
                sqlx::query("UPDATE notifications SET status = 'sent', sent_at = now(), last_error = NULL WHERE id = $1").bind(id).execute(pool).await?;
            }
            Err(e) => {
                let st = if attempts >= MAX_ATTEMPTS {
                    "failed"
                } else {
                    "pending"
                };
                sqlx::query("UPDATE notifications SET status = $2::notification_status, last_error = $3 WHERE id = $1").bind(id).bind(st).bind(e).execute(pool).await?;
            }
        }
    }
    Ok(n)
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
    let manage = match payload.get("recovery_token").and_then(Value::as_str) {
        Some(t) => format!("{base}/me/claims#r={t}"),
        None => format!("{base}/me/claims"),
    };
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
        "claim.confirmation" => (
            l.pick("【Wishly】這份禮物交給你了".into(), "[Wishly] This gift is yours to give".into(), "[Wishly] Món quà này giao cho bạn rồi".into(), "[Wishly] Hadiah ini kamu yang kasih".into()),
            l.pick(
                format!("已記下由你送這份禮物，謝謝你的心意。\n管理我的送禮：{manage}\n"),
                format!("It's recorded that you're giving this gift. Thank you for your kindness.\nManage my gifts: {manage}\n"),
                format!("Đã ghi nhận bạn sẽ tặng món quà này. Cảm ơn tấm lòng của bạn.\nQuản lý quà tôi tặng: {manage}\n"),
                format!("Sudah dicatat: kamu yang kasih hadiah ini. Terima kasih atas niat baikmu.\nKelola hadiah dariku: {manage}\n"),
            ),
        ),
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
        "event.reminder" => (
            l.pick(format!("【Wishly】《{title}》活動日快到了"), format!("[Wishly] The event day for \"{title}\" is coming up"), format!("[Wishly] Sắp đến ngày sự kiện của \"{title}\""), format!("[Wishly] Hari acara \"{title}\" sudah dekat")),
            l.pick(
                format!("你的清單《{title}》的活動日即將到來。\n{base}/dashboard\n"),
                format!("The event day for your list \"{title}\" is coming up.\n{base}/dashboard\n"),
                format!("Ngày sự kiện của danh sách \"{title}\" sắp đến.\n{base}/dashboard\n"),
                format!("Hari acara wishlist \"{title}\" milikmu sudah dekat.\n{base}/dashboard\n"),
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
