//! 品牌館圖片（purpose=catalog）：只有 staff 能上傳，且 confirm 能實際發佈（catalog 圖不記 user_media）。
//! config 是行程全域單例，S3 mock 需要獨占，故自成一個測試檔。
mod common;
use axum::{extract::Request, http::StatusCode, routing::put, Router};
use common::{call, ck, staff, user};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::mpsc;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn only_staff_can_upload_catalog_images_and_confirm_publishes(pool: PgPool) {
    let mut image = Vec::new();
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(
            &mut std::io::Cursor::new(&mut image),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let (puts, mut got_put) = mpsc::unbounded_channel();
    let mock = Router::new()
        .route("/media", put(|| async { StatusCode::OK }))
        .route(
            "/media/{*key}",
            put(move |request: Request| {
                let puts = puts.clone();
                async move {
                    puts.send(request.uri().path().to_string()).unwrap();
                    ([("etag", "\"e\"")], StatusCode::OK)
                }
            })
            .get(move || {
                let image = image.clone();
                async move { image }
            })
            .delete(|| async { StatusCode::NO_CONTENT }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::dev();
    cfg.s3.endpoint = format!("http://{}", listener.local_addr().unwrap());
    cfg.s3.bucket = "media".into();
    wishly_api::config::init(cfg);
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let req = json!({"purpose":"catalog","content_type":"image/jpeg","content_length":1});
    let member = user(&pool, "Member").await;
    let r = call(
        &pool,
        "POST",
        "/uploads/presign",
        &ck(&member.token),
        Some(req.clone()),
    )
    .await;
    assert_eq!(r.0, StatusCode::FORBIDDEN, "{}", r.2);
    let r = call(
        &pool,
        "POST",
        "/uploads/confirm",
        &ck(&member.token),
        Some(json!({"object_key": format!("catalog/{}.jpg", uuid::Uuid::new_v4())})),
    )
    .await;
    assert_eq!(r.0, StatusCode::FORBIDDEN, "{}", r.2);

    let op = staff(&pool, "Ops").await;
    let r = call(&pool, "POST", "/uploads/presign", &ck(&op.token), Some(req)).await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    let key = r.2["object_key"].as_str().unwrap().to_owned();
    assert!(key.starts_with("catalog/"));
    let r = call(
        &pool,
        "POST",
        "/uploads/confirm",
        &ck(&op.token),
        Some(json!({"object_key": key})),
    )
    .await;
    assert_eq!(r.0, StatusCode::OK, "{}", r.2);
    assert_eq!(r.2["image_status"], "ready");
    // 預留 PUT + 實際內容 PUT 都打到公開 key
    let paths: Vec<String> = std::iter::from_fn(|| got_put.try_recv().ok()).collect();
    assert_eq!(paths, vec![format!("/media/{key}"); 2]);
    let tracked: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_media WHERE object_key=$1)")
            .bind(&key)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!tracked, "catalog 圖不可掛在個人帳號下（刪帳號會被清掉）");
    server.abort();
}
