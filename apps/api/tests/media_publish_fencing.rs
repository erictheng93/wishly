//! Origin operations can outlive the HTTP client and must be fenced at commit.
mod common;
use axum::{
    body::{to_bytes, Body},
    extract::Request,
    http::{header, StatusCode},
    response::Response,
    routing::put,
    Router,
};
use common::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex, Notify};
use wishly_api::config::Config;

type Objects = Arc<Mutex<HashMap<String, (Vec<u8>, String, String)>>>;
type Gates = Arc<Mutex<HashMap<(String, bool), Arc<Notify>>>>;

async fn event(rx: &mut mpsc::UnboundedReceiver<(String, bool)>, path: &str, image: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while rx.recv().await.unwrap() != (path.to_owned(), image) {}
    })
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn late_origin_writes_cannot_restore_deleted_images(pool: PgPool) {
    let objects: Objects = Arc::default();
    let gates: Gates = Arc::default();
    let (started, mut starts) = mpsc::unbounded_channel();
    let (finished, mut finishes) = mpsc::unbounded_channel();
    let (deleted, mut deletes) = mpsc::unbounded_channel();
    let mut jpeg = Vec::new();
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(
            &mut std::io::Cursor::new(&mut jpeg),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let put_objects = objects.clone();
    let put_gates = gates.clone();
    let head_objects = objects.clone();
    let delete_objects = objects.clone();
    let mock = Router::new()
        .route("/media", put(|| async { StatusCode::OK }))
        .route(
            "/media/{*key}",
            put(move |request: Request| {
                let objects = put_objects.clone();
                let gates = put_gates.clone();
                let started = started.clone();
                let finished = finished.clone();
                async move {
                    let path = request.uri().path().to_string();
                    let headers = request.headers().clone();
                    let body = to_bytes(request.into_body(), 6 * 1024 * 1024)
                        .await
                        .unwrap()
                        .to_vec();
                    let ct = headers
                        .get(header::CONTENT_TYPE)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned();
                    let image = ct.starts_with("image/");
                    let gate = gates.lock().await.get(&(path.clone(), image)).cloned();
                    // The origin commits independently of a disconnected/timed-out client.
                    tokio::spawn(async move {
                        started.send((path.clone(), image)).unwrap();
                        if let Some(gate) = gate {
                            gate.notified().await;
                        }
                        let mut objects = objects.lock().await;
                        let current = objects.get(&path);
                        let rejected = headers
                            .get("if-none-match")
                            .is_some_and(|v| v == "*" && current.is_some())
                            || headers.get("if-match").is_some_and(|v| {
                                current.is_none_or(|(_, tag, _)| v.to_str().unwrap() != tag)
                            });
                        let mut response = Response::builder().status(if rejected {
                            StatusCode::PRECONDITION_FAILED
                        } else {
                            StatusCode::OK
                        });
                        if !rejected {
                            let etag = format!("\"{}\"", hex::encode(Sha256::digest(&body)));
                            objects.insert(path.clone(), (body, etag.clone(), ct));
                            response = response.header(header::ETAG, etag);
                        }
                        finished.send((path, image)).unwrap();
                        response.body(Body::empty()).unwrap()
                    })
                    .await
                    .unwrap()
                }
            })
            .get(move || {
                let jpeg = jpeg.clone();
                async move { jpeg }
            })
            .head(move |request: Request| {
                let objects = head_objects.clone();
                async move {
                    let objects = objects.lock().await;
                    match objects.get(request.uri().path()) {
                        Some((_, etag, ct)) => Response::builder()
                            .header(header::ETAG, etag)
                            .header(header::CONTENT_TYPE, ct)
                            .body(Body::empty())
                            .unwrap(),
                        None => Response::builder().status(404).body(Body::empty()).unwrap(),
                    }
                }
            })
            .delete(move |request: Request| {
                let objects = delete_objects.clone();
                let deleted = deleted.clone();
                async move {
                    let path = request.uri().path().to_owned();
                    objects.lock().await.remove(&path);
                    deleted.send(path).unwrap();
                    StatusCode::NO_CONTENT
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::dev();
    cfg.s3.endpoint = format!("http://{}", listener.local_addr().unwrap());
    cfg.s3.bucket = "media".into();
    wishly_api::config::init(cfg);
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    for mode in ["normal", "retry", "cancel", "timeout", "aba"] {
        let owner = user(&pool, mode).await;
        let response = call(
            &pool,
            "POST",
            "/uploads/presign",
            &ck(&owner.token),
            Some(json!({"purpose":"item","content_type":"image/jpeg","content_length":1})),
        )
        .await;
        assert_eq!(response.0, StatusCode::OK);
        let key = response.2["object_key"].as_str().unwrap().to_owned();
        let path = format!("/media/{key}");
        let image_gate = Arc::new(Notify::new());
        gates
            .lock()
            .await
            .insert((path.clone(), true), image_gate.clone());
        let confirm_pool = pool.clone();
        let confirm_token = owner.token.clone();
        let confirm_key = key.clone();
        let confirm = tokio::spawn(async move {
            call(
                &confirm_pool,
                "POST",
                "/uploads/confirm",
                &ck(&confirm_token),
                Some(json!({"object_key":confirm_key})),
            )
            .await
        });
        event(&mut starts, &path, true).await;
        if mode == "normal" {
            image_gate.notify_one();
            assert_eq!(confirm.await.unwrap().0, StatusCode::OK);
            let before = objects.lock().await.get(&path).unwrap().clone();
            assert!(image::load_from_memory(&before.0).is_ok());
            // If staging remains/reappears, publication reuses the completed image without overwriting it.
            assert_eq!(
                call(
                    &pool,
                    "POST",
                    "/uploads/confirm",
                    &ck(&owner.token),
                    Some(json!({"object_key":key}))
                )
                .await
                .0,
                StatusCode::OK
            );
            assert_eq!(objects.lock().await.get(&path).unwrap(), &before);
            continue;
        }
        if mode == "timeout" {
            let response = tokio::time::timeout(Duration::from_secs(12), confirm)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(response.0, StatusCode::BAD_GATEWAY);
        } else {
            confirm.abort();
            assert!(confirm.await.unwrap_err().is_cancelled());
        }
        if mode == "retry" {
            let retry_pool = pool.clone();
            let retry_token = owner.token.clone();
            let retry_key = key.clone();
            let retry = tokio::spawn(async move {
                call(
                    &retry_pool,
                    "POST",
                    "/uploads/confirm",
                    &ck(&retry_token),
                    Some(json!({"object_key":retry_key})),
                )
                .await
            });
            event(&mut starts, &path, true).await;
            image_gate.notify_waiters();
            assert_eq!(retry.await.unwrap().0, StatusCode::OK);
            assert!(image::load_from_memory(&objects.lock().await.get(&path).unwrap().0).is_ok());
            continue;
        }
        let mut reservation_gate = None;
        if mode == "aba" {
            event(&mut finishes, &path, false).await;
            let gate = Arc::new(Notify::new());
            gates
                .lock()
                .await
                .insert((path.clone(), false), gate.clone());
            let second_pool = pool.clone();
            let second_token = owner.token.clone();
            let second_key = key.clone();
            let second = tokio::spawn(async move {
                call(
                    &second_pool,
                    "POST",
                    "/uploads/confirm",
                    &ck(&second_token),
                    Some(json!({"object_key":second_key})),
                )
                .await
            });
            event(&mut starts, &path, false).await;
            second.abort();
            assert!(second.await.unwrap_err().is_cancelled());
            reservation_gate = Some(gate);
        }
        assert_eq!(
            call(
                &pool,
                "DELETE",
                "/me",
                &ck(&owner.token),
                Some(json!({"confirm":"DELETE"}))
            )
            .await
            .0,
            StatusCode::OK
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while deletes.recv().await.unwrap() != path {}
        })
        .await
        .unwrap();
        if let Some(gate) = reservation_gate {
            gate.notify_one();
            event(&mut finishes, &path, false).await;
        }
        image_gate.notify_one();
        event(&mut finishes, &path, true).await;
        assert!(
            objects
                .lock()
                .await
                .get(&path)
                .is_none_or(|(_, _, ct)| !ct.starts_with("image/")),
            "{mode}: late PUT restored deleted image"
        );
    }
    server.abort();
}
