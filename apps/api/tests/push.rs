//! Web Push（push.rs）：設定開關、訂閱管理、SSRF 白名單、實際送出的加密請求與失敗處理。
mod common;
use aes_gcm::{aead::Aead, Aes128Gcm, KeyInit};
use axum::{body::Bytes, extract::State, http::HeaderMap, Router};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use common::{call, ck, user, wishlist, ListOpts, TestList};
use hkdf::Hkdf;
use p256::{elliptic_curve::sec1::ToSec1Point, PublicKey, SecretKey};
use serde_json::{json, Value};
use sha2::Sha256;
use sqlx::PgPool;
use std::sync::{
    atomic::{AtomicU16, Ordering},
    Arc, Mutex,
};
use uuid::Uuid;
use wishly_api::config::Config;

const VAPID_KEY: &str = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw";

fn init_cfg() {
    let mut cfg = Config::dev();
    cfg.vapid = wishly_api::push::Vapid::from_env(
        Some(VAPID_KEY.into()),
        Some("mailto:ops@wishly.tw".into()),
    )
    .unwrap();
    cfg.push_allow_loopback = true;
    wishly_api::config::init(cfg);
}

/// 模擬瀏覽器端的訂閱金鑰
struct Device {
    secret: SecretKey,
    auth: [u8; 16],
}
impl Device {
    fn new() -> Self {
        Device {
            secret: wishly_api::push::random_key(),
            auth: rand::random(),
        }
    }
    fn public(&self) -> Vec<u8> {
        self.secret
            .public_key()
            .to_sec1_point(false)
            .as_bytes()
            .to_vec()
    }
    fn body(&self, endpoint: &str) -> Value {
        json!({ "endpoint": endpoint, "keys": { "p256dh": B64.encode(self.public()), "auth": B64.encode(self.auth) } })
    }
    /// 獨立於 push::encrypt 的 RFC 8291 解密
    fn decrypt(&self, body: &[u8]) -> String {
        decrypt(&self.secret, &self.auth, body)
    }
}

fn decrypt(ua: &SecretKey, auth: &[u8], body: &[u8]) -> String {
    let (salt, idlen) = (&body[..16], body[20] as usize);
    let as_pub = &body[21..21 + idlen];
    let ct = &body[21 + idlen..];
    let shared = p256::ecdh::diffie_hellman(
        ua.to_nonzero_scalar(),
        PublicKey::from_sec1_bytes(as_pub).unwrap().as_affine(),
    );
    let ua_pub = ua.public_key().to_sec1_point(false);
    let info = [b"WebPush: info\0".as_slice(), ua_pub.as_bytes(), as_pub].concat();
    let mut ikm = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes())
        .expand(&info, &mut ikm)
        .unwrap();
    let h = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let (mut cek, mut nonce) = ([0u8; 16], [0u8; 12]);
    h.expand(b"Content-Encoding: aes128gcm\0", &mut cek)
        .unwrap();
    h.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();
    let mut plain = Aes128Gcm::new(&cek.into())
        .decrypt(&nonce.into(), ct)
        .unwrap();
    assert_eq!(plain.pop(), Some(2));
    String::from_utf8(plain).unwrap()
}

