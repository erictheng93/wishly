use sqlx::postgres::PgPoolOptions;
use wishsync_api::{app, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cfg = wishsync_api::config::Config::from_env();
    if let Some(w) = cfg.bind_warning() {
        tracing::warn!("{w}");
        eprintln!("WARN: {w}");
    }
    let bind = cfg.bind.clone();
    wishsync_api::config::init(cfg);
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .expect("db");
    sqlx::migrate!("../../db/migrations")
        .run(&pool)
        .await
        .expect("migrate");
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    wishsync_api::notify::spawn_worker(pool.clone());
    wishsync_api::ratelimit::spawn_cleanup(pool.clone());
    // Cloud Run 縮容／換版送 SIGTERM，預設 10 秒後強殺：先停止接新連線並等進行中請求完成，
    // 但 SSE 單連線最長 30 分鐘，所以以 SHUTDOWN_GRACE 為上限，逾時直接結束（客戶端會依 retry 重連）。
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(
        listener,
        app(AppState { pool }).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = stopped.await;
    });
    tokio::select! {
        r = async { server.await } => r.unwrap(),
        _ = async {
            shutdown_signal().await;
            tracing::info!("收到終止訊號，開始優雅關閉");
            let _ = stop.send(());
            tokio::time::sleep(SHUTDOWN_GRACE).await;
        } => tracing::warn!("優雅關閉逾時，強制結束（仍有未完成連線）"),
    }
}

/// 低於 Cloud Run 預設的 10 秒終止寬限期。
const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(8);

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}
