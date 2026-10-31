//! Web Push（D68）：登入使用者在每台裝置開啟推播；時機與合併跟 email 相同（notify::tick），但不依賴 email 寄出。
//! 加密為 RFC 8291 aes128gcm、身分為 RFC 8292 VAPID（ES256）。內容只有標題與站內路徑，不放信件內文（鎖定畫面會外洩）。
//! endpoint 來自使用者、伺服器會對它發 POST：必須是 https 且 host 精確落在推播服務白名單（SSRF），且不跟隨重導向。
use crate::{config, error::AppError, session::CurrentUser, AppState};
use aes_gcm::{aead::Aead, Aes128Gcm, KeyInit};
use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use hkdf::Hkdf;
use p256::{
    ecdsa::{signature::Signer, Signature, SigningKey},
    elliptic_curve::sec1::ToSec1Point,
    PublicKey, SecretKey,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

/// 每位使用者最多幾台裝置；超過刪最舊
const MAX_PER_USER: i64 = 10;
/// 序列化後的 payload 上限（推播服務單則上限約 4KB）
const MAX_PAYLOAD: usize = 3072;

/// VAPID 設定（VAPID_PRIVATE_KEY 為 base64url 的 P-256 私鑰，機密；刻意不 derive Debug）
#[derive(Clone)]
pub struct Vapid {
    key: SecretKey,
    pub public_key: String,
    subject: String,
}

impl Vapid {
    /// 未設金鑰 = 推播關閉；金鑰格式錯誤或缺 subject 回 Err（啟動失敗）
    pub fn from_env(key: Option<String>, subject: Option<String>) -> Result<Option<Vapid>, String> {
        let Some(k) = key else { return Ok(None) };
        let raw = B64
            .decode(k.trim())
            .map_err(|_| "VAPID_PRIVATE_KEY 必須是 base64url")?;
        let key = SecretKey::from_slice(&raw)
            .map_err(|_| "VAPID_PRIVATE_KEY 必須是 32 bytes 的 P-256 私鑰")?;
        let subject = subject
            .filter(|s| s.starts_with("mailto:") || s.starts_with("https://"))
            .ok_or("設了 VAPID_PRIVATE_KEY 就必須設 VAPID_SUBJECT（mailto:… 或 https://…）")?;
        let public_key = B64.encode(key.public_key().to_sec1_point(false).as_bytes());
        Ok(Some(Vapid {
            key,
            public_key,
            subject,
        }))
    }

    /// RFC 8292：Authorization: vapid t=<jwt>, k=<公鑰>
    fn authorization(&self, aud: &str) -> String {
        let exp = chrono::Utc::now().timestamp() + 12 * 3600;
        let head = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = B64.encode(json!({ "aud": aud, "exp": exp, "sub": self.subject }).to_string());
        let msg = format!("{head}.{claims}");
        let sig: Signature = SigningKey::from(&self.key).sign(msg.as_bytes());
        format!(
            "vapid t={msg}.{}, k={}",
            B64.encode(sig.to_bytes()),
            self.public_key
        )
    }
}

/// 精確比對 URL 解析後的 host，不用字串包含。allow_loopback 只有測試程式會設（放行本機 http mock），production 不可能為 true。
fn endpoint_ok(endpoint: &str, allow_loopback: bool) -> bool {
    let Ok(u) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    if allow_loopback && u.scheme() == "http" && u.host_str() == Some("127.0.0.1") {
        return true;
    }
    let Some(host) = u.host_str() else {
        return false;
    };
    u.scheme() == "https"
        && u.username().is_empty()
        && u.password().is_none()
        && matches!(u.port(), None | Some(443))
        && (matches!(
            host,
            "fcm.googleapis.com" | "updates.push.services.mozilla.com"
        ) || [
            ".push.apple.com",
            ".notify.windows.com",
            ".push.services.mozilla.com",
        ]
        .iter()
        .any(|s| host.len() > s.len() && host.ends_with(s)))
}

/// RFC 8291 §3：回傳完整 aes128gcm 本文（header + 密文）。as_key / salt 由呼叫端提供以便用 RFC 附錄向量驗證。
pub fn encrypt(
    ua_public: &[u8],
    auth: &[u8],
    plain: &[u8],
    as_key: &SecretKey,
    salt: [u8; 16],
) -> Result<Vec<u8>, String> {
    let ua = PublicKey::from_sec1_bytes(ua_public).map_err(|e| e.to_string())?;
    let as_pub = as_key.public_key().to_sec1_point(false);
    let shared = p256::ecdh::diffie_hellman(as_key.to_nonzero_scalar(), ua.as_affine());
    let mut ikm = [0u8; 32];
    let info = [b"WebPush: info\0".as_slice(), ua_public, as_pub.as_bytes()].concat();
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&info, &mut ikm)
        .map_err(|e| e.to_string())?;
    let h = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
    h.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .map_err(|e| e.to_string())?;
    h.expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|e| e.to_string())?;
    // 單一 record：明文後補 0x02 分隔符（最後一筆）
    let ct = Aes128Gcm::new(&cek.into())
        .encrypt(&nonce.into(), [plain, &[2]].concat().as_slice())
        .map_err(|e| e.to_string())?;
    let mut out = [
        &salt[..],
        &4096u32.to_be_bytes(),
        &[as_pub.len() as u8],
        as_pub.as_bytes(),
    ]
    .concat();
    out.extend(ct);
    Ok(out)
}