type Seen = Arc<Mutex<Vec<(HeaderMap, Bytes)>>>;
/// 本機假推播服務：記錄請求，回傳指定狀態碼
async fn mock(status: u16) -> (String, Seen, Arc<AtomicU16>) {
    let seen: Seen = Default::default();
    let code = Arc::new(AtomicU16::new(status));
    let app = Router::new()
        .fallback(
            |State((s, c)): State<(Seen, Arc<AtomicU16>)>, h: HeaderMap, b: Bytes| async move {
                s.lock().unwrap().push((h, b));
                axum::http::StatusCode::from_u16(c.load(Ordering::SeqCst)).unwrap()
            },
        )
        .with_state((seen.clone(), code.clone()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/push/{}", l.local_addr().unwrap(), Uuid::new_v4());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (url, seen, code)
}

async fn queue_mail(pool: &PgPool, uid: Uuid) {
    let TestList { id: wl, .. } = wishlist(pool, uid, ListOpts::default()).await;
    sqlx::query("INSERT INTO notifications (user_id, channel, kind, payload) VALUES ($1, 'email', 'event.reminder', jsonb_build_object('wishlist_id', $2::uuid))")
        .bind(uid).bind(wl).execute(pool).await.unwrap();
}

async fn sub_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM push_subscriptions")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[test]
fn vapid_config_is_optional_but_validated() {
    use wishly_api::push::Vapid;
    assert!(Vapid::from_env(None, None).unwrap().is_none());
    assert!(Vapid::from_env(Some("not base64!".into()), Some("mailto:a@b.c".into())).is_err());
    assert!(Vapid::from_env(Some(B64.encode([1u8; 8])), Some("mailto:a@b.c".into())).is_err());
    assert!(Vapid::from_env(Some(VAPID_KEY.into()), None).is_err());
    let v = Vapid::from_env(Some(VAPID_KEY.into()), Some("mailto:a@b.c".into()))
        .unwrap()
        .unwrap();
    assert_eq!(B64.decode(&v.public_key).unwrap().len(), 65);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn config_endpoint_and_auth(pool: PgPool) {
    init_cfg();
    let (s, _, b) = call(&pool, "GET", "/push/config", &[], None).await;
    assert_eq!(s, 200);
    assert_eq!(
        B64.decode(b["public_key"].as_str().unwrap()).unwrap().len(),
        65
    );
    let d = Device::new();
    let (s, ..) = call(
        &pool,
        "POST",
        "/me/push-subscriptions",
        &[],
        Some(d.body("https://fcm.googleapis.com/x")),
    )
    .await;
    assert_eq!(s, 401);
    let (s, ..) = call(
        &pool,
        "DELETE",
        "/me/push-subscriptions",
        &[],
        Some(json!({ "endpoint": "x" })),
    )
    .await;
    assert_eq!(s, 401);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn subscribe_upserts_reassigns_and_delete_is_own_only(pool: PgPool) {
    init_cfg();
    let (a, b) = (user(&pool, "A").await, user(&pool, "B").await);
    let d = Device::new();
    let ep = "https://fcm.googleapis.com/fcm/send/abc";
    for _ in 0..2 {
        let (s, ..) = call(
            &pool,
            "POST",
            "/me/push-subscriptions",
            &ck(&a.token),
            Some(d.body(ep)),
        )
        .await;
        assert_eq!(s, 204);
    }
    assert_eq!(sub_count(&pool).await, 1);
    // B 不能刪 A 的
    call(
        &pool,
        "DELETE",
        "/me/push-subscriptions",
        &ck(&b.token),
        Some(json!({ "endpoint": ep })),
    )
    .await;
    assert_eq!(sub_count(&pool).await, 1);
    // 同一 endpoint 換帳號登入：歸屬改到 B
    call(
        &pool,
        "POST",
        "/me/push-subscriptions",
        &ck(&b.token),
        Some(d.body(ep)),
    )
    .await;
    let owner: Uuid =
        sqlx::query_scalar("SELECT user_id FROM push_subscriptions WHERE endpoint=$1")
            .bind(ep)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (owner, sub_count(&pool).await),
        (b.id, 1),
        "push endpoint not reassigned to the new user"
    );
    // A 已不是擁有者，刪不掉；B 刪得掉
    call(
        &pool,
        "DELETE",
        "/me/push-subscriptions",
        &ck(&a.token),
        Some(json!({ "endpoint": ep })),
    )
    .await;
    assert_eq!(sub_count(&pool).await, 1);
    let (s, ..) = call(
        &pool,
        "DELETE",
        "/me/push-subscriptions",
        &ck(&b.token),
        Some(json!({ "endpoint": ep })),
    )
    .await;
    assert_eq!((s.as_u16(), sub_count(&pool).await), (204, 0));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn endpoint_whitelist_and_key_validation(pool: PgPool) {
    init_cfg();
    let u = user(&pool, "A").await;
    let d = Device::new();
    for bad in [
        "http://fcm.googleapis.com/x",
        "https://10.0.0.1/x",
        "https://127.0.0.1/x",
        "http://127.0.0.2:9/x",
        "https://fcm.googleapis.com.evil.com/x",
        "https://fcm.googleapis.com@evil.com/",
        "https://evil.com/fcm.googleapis.com",
        "file:///etc/passwd",
    ] {
        let (s, ..) = call(
            &pool,
            "POST",
            "/me/push-subscriptions",
            &ck(&u.token),
            Some(d.body(bad)),
        )
        .await;
        assert_eq!(s, 422, "{bad}");
    }
    let ep = "https://web.push.apple.com/x";
    let mut b = d.body(ep);
    b["keys"]["p256dh"] = json!(B64.encode([4u8; 65]));
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/push-subscriptions",
            &ck(&u.token),
            Some(b)
        )
        .await
        .0,
        422
    );
    let mut b = d.body(ep);
    b["keys"]["auth"] = json!("short");
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/push-subscriptions",
            &ck(&u.token),
            Some(b)
        )
        .await
        .0,
        422
    );
    assert_eq!(sub_count(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn per_user_limit_drops_oldest(pool: PgPool) {
    init_cfg();
    let u = user(&pool, "A").await;
    let d = Device::new();
    for i in 0..12 {
        let (s, ..) = call(
            &pool,
            "POST",
            "/me/push-subscriptions",
            &ck(&u.token),
            Some(d.body(&format!("https://fcm.googleapis.com/{i}"))),
        )
        .await;
        assert_eq!(s, 204);
    }
    assert_eq!(sub_count(&pool).await, 10);
    let oldest: i64 = sqlx::query_scalar("SELECT count(*) FROM push_subscriptions WHERE endpoint IN ('https://fcm.googleapis.com/0','https://fcm.googleapis.com/1')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(oldest, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mail_success_pushes_encrypted_payload_without_body(pool: PgPool) {
    init_cfg();
    let u = user(&pool, "A").await;
    let (url, seen, _) = mock(201).await;
    let d = Device::new();
    let (s, ..) = call(
        &pool,
        "POST",
        "/me/push-subscriptions",
        &ck(&u.token),
        Some(d.body(&url)),
    )
    .await;
    assert_eq!(s, 204);
    queue_mail(&pool, u.id).await;
    assert_eq!(wishly_api::notify::tick(&pool).await.unwrap(), 1);
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (h, body) = &seen[0];
    let auth = h["authorization"].to_str().unwrap();
    assert!(auth.starts_with("vapid t="), "{auth}");
    let cfg = wishly_api::config::get();
    assert!(auth.ends_with(&format!(", k={}", cfg.vapid.unwrap().public_key)));
    assert_eq!(h["content-encoding"], "aes128gcm");
    assert_eq!(h["ttl"], "86400");
    let jwt = auth
        .strip_prefix("vapid t=")
        .unwrap()
        .split(',')
        .next()
        .unwrap();
    let claims: Value =
        serde_json::from_slice(&B64.decode(jwt.split('.').nth(1).unwrap()).unwrap()).unwrap();
    assert_eq!(
        (
            claims["sub"].as_str(),
            claims["aud"].as_str().map(|a| url.starts_with(a))
        ),
        (Some("mailto:ops@wishly.tw"), Some(true))
    );
    let payload: Value = serde_json::from_str(&d.decrypt(body)).unwrap();
    assert_eq!(payload["path"], "/dashboard");
    assert!(!payload["title"].as_str().unwrap().is_empty());
    assert_eq!(payload.as_object().unwrap().len(), 2, "只含標題與路徑");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn gone_deletes_subscription_and_server_error_keeps_it_and_notification_sent(pool: PgPool) {
    init_cfg();
    let u = user(&pool, "A").await;
    let (url, seen, code) = mock(500).await;
    let d = Device::new();
    call(
        &pool,
        "POST",
        "/me/push-subscriptions",
        &ck(&u.token),
        Some(d.body(&url)),
    )
    .await;
    queue_mail(&pool, u.id).await;
    wishly_api::notify::tick(&pool).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(sub_count(&pool).await, 1);
    let st: String = sqlx::query_scalar("SELECT status::text FROM notifications WHERE user_id=$1")
        .bind(u.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(st, "sent");
    for c in [404, 410] {
        code.store(c, Ordering::SeqCst);
        call(
            &pool,
            "POST",
            "/me/push-subscriptions",
            &ck(&u.token),
            Some(d.body(&url)),
        )
        .await;
        queue_mail(&pool, u.id).await;
        wishly_api::notify::tick(&pool).await.unwrap();
        assert_eq!(sub_count(&pool).await, 0, "{c}");
    }
}

/// 對照 RFC 8291 附錄 A 的對端：用獨立的解密實作解開 RFC 範例本文
#[test]
fn independent_decrypt_opens_rfc_example() {
    let d = |s: &str| B64.decode(s).unwrap();
    let body = d("DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN");
    let ua = SecretKey::from_slice(&d("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")).unwrap();
    assert_eq!(
        decrypt(&ua, &d("BTBZMqHH6r4Tts7J_aSIgg"), &body),
        "When I grow up, I want to be a watermelon"
    );
}
