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
    for (id, uid, gid, channel, kind, payload, attempts) in rows {
        let security = matches!(
            kind.as_str(),
            "auth.identity_linked" | "auth.identity_unlinked" | "auth.password_changed"
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
}

impl Lang {
    pub fn parse(tag: &str) -> Lang {
        let t = tag.trim().to_ascii_lowercase();
        if t.starts_with("en") {
            Lang::En
        } else if t.starts_with("vi") {
            Lang::Vi
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
        }
    }
    fn pick(self, zh: String, en: String, vi: String) -> String {
        match self {
            Lang::Zh => zh,
            Lang::En => en,
            Lang::Vi => vi,
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

pub fn render_lang(lang: Lang, kind: &str, title: &str, payload: &Value) -> (String, String) {
    let l = lang;
    let app = crate::config::get().app_url;
    if kind == "auth.password_changed" {
        return (
            l.pick("【Wishly】密碼已變更".into(), "[Wishly] Your password was changed".into(), "[Wishly] Mật khẩu đã được thay đổi".into()),
            l.pick(
                format!("你的帳號 Email 登入密碼剛剛已變更，其他裝置已登出。\n若非本人操作，請立即使用「忘記密碼」重設。\n帳號設定：{app}/settings\n這是帳號安全通知。"),
                format!("The password for your account's email login was just changed, and your other devices have been signed out.\nIf this wasn't you, reset it right away with \"Forgot password\".\nAccount settings: {app}/settings\nThis is an account security notice."),
                format!("Mật khẩu đăng nhập bằng email của tài khoản bạn vừa được thay đổi, các thiết bị khác đã bị đăng xuất.\nNếu không phải bạn thực hiện, hãy đặt lại ngay bằng \"Quên mật khẩu\".\nCài đặt tài khoản: {app}/settings\nĐây là thông báo bảo mật tài khoản."),
            ),
        );
    }
    if matches!(kind, "auth.identity_linked" | "auth.identity_unlinked") {
        let linked = kind == "auth.identity_linked";
        let provider = match payload["provider"].as_str() {
            Some("google") => "Google".to_string(),
            Some("line") => "LINE".to_string(),
            Some("email") => "Email".to_string(),
            _ => l.pick("第三方".into(), "third-party".into(), "bên thứ ba".into()),
        };
        let action_zh = if linked { "新增" } else { "解除" };
        let action_en = if linked { "added" } else { "removed" };
        let action_vi = if linked { "thêm" } else { "gỡ" };
        return (
            l.pick("【Wishly】登入方式已變更".into(), "[Wishly] Sign-in method changed".into(), "[Wishly] Phương thức đăng nhập đã thay đổi".into()),
            l.pick(
                format!("你的帳號已{action_zh} {provider} 登入方式。\n若非本人操作，請立即檢查帳號設定及重設密碼。\n帳號設定：{app}/settings\n這是帳號安全通知。"),
                format!("The {provider} sign-in method was {action_en} on your account.\nIf this wasn't you, check your account settings and reset your password right away.\nAccount settings: {app}/settings\nThis is an account security notice."),
                format!("Phương thức đăng nhập {provider} đã được {action_vi} cho tài khoản của bạn.\nNếu không phải bạn thực hiện, hãy kiểm tra cài đặt tài khoản và đặt lại mật khẩu ngay.\nCài đặt tài khoản: {app}/settings\nĐây là thông báo bảo mật tài khoản."),
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
        .unwrap_or_else(|| l.pick("品項".into(), "item".into(), "món quà".into()));
    let wallet = format!("{base}/me/wallet");
    let (subject, body) = match kind {
        "crowdfund.funded" => (
            l.pick(format!("【Wishly】《{item}》集資達標了"), format!("[Wishly] \"{item}\" has reached its goal"), format!("[Wishly] \"{item}\" đã đạt mục tiêu góp quà")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）已集資達標，營運團隊會開始代購。\n查看我的點數與捐贈：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\"), which you backed, has reached its goal. Our team will start purchasing it.\nView my points and contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") mà bạn ủng hộ đã đạt mục tiêu, đội ngũ của chúng tôi sẽ bắt đầu mua hộ.\nXem điểm và các khoản đóng góp của tôi: {wallet}\n"),
            ),
        ),
        "order.shipped" => (
            l.pick(format!("【Wishly】《{item}》已出貨"), format!("[Wishly] \"{item}\" has shipped"), format!("[Wishly] \"{item}\" đã được gửi đi")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）已出貨。\n查看我的點數與捐贈：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\"), which you backed, has shipped.\nView my points and contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") mà bạn ủng hộ đã được gửi đi.\nXem điểm và các khoản đóng góp của tôi: {wallet}\n"),
            ),
        ),
        "order.delivered" => (
            l.pick(format!("【Wishly】《{item}》已送達"), format!("[Wishly] \"{item}\" has been delivered"), format!("[Wishly] \"{item}\" đã được giao")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）已送達受贈者，謝謝你的心意。\n查看我的點數與捐贈：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\"), which you backed, has been delivered to the recipient. Thank you for your kindness.\nView my points and contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") mà bạn ủng hộ đã được giao đến người nhận. Cảm ơn tấm lòng của bạn.\nXem điểm và các khoản đóng góp của tôi: {wallet}\n"),
            ),
        ),
        "order.failed" => (
            l.pick(format!("【Wishly】《{item}》代購未成功，點數已退回"), format!("[Wishly] \"{item}\" could not be purchased; points refunded"), format!("[Wishly] Không mua được \"{item}\", điểm đã được hoàn lại")),
            l.pick(
                format!("你支持的「{item}」（清單《{title}》）代購未成功，你捐出的點數已全數退回錢包。\n查看我的點數：{wallet}\n"),
                format!("The purchase of \"{item}\" (list \"{title}\"), which you backed, was unsuccessful. All the points you contributed have been refunded to your wallet.\nView my points: {wallet}\n"),
                format!("Việc mua \"{item}\" (danh sách \"{title}\") mà bạn ủng hộ không thành công. Toàn bộ điểm bạn đã đóng góp đã được hoàn vào ví.\nXem điểm của tôi: {wallet}\n"),
            ),
        ),
        "funding.expired" => (
            l.pick(format!("【Wishly】《{item}》募集期限已到"), format!("[Wishly] The funding period for \"{item}\" has ended"), format!("[Wishly] \"{item}\" đã hết hạn góp quà")),
            l.pick(
                format!("「{item}」（清單《{title}》）在期限內未達標。你的點數會再保留 7 天：期間可到錢包撤回，或轉捐給同一份清單的其他品項；逾期未處理會自動退回錢包。\n處理我的捐贈：{wallet}\n"),
                format!("\"{item}\" (list \"{title}\") did not reach its goal in time. Your points are held for another 7 days: you can withdraw them in your wallet or move them to another item on the same list; if you do nothing they are refunded to your wallet automatically.\nManage my contributions: {wallet}\n"),
                format!("\"{item}\" (danh sách \"{title}\") chưa đạt mục tiêu trong thời hạn. Điểm của bạn được giữ thêm 7 ngày: bạn có thể rút về ví hoặc chuyển sang món khác trong cùng danh sách; nếu không xử lý, điểm sẽ tự động hoàn vào ví.\nQuản lý đóng góp của tôi: {wallet}\n"),
            ),
        ),
        "item.removed" => (
            l.pick(format!("【Wishly】《{item}》已被移除，點數已退回"), format!("[Wishly] \"{item}\" was removed; points refunded"), format!("[Wishly] \"{item}\" đã bị gỡ, điểm đã được hoàn lại")),
            l.pick(
                format!("清單《{title}》的建立者移除了「{item}」（或封存了清單），你捐出的點數已退回錢包。\n查看我的點數：{wallet}\n"),
                format!("The owner of list \"{title}\" removed \"{item}\" (or archived the list), and the points you contributed have been refunded to your wallet.\nView my points: {wallet}\n"),
                format!("Chủ danh sách \"{title}\" đã gỡ \"{item}\" (hoặc lưu trữ danh sách), điểm bạn đã đóng góp đã được hoàn vào ví.\nXem điểm của tôi: {wallet}\n"),
            ),
        ),
        "claim.confirmation" => (
            l.pick("【Wishly】捐助已確認".into(), "[Wishly] Pledge confirmed".into(), "[Wishly] Đã xác nhận đóng góp".into()),
            l.pick(
                format!("你的捐助已記錄，謝謝你的心意。\n管理我的捐助：{manage}\n"),
                format!("Your pledge has been recorded. Thank you for your kindness.\nManage my pledges: {manage}\n"),
                format!("Đóng góp của bạn đã được ghi nhận. Cảm ơn tấm lòng của bạn.\nQuản lý đóng góp của tôi: {manage}\n"),
            ),
        ),
        "claim.item_removed" => (
            l.pick("【Wishly】你捐助的品項已被移除".into(), "[Wishly] An item you pledged for was removed".into(), "[Wishly] Món quà bạn đóng góp đã bị gỡ".into()),
            l.pick(
                format!("清單《{title}》的建立者移除了你捐助的品項，這筆捐助已取消。\n查看其他品項：{manage}\n"),
                format!("The owner of list \"{title}\" removed the item you pledged for, so this pledge has been cancelled.\nSee other items: {manage}\n"),
                format!("Chủ danh sách \"{title}\" đã gỡ món quà bạn đóng góp, khoản đóng góp này đã bị hủy.\nXem các món khác: {manage}\n"),
            ),
        ),
        "claim.item_changed" => {
            // 眾籌認捐在錢包管理，一般捐助在「我的捐助」
            let link = if payload["crowdfund"] == true { &wallet } else { &manage };
            (
                l.pick(format!("【Wishly】你捐助的《{item}》內容有更新"), format!("[Wishly] \"{item}\" you pledged for was updated"), format!("[Wishly] \"{item}\" bạn đóng góp đã được cập nhật")),
                l.pick(
                    format!("清單《{title}》的建立者修改了「{item}」的內容（例如數量、規格或說明）。購買前請再確認一次數量與規格。\n管理我的捐助：{link}\n"),
                    format!("The owner of list \"{title}\" updated \"{item}\" (for example its quantity, spec or description). Please re-check the quantity and spec before buying.\nManage my pledges: {link}\n"),
                    format!("Chủ danh sách \"{title}\" đã cập nhật \"{item}\" (ví dụ số lượng, quy cách hoặc mô tả). Vui lòng kiểm tra lại số lượng và quy cách trước khi mua.\nQuản lý đóng góp của tôi: {link}\n"),
                ),
            )
        }
        "wishlist.closing" => {
            // 以台北時間顯示；解析失敗就只講「24 小時內」
            let at = payload.get("closes_at").and_then(Value::as_str).and_then(|t| t.parse::<chrono::DateTime<chrono::Utc>>().ok())
                .map(|t| (t + chrono::Duration::hours(8)).format("%Y-%m-%d %H:%M").to_string());
            let (at_zh, at_en, at_vi) = match at {
                Some(a) => (format!("{a}（台北時間）"), format!("{a} (Taipei time)"), format!("{a} (giờ Đài Bắc)")),
                None => ("24 小時內".into(), "within 24 hours".into(), "trong vòng 24 giờ".into()),
            };
            (
                l.pick(format!("【Wishly】《{title}》即將結束"), format!("[Wishly] \"{title}\" is about to close"), format!("[Wishly] \"{title}\" sắp kết thúc")),
                l.pick(
                    format!("你的清單《{title}》將於 {at_zh}自動結束，屆時訪客不能再捐助。\n要延長或取消期限：{base}/dashboard\n"),
                    format!("Your list \"{title}\" will close automatically {at_en}, after which visitors can no longer pledge.\nTo extend or cancel the deadline: {base}/dashboard\n"),
                    format!("Danh sách \"{title}\" của bạn sẽ tự động kết thúc vào {at_vi}, sau đó khách không thể đóng góp nữa.\nĐể gia hạn hoặc hủy thời hạn: {base}/dashboard\n"),
                ),
            )
        }
        "event.reminder" => (
            l.pick(format!("【Wishly】《{title}》活動日快到了"), format!("[Wishly] The event day for \"{title}\" is coming up"), format!("[Wishly] Sắp đến ngày sự kiện của \"{title}\"")),
            l.pick(
                format!("你的清單《{title}》的活動日即將到來。\n{base}/dashboard\n"),
                format!("The event day for your list \"{title}\" is coming up.\n{base}/dashboard\n"),
                format!("Ngày sự kiện của danh sách \"{title}\" sắp đến.\n{base}/dashboard\n"),
            ),
        ),
        _ => (l.pick("【Wishly】通知".into(), "[Wishly] Notification".into(), "[Wishly] Thông báo".into()), format!("{base}\n")),
    };
    match payload.get("unsubscribe_url").and_then(Value::as_str) {
        Some(u) => {
            let foot = l.pick(
                format!("退訂此類通知：{u}"),
                format!("Unsubscribe from these notifications: {u}"),
                format!("Hủy nhận loại thông báo này: {u}"),
            );
            (subject, format!("{body}\n{foot}\n"))
        }
        None => (subject, body),
    }
}