/// 隨機 P-256 私鑰（p256 0.14 的 rand_core 與專案的 rand 0.8 不相容，直接用 32 隨機 bytes；超出曲線階數時重抽）
pub fn random_key() -> SecretKey {
    loop {
        if let Ok(k) = SecretKey::from_slice(&rand::random::<[u8; 32]>()) {
            return k;
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/push/config", get(push_config)).route(
        "/me/push-subscriptions",
        axum::routing::post(subscribe).delete(unsubscribe),
    )
}

async fn push_config() -> Json<Value> {
    Json(json!({ "public_key": config::get().vapid.map(|v| v.public_key) }))
}

#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}
#[derive(Deserialize)]
struct Sub {
    endpoint: String,
    keys: Keys,
}

async fn subscribe(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(b): Json<Sub>,
) -> Result<StatusCode, AppError> {
    let cfg = config::get();
    if cfg.vapid.is_none() {
        return Err(AppError::invalid(
            "/endpoint",
            "PUSH_DISABLED",
            "伺服器未啟用推播",
        ));
    }
    if b.endpoint.len() > 2048 || !endpoint_ok(&b.endpoint, cfg.push_allow_loopback) {
        return Err(AppError::invalid(
            "/endpoint",
            "ENDPOINT_NOT_ALLOWED",
            "不支援的推播服務",
        ));
    }
    let key_ok = B64
        .decode(&b.keys.p256dh)
        .ok()
        .is_some_and(|k| k.len() == 65 && PublicKey::from_sec1_bytes(&k).is_ok());
    if !key_ok {
        return Err(AppError::invalid(
            "/keys/p256dh",
            "INVALID_KEY",
            "p256dh 格式錯誤",
        ));
    }
    if !B64.decode(&b.keys.auth).ok().is_some_and(|k| k.len() == 16) {
        return Err(AppError::invalid(
            "/keys/auth",
            "INVALID_KEY",
            "auth 格式錯誤",
        ));
    }
    let mut tx = st.pool.begin().await?;
    // 同一 endpoint 換帳號登入時歸屬改到新使用者
    sqlx::query("INSERT INTO push_subscriptions (user_id, endpoint, p256dh, auth) VALUES ($1,$2,$3,$4)
                 ON CONFLICT (endpoint) DO UPDATE SET user_id = EXCLUDED.user_id, p256dh = EXCLUDED.p256dh, auth = EXCLUDED.auth, created_at = now()")
        .bind(u.id).bind(&b.endpoint).bind(&b.keys.p256dh).bind(&b.keys.auth).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1 AND id NOT IN
                 (SELECT id FROM push_subscriptions WHERE user_id = $1 ORDER BY created_at DESC, id LIMIT $2)")
        .bind(u.id).bind(MAX_PER_USER).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Unsub {
    endpoint: String,
}

async fn unsubscribe(
    u: CurrentUser,
    State(st): State<AppState>,
    Json(b): Json<Unsub>,
) -> Result<StatusCode, AppError> {
    sqlx::query("DELETE FROM push_subscriptions WHERE user_id = $1 AND endpoint = $2")
        .bind(u.id)
        .bind(&b.endpoint)
        .execute(&st.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// notify worker 在寄送階段呼叫。任何失敗只 warn：不 panic、不影響通知狀態、不重試。
pub async fn deliver(pool: &PgPool, uid: Uuid, title: &str, path: &str) {
    let cfg = config::get();
    let Some(vapid) = cfg.vapid.as_ref() else {
        return;
    };
    let subs: Vec<(Uuid, String, String, String)> = match sqlx::query_as(
        "SELECT id, endpoint, p256dh, auth FROM push_subscriptions WHERE user_id = $1",
    )
    .bind(uid)
    .fetch_all(pool)
    .await
    {
        Ok(s) => s,
        Err(e) => return tracing::warn!("push: 查訂閱失敗：{e}"),
    };
    if subs.is_empty() {
        return;
    }
    let mut title: String = title.into();
    let plain = loop {
        let p = json!({ "title": title, "path": path }).to_string();
        if p.len() <= MAX_PAYLOAD || title.is_empty() {
            break p;
        }
        title.pop();
    };
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return;
    };
    // 並行送出：通知 worker 是循序的，最久只被單次逾時（5 秒）卡住，不隨裝置數累加
    futures_util::future::join_all(subs.iter().map(|(id, endpoint, p256dh, auth)| {
        let (client, plain) = (&client, plain.as_bytes());
        async move {
            match send_one(client, vapid, endpoint, p256dh, auth, plain).await {
                Ok(s) if s == StatusCode::NOT_FOUND || s == StatusCode::GONE => {
                    let _ = sqlx::query("DELETE FROM push_subscriptions WHERE id = $1")
                        .bind(id)
                        .execute(pool)
                        .await;
                }
                Ok(s) if !s.is_success() => tracing::warn!("push: 推播服務回 {s}"),
                Ok(_) => {}
                Err(e) => tracing::warn!("push: 送出失敗：{e}"),
            }
        }
    }))
    .await;
}

async fn send_one(
    c: &reqwest::Client,
    v: &Vapid,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    plain: &[u8],
) -> Result<StatusCode, String> {
    let url = reqwest::Url::parse(endpoint).map_err(|e| e.to_string())?;
    let aud = url.origin().ascii_serialization();
    let (ua, auth) = (
        B64.decode(p256dh).map_err(|e| e.to_string())?,
        B64.decode(auth).map_err(|e| e.to_string())?,
    );
    let mut salt = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
    let body = encrypt(&ua, &auth, plain, &random_key(), salt)?;
    let r = c
        .post(url)
        .header("Authorization", v.authorization(&aud))
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", "86400")
        .body(body)
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    Ok(r.status())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc8291_appendix_a_vector() {
        let d = |s: &str| B64.decode(s).unwrap();
        let as_key =
            SecretKey::from_slice(&d("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let salt: [u8; 16] = d("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let body = encrypt(
            &d("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
            &d("BTBZMqHH6r4Tts7J_aSIgg"),
            b"When I grow up, I want to be a watermelon",
            &as_key,
            salt,
        )
        .unwrap();
        assert_eq!(
            B64.encode(body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
    }

    #[test]
    fn endpoint_whitelist() {
        for ok in [
            "https://fcm.googleapis.com/fcm/send/x",
            "https://web.push.apple.com/Q",
            "https://wns2.notify.windows.com/w",
            "https://updates.push.services.mozilla.com/wpush/v2/x",
        ] {
            assert!(endpoint_ok(ok, false), "{ok}");
        }
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://127.0.0.1/x",
            "https://fcm.googleapis.com.evil.com/x",
            "https://fcm.googleapis.com@evil.com/",
            "https://evil.com/fcm.googleapis.com",
            "https://push.apple.com/x",
            "https://fcm.googleapis.com:8443/x",
            "https://x.push.apple.com.evil.com/",
        ] {
            assert!(!endpoint_ok(bad, false), "{bad}");
        }
    }
}
