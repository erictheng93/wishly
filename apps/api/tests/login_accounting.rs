mod common;

use axum::{extract::ConnectInfo, http::StatusCode};
use serde_json::json;
use sqlx::PgPool;
use std::net::SocketAddr;

async fn login(pool: &PgPool, email: &str, ip: &str) -> StatusCode {
    let mut request = common::request(
        "POST",
        "/api/v1/auth/login",
        &[],
        Some(json!({"email": email, "password": "wrong-password"})),
    );
    request.extensions_mut().insert(ConnectInfo(
        format!("{ip}:1234").parse::<SocketAddr>().unwrap(),
    ));
    common::call_request(&common::test_app(pool), request)
        .await
        .0
}

async fn failures(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM login_failures")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn database_errors_do_not_count_as_bad_credentials(pool: PgPool) {
    sqlx::raw_sql(
        "CREATE FUNCTION reject_global() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'injected rate limit failure'; END $$;
         CREATE TRIGGER reject_global BEFORE INSERT ON rate_limits
         FOR EACH ROW EXECUTE FUNCTION reject_global()",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        login(&pool, "failure@example.com", "1.1.1.1").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        failures(&pool).await,
        0,
        "rate-limit DB errors are not bad passwords"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn identity_database_error_does_not_count_as_bad_credentials(pool: PgPool) {
    sqlx::raw_sql("ALTER TABLE auth_identities RENAME TO hidden_identities")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        login(&pool, "failure@example.com", "1.1.1.1").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        failures(&pool).await,
        0,
        "identity DB errors are not bad passwords"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cancelled_login_does_not_leave_a_failure(pool: PgPool) {
    let mut blocker = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("LOCK TABLE auth_identities IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let p = pool.clone();
    let request = tokio::spawn(async move { login(&p, "cancel@example.com", "1.1.1.1").await });
    common::wait_for_blocked(&pool, pid, 1).await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());
    blocker.rollback().await.unwrap();
    assert_eq!(failures(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn locked_logins_never_insert_or_delete_even_under_a_burst(pool: PgPool) {
    sqlx::raw_sql(
        "INSERT INTO login_failures(email,ip)
         SELECT 'pair@example.com','1.1.1.1' FROM generate_series(1,5);
         INSERT INTO login_failures(email,ip)
         SELECT 'email@example.com','10.0.0.' || n FROM generate_series(1,20) n;
         INSERT INTO login_failures(email,ip)
         SELECT 'ip' || n || '@example.com','2.2.2.2' FROM generate_series(1,30) n;
         CREATE FUNCTION reject_failure_write() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'locked request attempted a failure write'; END $$;
         CREATE TRIGGER reject_failure_write BEFORE INSERT OR DELETE ON login_failures
         FOR EACH STATEMENT EXECUTE FUNCTION reject_failure_write()",
    )
    .execute(&pool)
    .await
    .unwrap();
    for (email, ip) in [
        ("pair@example.com", "1.1.1.1"),
        ("email@example.com", "3.3.3.3"),
        ("fresh@example.com", "2.2.2.2"),
    ] {
        let mut requests = vec![];
        for _ in 0..12 {
            let p = pool.clone();
            requests.push(tokio::spawn(async move { login(&p, email, ip).await }));
        }
        for request in requests {
            assert_eq!(request.await.unwrap(), StatusCode::TOO_MANY_REQUESTS);
        }
    }
    assert_eq!(failures(&pool).await, 55);
    let global_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM rate_limits WHERE key='login_global'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        global_rows, 0,
        "locked traffic must not write the global counter either"
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn concurrent_last_ip_slot_admits_only_one_bad_password(pool: PgPool) {
    sqlx::query("INSERT INTO login_failures(email,ip) SELECT 'seed' || n || '@example.com','1.1.1.1' FROM generate_series(1,29) n")
        .execute(&pool).await.unwrap();
    let mut requests = vec![];
    for n in 0..8 {
        let p = pool.clone();
        requests.push(tokio::spawn(async move {
            login(&p, &format!("last{n}@example.com"), "1.1.1.1").await
        }));
    }
    let mut unauthorized = 0;
    for request in requests {
        match request.await.unwrap() {
            StatusCode::UNAUTHORIZED => unauthorized += 1,
            StatusCode::TOO_MANY_REQUESTS => {}
            status => panic!("unexpected login status: {status}"),
        }
    }
    assert_eq!(unauthorized, 1, "最後一個 IP 名額只能放行一個密碼錯誤");
    assert_eq!(failures(&pool).await, 30);
}

/// 計數與寫入預約之間拉長（插入前睡 0.3 秒）：沒有 email / IP 交易鎖時兩個請求都會看到 29 而一起放行。
#[sqlx::test(migrations = "../../db/migrations")]
async fn reservation_lock_serializes_count_and_insert(pool: PgPool) {
    sqlx::raw_sql(
        "INSERT INTO login_failures(email,ip) SELECT 'seed' || n || '@example.com','1.1.1.1' FROM generate_series(1,29) n;
         CREATE FUNCTION slow_reserve() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN PERFORM pg_sleep(0.3); RETURN NEW; END $$;
         CREATE TRIGGER slow_reserve BEFORE INSERT ON login_failures FOR EACH ROW EXECUTE FUNCTION slow_reserve()",
    )
    .execute(&pool)
    .await
    .unwrap();
    let requests: Vec<_> = (0..2)
        .map(|n| {
            let p = pool.clone();
            tokio::spawn(async move { login(&p, &format!("race{n}@example.com"), "1.1.1.1").await })
        })
        .collect();
    let mut unauthorized = 0;
    for request in requests {
        if request.await.unwrap() == StatusCode::UNAUTHORIZED {
            unauthorized += 1;
        }
    }
    assert_eq!(unauthorized, 1, "預約時計數與寫入須在同一把鎖內");
}
