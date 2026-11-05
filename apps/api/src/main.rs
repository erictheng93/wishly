use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use wishly_api::{app, AppState};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

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
    // DB_POOL_MAX：連線池上限（預設 10）。1 vCPU 主機上連線多不會更快，只會讓 Postgres 行程互搶 CPU、增加切換。
    let max_conns = std::env::var("DB_POOL_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);
    let pool = PgPoolOptions::new()
        .max_connections(max_conns)
        .acquire_timeout(std::time::Duration::from_secs(3)) // 6.4：取不到連線 3 秒即回 503，不讓請求堆積
        .connect_with(opts)
        .await
        .expect("db");
    sqlx::migrate!("../../db/migrations")
        .run(&pool)
        .await
        .expect("migrate");
    let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
    wishly_api::session::spawn_cache_listener(pool.clone());
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
