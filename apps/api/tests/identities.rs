mod common;
use axum::http::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;
use wishsync_api::{auth::code_hash, session::hash_token};

async fn call(
    pool: &PgPool,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let h = cookie.map(common::ck).unwrap_or_default();
    let (s, _, b) = common::api(pool, method, path, &h, body).await;
    (s, b)
}

async fn user(pool: &PgPool, provider: &str) -> (Uuid, String) {
    let (id, token) = common::user_pair(pool, "LINE 測試").await;
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id, provider, provider_uid) VALUES ($1,$2::auth_provider,$3)")
        .bind(id).bind(provider).bind(Uuid::new_v4().to_string()).execute(pool).await.unwrap();
    (id, token)
}

async fn wait_for_lock(pool: &PgPool) {
    common::wait_until("identity lock wait", || async {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND pid<>pg_backend_pid())")
            .fetch_one(pool).await.unwrap()
    }).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn email_link_is_scoped_and_does_not_create_another_account(pool: PgPool) {
    let (id, token) = user(&pool, "line").await;
    let (_, other) = user(&pool, "line").await;
    let request = json!({"email":"LINK@example.com"});
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            None,
            Some(request.clone())
        )
        .await
        .0,
        401
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(request.clone())
        )
        .await
        .0,
        202
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(request)
        )
        .await
        .0,
        429
    );
    // 固定測試碼，不依賴寄信或暴力解碼。
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email='link@example.com'")
        .bind(code_hash("123456", "dev-pepper"))
        .execute(&pool)
        .await
        .unwrap();
    let verify = json!({"email":"link@example.com","code":"123456"});
    assert_eq!(
        call(
            &pool,
            "POST",
            "/auth/otp/verify",
            None,
            Some(verify.clone())
        )
        .await
        .0,
        400
    );
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/verify",
            Some(&other),
            Some(verify.clone())
        )
        .await
        .0,
        400
    );
    let (status, me) = call(
        &pool,
        "POST",
        "/me/identities/email/verify",
        Some(&token),
        Some(verify.clone()),
    )
    .await;
    assert_eq!(status, 200, "{me}");
    assert_eq!(me["id"], id.to_string());
    assert_eq!(me["email"], "link@example.com");
    assert_eq!(me["identities"].as_array().unwrap().len(), 2);
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/verify",
            Some(&token),
            Some(verify)
        )
        .await
        .0,
        400
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='auth.identity_linked'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unlink_serializes_and_keeps_one_login_method(pool: PgPool) {
    let (id, token) = user(&pool, "line").await;
    let (s, b) = call(&pool, "DELETE", "/me/identities/line", Some(&token), None).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("LAST_IDENTITY"))
    );
    sqlx::query(
        "INSERT INTO auth_identities (user_id,provider,provider_uid) VALUES ($1,'google','g')",
    )
    .bind(id)
    .execute(&pool)
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        call(&pool, "DELETE", "/me/identities/line", Some(&token), None),
        call(&pool, "DELETE", "/me/identities/google", Some(&token), None)
    );
    assert!(
        (a.0 == StatusCode::OK && b.0 == StatusCode::CONFLICT)
            || (b.0 == StatusCode::OK && a.0 == StatusCode::CONFLICT),
        "{a:?} {b:?}"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_identities WHERE user_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unlink_email_clears_contact_and_blocks_implicit_relink(pool: PgPool) {
    let (id, token) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET email='old@example.com' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'email','old@example.com','old@example.com')").bind(id).execute(&pool).await.unwrap();
    let (s, me) = call(&pool, "DELETE", "/me/identities/email", Some(&token), None).await;
    assert_eq!(s, 200, "{me}");
    assert!(me["email"].is_null());
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM notifications WHERE user_id=$1 AND kind='auth.identity_unlinked'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(payload["to"], "old@example.com", "安全信仍寄往解除前的信箱");
    wishsync_api::notify::tick(&pool).await.unwrap();
    let status: String = sqlx::query_scalar(
        "SELECT status::text FROM notifications WHERE user_id=$1 AND kind='auth.identity_unlinked'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_ne!(status, "cancelled", "解除 email 後仍保留安全通知的收件人");
    assert_eq!(call(&pool, "GET", "/me", Some(&token), None).await.0, 200);
}

#[test]
fn security_mail_explains_change_and_cannot_be_unsubscribed() {
    for (kind, action) in [
        ("auth.identity_linked", "新增"),
        ("auth.identity_unlinked", "解除"),
    ] {
        let (subject, body) = wishsync_api::notify::render(
            kind,
            "",
            &json!({"provider":"google","unsubscribe_url":"https://bad.example/unsubscribe"}),
        );
        assert!(subject.contains("登入方式"), "{subject}");
        assert!(
            body.contains("Google") && body.contains(action) && body.contains("/settings"),
            "{body}"
        );
        assert!(!body.contains("退訂") && !body.contains("bad.example"));
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn email_link_refuses_existing_account(pool: PgPool) {
    let (_, token) = user(&pool, "line").await;
    let (other, _) = user(&pool, "google").await;
    sqlx::query("UPDATE users SET email='taken@example.com' WHERE id=$1")
        .bind(other)
        .execute(&pool)
        .await
        .unwrap();
    // 請求仍回 202；通過信箱驗證才揭露衝突。
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(json!({"email":"taken@example.com"}))
        )
        .await
        .0,
        202
    );
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email='taken@example.com'")
        .bind(code_hash("123456", "dev-pepper"))
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = call(
        &pool,
        "POST",
        "/me/identities/email/verify",
        Some(&token),
        Some(json!({"email":"taken@example.com","code":"123456"})),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("IDENTITY_IN_USE"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn read_only_and_blocked_accounts_cannot_link(pool: PgPool) {
    let (id, token) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET blocked_at=now() WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(json!({"email":"blocked@example.com"}))
        )
        .await
        .0,
        403
    );
    sqlx::query("UPDATE users SET blocked_at=NULL WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO system_flags (key,value) VALUES ('read_only','true') ON CONFLICT (key) DO UPDATE SET value='true'").execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(json!({"email":"blocked@example.com"}))
        )
        .await
        .0,
        503
    );
    assert_eq!(
        call(&pool, "DELETE", "/me/identities/line", Some(&token), None)
            .await
            .0,
        503
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn reset_rechecks_contact_after_concurrent_unlink(pool: PgPool) {
    let (id, _) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET email='race@example.com' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'email','race@example.com','race@example.com')").bind(id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO otp_challenges (email,code_hash,purpose,expires_at) VALUES ('race@example.com',$1,'reset',now()+interval '10 minutes')").bind(code_hash("123456","dev-pepper")).execute(&pool).await.unwrap();
    let mut unlink = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM auth_identities WHERE user_id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    let reset = tokio::spawn({
        let pool = pool.clone();
        async move {
            call(&pool,"POST","/auth/password/reset/confirm",None,Some(json!({"email":"race@example.com","code":"123456","new_password":"valid-password"}))).await
        }
    });
    wait_for_lock(&pool).await;
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1 AND provider='email'")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    unlink.commit().await.unwrap();
    assert_eq!(
        reset.await.unwrap().0,
        400,
        "解除完成後，舊重設請求不得重新建立登入方式"
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auth_identities WHERE user_id=$1 AND provider='email'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn registration_identity_and_password_are_atomic(pool: PgPool) {
    let (id, _) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET email='register-race@example.com' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'email','register-race@example.com','register-race@example.com')").bind(id).execute(&pool).await.unwrap();
    assert_eq!(call(&pool,"POST","/auth/register",None,Some(json!({"email":"register-race@example.com","password":"valid-password","display_name":"new"}))).await.0,202);
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email='register-race@example.com'")
        .bind(code_hash("123456", "dev-pepper"))
        .execute(&pool)
        .await
        .unwrap();
    let mut unlink = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM auth_identities WHERE user_id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    let register = tokio::spawn({
        let pool = pool.clone();
        async move {
            call(
                &pool,
                "POST",
                "/auth/register/verify",
                None,
                Some(json!({"email":"register-race@example.com","code":"123456"})),
            )
            .await
        }
    });
    wait_for_lock(&pool).await;
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1 AND provider='email'")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET email=NULL WHERE id=$1")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    unlink.commit().await.unwrap();
    let (s, b) = register.await.unwrap();
    assert_eq!(s, 200, "{b}");
    let identity: Option<(Uuid,Option<String>)> = sqlx::query_as("SELECT user_id,password_hash FROM auth_identities WHERE provider='email' AND provider_uid='register-race@example.com'").fetch_optional(&pool).await.unwrap();
    let (owner, password) = identity.expect("註冊成功一定有可用的 Email identity");
    assert_eq!(b["user"]["id"], owner.to_string());
    assert!(password.unwrap().starts_with("$argon2id$"));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn email_link_checks_email_on_other_provider_identity(pool: PgPool) {
    let (other, _) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET email='contact@example.com' WHERE id=$1")
        .bind(other)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'google','alternate-google','alternate@example.com')").bind(other).execute(&pool).await.unwrap();
    let (_, token) = user(&pool, "line").await;
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(json!({"email":"alternate@example.com"}))
        )
        .await
        .0,
        202
    );
    sqlx::query("UPDATE otp_challenges SET code_hash=$1 WHERE email='alternate@example.com'")
        .bind(code_hash("123456", "dev-pepper"))
        .execute(&pool)
        .await
        .unwrap();
    let (s, b) = call(
        &pool,
        "POST",
        "/me/identities/email/verify",
        Some(&token),
        Some(json!({"email":"alternate@example.com","code":"123456"})),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("IDENTITY_IN_USE"))
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn same_email_unlink_requires_fresh_verification_to_relink(pool: PgPool) {
    let (id, token) = user(&pool, "google").await;
    sqlx::query("UPDATE users SET email='same@example.com' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE auth_identities SET email='same@example.com' WHERE user_id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'email','same@example.com','same@example.com')").bind(id).execute(&pool).await.unwrap();
    let (s, me) = call(&pool, "DELETE", "/me/identities/email", Some(&token), None).await;
    assert_eq!(s, 200);
    assert_eq!(
        me["email"], "same@example.com",
        "Google 的已驗證信箱仍可收通知"
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auth_identities WHERE user_id=$1 AND provider='email'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        call(
            &pool,
            "POST",
            "/auth/otp/verify",
            None,
            Some(json!({"email":"same@example.com","code":"123456"}))
        )
        .await
        .0,
        400
    );
    // 使用者重新證明信箱所有權，沿用原有 verified-email 合併政策。
    sqlx::query("INSERT INTO otp_challenges (email,code_hash,expires_at) VALUES ('same@example.com',$1,now()+interval '10 minutes')").bind(code_hash("123456","dev-pepper")).execute(&pool).await.unwrap();
    let (s, b) = call(
        &pool,
        "POST",
        "/auth/otp/verify",
        None,
        Some(json!({"email":"same@example.com","code":"123456"})),
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(b["user"]["id"], id.to_string());
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notifications WHERE user_id=$1 AND kind='auth.identity_linked'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn auto_email_link_rechecks_alternate_email_after_unlink(pool: PgPool) {
    let (id, _) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET email='contact@example.com' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'google','alternate','alternate@example.com')").bind(id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO otp_challenges (email,code_hash,expires_at) VALUES ('alternate@example.com',$1,now()+interval '10 minutes')").bind(code_hash("123456","dev-pepper")).execute(&pool).await.unwrap();
    let mut unlink = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    let login = tokio::spawn({
        let pool = pool.clone();
        async move {
            call(
                &pool,
                "POST",
                "/auth/otp/verify",
                None,
                Some(json!({"email":"alternate@example.com","code":"123456"})),
            )
            .await
        }
    });
    wait_for_lock(&pool).await;
    sqlx::query("DELETE FROM auth_identities WHERE user_id=$1 AND provider='google'")
        .bind(id)
        .execute(&mut *unlink)
        .await
        .unwrap();
    unlink.commit().await.unwrap();
    let (s, b) = login.await.unwrap();
    assert_eq!(s, 200, "{b}");
    assert_ne!(
        b["user"]["id"],
        id.to_string(),
        "已解除的不同信箱不再被視為舊帳號的所有權證明"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn auto_email_link_matches_manual_contact_policy(pool: PgPool) {
    let (id, _) = user(&pool, "line").await;
    sqlx::query("UPDATE users SET email='contact@example.com' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid,email) VALUES ($1,'google','alternate','alternate@example.com')").bind(id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO otp_challenges (email,code_hash,expires_at) VALUES ('alternate@example.com',$1,now()+interval '10 minutes')").bind(code_hash("123456","dev-pepper")).execute(&pool).await.unwrap();
    let (s, b) = call(
        &pool,
        "POST",
        "/auth/otp/verify",
        None,
        Some(json!({"email":"alternate@example.com","code":"123456"})),
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (StatusCode::CONFLICT, Some("EMAIL_MISMATCH"))
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM auth_identities WHERE user_id=$1 AND provider='email'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 0, "不得產生無法用該信箱重設密碼的 Email 登入方式");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_deletion_removes_link_challenges_and_security_recipients(pool: PgPool) {
    let (id, token) = user(&pool, "line").await;
    assert_eq!(
        call(
            &pool,
            "POST",
            "/me/identities/email/request",
            Some(&token),
            Some(json!({"email":"pending@example.com"}))
        )
        .await
        .0,
        202
    );
    sqlx::query("INSERT INTO notifications (user_id,channel,kind,payload,status) VALUES ($1,'email','auth.identity_unlinked','{\"to\":\"old@example.com\",\"provider\":\"email\"}','sent')").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            Some(&token),
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .0,
        200
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM otp_challenges WHERE link_user_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "連結至已刪帳號的待驗證信箱也須清除");
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM notifications WHERE user_id=$1 AND kind='auth.identity_unlinked'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        payload.get("to").is_none(),
        "安全信留存的 Email 也須去識別化"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn link_otp_request_cannot_commit_after_account_deletion(pool: PgPool) {
    let (id, token) = user(&pool, "line").await;
    let mut deletion = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM users WHERE id=$1 FOR UPDATE")
        .bind(id)
        .execute(&mut *deletion)
        .await
        .unwrap();
    let request = tokio::spawn({
        let pool = pool.clone();
        async move {
            call(
                &pool,
                "POST",
                "/me/identities/email/request",
                Some(&token),
                Some(json!({"email":"delete-race@example.com"})),
            )
            .await
        }
    });
    wait_for_lock(&pool).await;
    sqlx::query("UPDATE users SET deleted_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *deletion)
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(id)
        .execute(&mut *deletion)
        .await
        .unwrap();
    deletion.commit().await.unwrap();
    assert_eq!(request.await.unwrap().0, 401);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM otp_challenges WHERE link_user_id=$1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn self_unlink_revokes_other_sessions_only_on_success(pool: PgPool) {
    let (id, current) = user(&pool, "line").await;
    let other = "other-device";
    sqlx::query("INSERT INTO sessions (user_id,token_hash,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(id).bind(hash_token(other)).execute(&pool).await.unwrap();
    let (_, unrelated) = user(&pool, "line").await;
    assert_eq!(
        call(&pool, "DELETE", "/me/identities/line", Some(&current), None)
            .await
            .0,
        409
    );
    assert_eq!(call(&pool, "GET", "/me", Some(other), None).await.0, 200);
    sqlx::query("INSERT INTO auth_identities (user_id,provider,provider_uid) VALUES ($1,'google','unlink-google')").bind(id).execute(&pool).await.unwrap();
    assert_eq!(
        call(&pool, "DELETE", "/me/identities/line", Some(&current), None)
            .await
            .0,
        200
    );
    assert_eq!(call(&pool, "GET", "/me", Some(other), None).await.0, 401);
    assert_eq!(call(&pool, "GET", "/me", Some(&current), None).await.0, 200);
    assert_eq!(
        call(&pool, "GET", "/me", Some(&unrelated), None).await.0,
        200
    );
}
