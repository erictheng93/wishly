//! Endpoint validation and ownership of incoming upload objects.
mod common;
use axum::{
    extract::Request,
    http::StatusCode,
    routing::{get, put},
    Router,
};
use common::{call, ck, user, TestUser};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::mpsc;
use wishly_api::{config::Config, uploads::MAX_BYTES};

#[sqlx::test(migrations = "../../db/migrations")]
async fn presign_whitelists_size_type_purpose_and_binds_incoming_to_user(pool: PgPool) {
    let (seen, mut received) = mpsc::unbounded_channel();
    let mock = Router::new()
        .route("/media", put(|| async { StatusCode::OK }))
        .route(
            "/media/{*key}",
            get(move |request: Request| {
                let seen = seen.clone();
                async move {
                    seen.send(request.uri().path().to_string()).unwrap();
                    StatusCode::NOT_FOUND
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::dev();
    cfg.s3.endpoint = format!("http://{}", listener.local_addr().unwrap());
    cfg.s3.bucket = "media".into();
    cfg.s3.public_base = "https://cdn.example.test/media".into();
    wishly_api::config::init(cfg);
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let TestUser { id: alice, token } = user(&pool, "Alice").await;
    let TestUser {
        id: bob,
        token: bob_token,
    } = user(&pool, "Bob").await;
    let valid = json!({"purpose":"item","content_type":"image/png","content_length":1});
    assert_eq!(
        call(&pool, "POST", "/uploads/presign", &[], Some(valid.clone()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    for invalid in [
        json!({"purpose":"avatar","content_type":"image/png","content_length":1}),
        json!({"purpose":"item","content_type":"image/gif","content_length":1}),
        json!({"purpose":"item","content_type":"image/svg+xml","content_length":1}),
        json!({"purpose":"item","content_type":"IMAGE/PNG","content_length":1}),
        json!({"purpose":"item","content_type":"image/png","content_length":0}),
        json!({"purpose":"item","content_type":"image/png","content_length":MAX_BYTES+1}),
    ] {
        let response = call(
            &pool,
            "POST",
            "/uploads/presign",
            &ck(&token),
            Some(invalid),
        )
        .await;
        assert_eq!(
            response.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{}",
            response.2
        );
        assert!(response.2.get("upload_url").is_none());
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM rate_limits WHERE key=$1")
            .bind(format!("presign:{alice}"))
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    for (purpose, content_type, extension, length) in [
        ("item", "image/png", "png", 1),
        ("cover", "image/jpeg", "jpg", MAX_BYTES),
        ("item", "image/webp", "webp", 2),
    ] {
        let response = call(
            &pool,
            "POST",
            "/uploads/presign",
            &ck(&token),
            Some(json!({"purpose":purpose,"content_type":content_type,"content_length":length})),
        )
        .await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.2);
        let body = response.2;
        let key = body["object_key"].as_str().unwrap();
        assert!(wishly_api::uploads::valid_key(key));
        assert!(key.ends_with(&format!(".{extension}")));
        assert!(key.starts_with(if purpose == "cover" {
            "covers/"
        } else {
            "items/"
        }));
        let url = reqwest::Url::parse(body["upload_url"].as_str().unwrap()).unwrap();
        assert_eq!(url.path(), format!("/media/incoming/{alice}/{key}"));
        let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(
            query["X-Amz-SignedHeaders"],
            "content-length;content-type;host"
        );
        assert_eq!(query["X-Amz-Expires"], "300");
        assert_eq!(body["headers"]["Content-Type"], content_type);
        assert_eq!(body["headers"]["Content-Length"], length.to_string());
        assert_eq!(body["method"], "PUT");
        assert_eq!(
            body["public_url"],
            format!("https://cdn.example.test/media/{key}")
        );
        // A second user cannot confirm Alice's staged object: GET must stay in Bob's namespace.
        let response = call(
            &pool,
            "POST",
            "/uploads/confirm",
            &ck(&bob_token),
            Some(json!({"object_key":key})),
        )
        .await;
        assert_eq!(response.0, StatusCode::NOT_FOUND);
        assert_eq!(
            received.try_recv().unwrap(),
            format!("/media/incoming/{bob}/{key}")
        );
    }
    let used: i32 = sqlx::query_scalar("SELECT count FROM rate_limits WHERE key=$1")
        .bind(format!("presign:{alice}"))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(used, 3);
    server.abort();
    common::assert_all_reconciled(&pool).await;
}
