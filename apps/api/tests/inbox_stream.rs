//! 站內通知即時串流：本人收得到訊號、別人收不到、未登入 401。
//! dashboard 的 BUS 是綁第一個 pool 的全域單例，所以這個檔案只放一個 #[sqlx::test]。
mod common;
use common::*;
use sqlx::PgPool;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn open(addr: std::net::SocketAddr, token: Option<&str>) -> tokio::net::TcpStream {
    let mut c = tokio::net::TcpStream::connect(addr).await.unwrap();
    let ck = token.map(|t| format!("Cookie: ws_session={t}\r\n")).unwrap_or_default();
    c.write_all(format!("GET /api/v1/me/notifications/stream HTTP/1.1\r\nHost: x\r\n{ck}\r\n").as_bytes()).await.unwrap();
    c
}

/// 讀到 `needle` 為止（或逾時）；回傳目前累積的內容
async fn read_until(c: &mut tokio::net::TcpStream, needle: &str, wait: Duration) -> String {
    let mut acc = String::new();
    let mut buf = [0u8; 1024];
    let _ = tokio::time::timeout(wait, async {
        while !acc.contains(needle) {
            match c.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => acc.push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    })
    .await;
    acc
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn stream_signals_only_the_owner(pool: PgPool) {
    wishly_api::config::init(wishly_api::config::Config::dev());
    let (a, b) = (user(&pool, "a").await, user(&pool, "b").await);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = test_app(&pool).into_make_service_with_connect_info::<std::net::SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // 未登入 401
    let mut anon = open(addr, None).await;
    assert!(read_until(&mut anon, "\r\n", Duration::from_secs(5)).await.starts_with("HTTP/1.1 401"));

    let mut ca = open(addr, Some(&a.token)).await;
    let mut cb = open(addr, Some(&b.token)).await;
    // retry 事件出現代表已完成訂閱
    assert!(read_until(&mut ca, "retry", Duration::from_secs(5)).await.contains("text/event-stream"));
    read_until(&mut cb, "retry", Duration::from_secs(5)).await;

    wishly_api::dashboard::notify(&pool, a.id).await.unwrap();
    let got = read_until(&mut ca, "event: inbox", Duration::from_secs(5)).await;
    assert!(got.contains("event: inbox") && got.contains("data: {}"), "{got}");
    let other = read_until(&mut cb, "event: inbox", Duration::from_millis(500)).await;
    assert!(!other.contains("event: inbox"), "別人不該收到：{other}");
}
