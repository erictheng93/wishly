//! Origin deletion must precede purging the exact public media URLs, including origin 404s.
mod common;
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    routing::{delete, post},
    Json, Router,
};
use common::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;
use uuid::Uuid;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_purges_media_after_origin_delete_or_not_found(pool: PgPool) {
    let deleted = Arc::new(Mutex::new(HashSet::new()));
    let cached = Arc::new(Mutex::new(HashSet::new()));
    let (seen, mut received) = mpsc::unbounded_channel();
    let origin = deleted.clone();
    let cache = deleted.clone();
    let edge = cached.clone();
    let mock = Router::new()
        .route(
            "/media/{*key}",
            delete(move |request: Request| {
                let deleted = origin.clone();
                async move {
                    let key = request
                        .uri()
                        .path()
                        .strip_prefix("/media/")
                        .unwrap()
                        .to_owned();
                    let status = if key.starts_with("avatars/") {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::NO_CONTENT
                    };
                    deleted.lock().unwrap().insert(key);
                    status
                }
            }),
        )
        .route(
            "/zones/zone1/purge_cache",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let seen = seen.clone();
                let deleted = cache.clone();
                let cached = edge.clone();
                async move {
                    assert_eq!(headers["authorization"], "Bearer test-purge-token");
                    for field in ["files", "prefixes"] {
                        assert!(body[field]
                            .as_array()
                            .is_none_or(|values| values.len() <= 100));
                    }
                    for (field, base) in [
                        ("files", "https://media.example.test/"),
                        ("prefixes", "media.example.test/"),
                    ] {
                        for value in body[field].as_array().into_iter().flatten() {
                            if let Some(key) = value.as_str().unwrap().strip_prefix(base) {
                                assert!(
                                    deleted.lock().unwrap().contains(key),
                                    "purge preceded origin delete: {key}"
                                );
                                let url = format!("https://media.example.test/{key}");
                                cached.lock().unwrap().retain(|entry: &String| {
                                    if field == "files" {
                                        entry != &url
                                    } else {
                                        !entry.starts_with(&url)
                                    }
                                });
                                seen.send(url).unwrap();
                            }
                        }
                    }
                    Json(json!({"success": true}))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let mut cfg = Config::dev();
    cfg.s3.endpoint = address.clone();
    cfg.s3.bucket = "media".into();
    cfg.s3.public_base = "https://media.example.test/".into();
    cfg.cf_api_base = address;
    cfg.cf_zone_id = Some("zone1".into());
    cfg.cf_purge_token = Some("test-purge-token".into());
    wishly_api::config::init(cfg);
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let owner = user(&pool, "小米").await;
    let list = wishlist(&pool, owner.id, ListOpts::default()).await;
    let cover = format!("covers/{}.jpg", Uuid::new_v4());
    let avatar = format!("avatars/{}.png", Uuid::new_v4());
    for key in [&cover, &avatar] {
        cached.lock().unwrap().extend([
            format!("https://media.example.test/{key}"),
            format!("https://media.example.test/{key}?variant=1"),
        ]);
    }
    sqlx::query("UPDATE wishlists SET cover_image_key=$2, cover_image_status='ready' WHERE id=$1")
        .bind(list.id)
        .bind(&cover)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET avatar_key=$2 WHERE id=$1")
        .bind(owner.id)
        .bind(&avatar)
        .execute(&pool)
        .await
        .unwrap();
    let (status, _, body) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&owner.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut actual = Vec::new();
    for _ in 0..2 {
        actual.push(
            tokio::time::timeout(Duration::from_secs(5), received.recv())
                .await
                .expect("missing public media URL purge")
                .unwrap(),
        );
    }
    actual.sort();
    let mut expected = vec![
        format!("https://media.example.test/{cover}"),
        format!("https://media.example.test/{avatar}"),
    ];
    expected.sort();
    assert_eq!(actual, expected);
    assert!(
        cached.lock().unwrap().is_empty(),
        "query-string cache variants survived media deletion"
    );
    // Larger accounts must purge every batch without exceeding Cloudflare's limit.
    let keys: Vec<String> = (0..205)
        .map(|_| format!("items/{}.jpg", Uuid::new_v4()))
        .collect();
    deleted.lock().unwrap().extend(keys.clone());
    let mut expected: Vec<String> = keys
        .iter()
        .map(|key| format!("https://media.example.test/{key}"))
        .collect();
    cached
        .lock()
        .unwrap()
        .extend(expected.iter().map(|url| format!("{url}?variant=1")));
    wishly_api::cdn::purge_media(keys);
    let mut actual = Vec::new();
    for _ in 0..205 {
        actual.push(
            tokio::time::timeout(Duration::from_secs(5), received.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    assert!(cached.lock().unwrap().is_empty());
    server.abort();
}
