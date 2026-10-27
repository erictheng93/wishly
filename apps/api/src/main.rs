use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use wishly_api::{app, AppState};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cfg = wishly_api::config::Config::from_env();
    if let Some(w) = cfg.bind_warning() {
        tracing::warn!("{w}");
        eprintln!("WARN: {w}");
    }
    let bind = cfg.bind.clone();
    wishly_api::config::init(cfg);
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    // Q11：交易開著卻閒置（忘了 commit 等 bug）超過 30 秒即由 DB 中止，避免長時間占著列鎖
    let opts: PgConnectOptions = url.parse().expect("DATABASE_URL");
    let opts = opts.options([("idle_in_transaction_session_timeout", "30s")]);
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(std::time::Duration::from_secs(3)) // 6.4：取不到連線 3 秒即回 503，不讓請求堆積
        .connect_with(opts)
        .await
        .expect("db");
    sqlx::migrate!("../../db/migrations")
        .run(&pool)
        .await
        .expect("migrate");
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    wishly_api::notify::spawn_worker(pool.clone());
    wishly_api::ratelimit::spawn_cleanup(pool.clone());
    wishly_api::relief_events::spawn_permit_expiry(pool.clone());
    wishly_api::uploads::spawn_media_worker(pool.clone());
    wishly_api::topup::spawn_expire(pool.clone());
    axum::serve(
        listener,
        app(AppState { pool }).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
