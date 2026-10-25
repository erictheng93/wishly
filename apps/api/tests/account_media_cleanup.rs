//! DELETE /me 後清掉使用者自己的 R2 物件；仍被他人引用的 key 不可刪。
mod common;
use axum::{
    extract::Request,
    http::StatusCode,
    routing::{delete, put},
    Router,
};
use common::*;
use serde_json::json;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::sync::{mpsc, Mutex, Notify};
use uuid::Uuid;
use wishly_api::config::Config;

#[sqlx::test(migrations = "../../db/migrations")]
async fn delete_me_removes_unshared_media_only(pool: PgPool) {
    let (seen, mut received) = mpsc::unbounded_channel();
    let delete_gate = Arc::new(Mutex::new(None::<(String, Arc<Notify>)>));
    let gate = delete_gate.clone();
    let get_gate = Arc::new(Mutex::new(None::<(String, Arc<Notify>)>));
    let put_gate = Arc::new(Mutex::new(None::<(String, Arc<Notify>)>));
    let get_barrier = get_gate.clone();
    let put_barrier = put_gate.clone();
    let (gets, mut got_get) = mpsc::unbounded_channel();
    let (puts, mut got_put) = mpsc::unbounded_channel();
    let mut image = Vec::new();
    image::DynamicImage::new_rgb8(1, 1)
        .write_to(
            &mut std::io::Cursor::new(&mut image),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
    let mock = Router::new()
        .route(
            "/media/{*key}",
            delete(move |request: Request| {
                let seen = seen.clone();
                let gate = gate.clone();
                async move {
                    let path = request.uri().path().to_string();
                    seen.send(path.clone()).unwrap();
                    let wait = gate
                        .lock()
                        .await
                        .as_ref()
                        .filter(|(key, _)| path.ends_with(key))
                        .map(|(_, release)| release.clone());
                    if let Some(release) = wait {
                        release.notified().await;
                    }
                    StatusCode::NO_CONTENT
                }
            })
            .get(move |request: Request| {
                let gate = get_barrier.clone();
                let seen = gets.clone();
                let image = image.clone();
                async move {
                    let path = request.uri().path().to_string();
                    seen.send(path.clone()).unwrap();
                    let wait = gate
                        .lock()
                        .await
                        .as_ref()
                        .filter(|(key, _)| path.ends_with(key))
                        .map(|(_, notify)| notify.clone());
                    if let Some(notify) = wait {
                        notify.notified().await;
                    }
                    image
                }
            })
            .put(move |request: Request| {
                let gate = put_barrier.clone();
                let seen = puts.clone();
                async move {
                    let path = request.uri().path().to_string();
                    if request
                        .headers()
                        .get("content-type")
                        .is_some_and(|v| v == "application/octet-stream")
                    {
                        return (
                            [("etag", format!("\"{}\"", Uuid::new_v4()))],
                            StatusCode::OK,
                        );
                    }
                    seen.send(path.clone()).unwrap();
                    let wait = gate
                        .lock()
                        .await
                        .as_ref()
                        .filter(|(key, _)| path.ends_with(key))
                        .map(|(_, notify)| notify.clone());
                    if let Some(notify) = wait {
                        notify.notified().await;
                    }
                    ([("etag", "\"image\"".to_owned())], StatusCode::OK)
                }
            }),
        )
        .route("/media", put(|| async { StatusCode::OK }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut cfg = Config::dev();
    cfg.s3.endpoint = format!("http://{}", listener.local_addr().unwrap());
    cfg.s3.bucket = "media".into();
    wishly_api::config::init(cfg);
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });

    let TestUser { id: uid, token } = user(&pool, "小米").await;
    let TestUser {
        id: other,
        token: other_token,
    } = user(&pool, "o").await;
    let k = |dir: &str| format!("{dir}/{}.jpg", Uuid::new_v4());
    let (cover, item_own, shared, avatar) = (k("covers"), k("items"), k("items"), k("avatars"));
    let (new_cover, new_item) = (k("covers"), k("items"));
    let mut replaced_item = Uuid::nil();
    let mine = wishlist(&pool, uid, ListOpts::default()).await;
    let theirs = wishlist(&pool, other, ListOpts::default()).await;
    sqlx::query("UPDATE wishlists SET cover_image_key=$2, cover_image_status='ready' WHERE id=$1")
        .bind(mine.id)
        .bind(&cover)
        .execute(&pool)
        .await
        .unwrap();
    for (wl, key) in [
        (mine.id, &item_own),
        (mine.id, &shared),
        (theirs.id, &shared),
    ] {
        let it = item(&pool, wl, "x", 1).await;
        if key == &item_own {
            replaced_item = it;
        }
        sqlx::query("UPDATE wishlist_items SET image_key=$2, image_status='ready' WHERE id=$1")
            .bind(it)
            .bind(key)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE users SET avatar_key=$2 WHERE id=$1")
        .bind(uid)
        .bind(&avatar)
        .execute(&pool)
        .await
        .unwrap();

    for (uri, body) in [
        (
            format!("/wishlists/{}", mine.id),
            json!({"cover_image_key":new_cover}),
        ),
        (
            format!("/items/{replaced_item}"),
            json!({"image_key":new_item}),
        ),
    ] {
        let response = call(&pool, "PATCH", &uri, &ck(&token), Some(body)).await;
        assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    }

    // Media transactions must reuse their connection even when the pool has one slot.
    let single = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    for (method, uri, body) in [
        (
            "PATCH",
            format!("/wishlists/{}", mine.id),
            json!({"title":"Updated"}),
        ),
        (
            "POST",
            format!("/wishlists/{}/items", mine.id),
            json!({"title":"New item"}),
        ),
    ] {
        let response = tokio::time::timeout(
            Duration::from_secs(5),
            call(&single, method, &uri, &ck(&token), Some(body)),
        )
        .await
        .expect("media mutation must not need a second pool connection");
        assert!(response.0.is_success(), "{}", response.2);
    }
    single.close().await;

    let (s, _, _) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&token),
        Some(json!({"confirm":"DELETE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let mut got = Vec::new();
    for _ in 0..5 {
        let p = tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .expect("替換前後的圖片都應刪除")
            .unwrap();
        got.push(p);
    }
    got.sort();
    let mut want: Vec<String> = [&cover, &new_cover, &item_own, &new_item, &avatar]
        .iter()
        .map(|k| format!("/media/{k}"))
        .collect();
    want.sort();
    assert_eq!(got, want);
    // 他人仍引用的 key 不可被刪
    assert!(
        tokio::time::timeout(Duration::from_millis(500), received.recv())
            .await
            .is_err()
    );
    let response = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&other_token),
        Some(json!({"confirm":"DELETE"})),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .expect("最後一個分享者刪帳後應刪除共用圖片")
            .unwrap(),
        format!("/media/{shared}")
    );
    // Deletion wins: while the origin DELETE is in flight, attachment must wait,
    // then reject the retired key rather than leave a live broken reference.
    let owner = user(&pool, "Race owner").await;
    let recipient = user(&pool, "Race recipient").await;
    let owned = wishlist(&pool, owner.id, ListOpts::default()).await;
    let destination = wishlist(&pool, recipient.id, ListOpts::default()).await;
    let race_key = k("covers");
    assert_eq!(
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{}", owned.id),
            &ck(&owner.token),
            Some(json!({"cover_image_key":race_key}))
        )
        .await
        .0,
        StatusCode::OK
    );
    let release = Arc::new(Notify::new());
    *delete_gate.lock().await = Some((race_key.clone(), release.clone()));
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
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap(),
        format!("/media/{race_key}")
    );
    // origin DELETE 還卡著：退役已先提交，掛圖不必等刪除完成就直接被拒
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        call(
            &pool,
            "PATCH",
            &format!("/wishlists/{}", destination.id),
            &ck(&recipient.token),
            Some(json!({"cover_image_key":race_key})),
        ),
    )
    .await
    .expect("attachment must not wait for the origin DELETE");
    release.notify_one();
    assert_eq!(response.0, StatusCode::CONFLICT, "{}", response.2);
    let attached: Option<String> =
        sqlx::query_scalar("SELECT cover_image_key FROM wishlists WHERE id=$1")
            .bind(destination.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(attached.is_none());
    *delete_gate.lock().await = None;

    // Attachment wins: cleanup sees the committed second user's reference.
    let first = user(&pool, "First").await;
    let second = user(&pool, "Second").await;
    let first_list = wishlist(&pool, first.id, ListOpts::default()).await;
    let second_list = wishlist(&pool, second.id, ListOpts::default()).await;
    let kept = k("covers");
    for (list, token) in [
        (first_list.id, &first.token),
        (second_list.id, &second.token),
    ] {
        assert_eq!(
            call(
                &pool,
                "PATCH",
                &format!("/wishlists/{list}"),
                &ck(token),
                Some(json!({"cover_image_key":kept}))
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            &ck(&first.token),
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), received.recv())
            .await
            .is_err()
    );
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            &ck(&second.token),
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap(),
        format!("/media/{kept}")
    );
    // A confirm that downloaded before deletion cannot publish after deletion.
    let uploading = user(&pool, "Downloading").await;
    let response = call(
        &pool,
        "POST",
        "/uploads/presign",
        &ck(&uploading.token),
        Some(json!({"purpose":"item","content_type":"image/jpeg","content_length":1})),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    let late_key = response.2["object_key"].as_str().unwrap().to_owned();
    let release = Arc::new(Notify::new());
    *get_gate.lock().await = Some((late_key.clone(), release.clone()));
    let confirm_pool = pool.clone();
    let confirm_token = uploading.token.clone();
    let confirm_key = late_key.clone();
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
    assert!(tokio::time::timeout(Duration::from_secs(5), got_get.recv())
        .await
        .unwrap()
        .unwrap()
        .ends_with(&late_key));
    assert_eq!(
        call(
            &pool,
            "DELETE",
            "/me",
            &ck(&uploading.token),
            Some(json!({"confirm":"DELETE"}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap(),
        format!("/media/{late_key}")
    );
    release.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), confirm)
            .await
            .unwrap()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert!(got_put.try_recv().is_err(), "late confirm must not publish");
    *get_gate.lock().await = None;

    // Conversely, an in-flight public PUT completes before account deletion.
    let uploading = user(&pool, "Publishing").await;
    let response = call(
        &pool,
        "POST",
        "/uploads/presign",
        &ck(&uploading.token),
        Some(json!({"purpose":"item","content_type":"image/jpeg","content_length":1})),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    let active_key = response.2["object_key"].as_str().unwrap().to_owned();
    let release = Arc::new(Notify::new());
    *put_gate.lock().await = Some((active_key.clone(), release.clone()));
    let confirm_pool = pool.clone();
    let confirm_token = uploading.token.clone();
    let confirm_key = active_key.clone();
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
    assert!(tokio::time::timeout(Duration::from_secs(5), got_put.recv())
        .await
        .unwrap()
        .unwrap()
        .ends_with(&active_key));
    let upload_owner = uploading.id;
    let delete_pool = pool.clone();
    let mut deleting = tokio::spawn(async move {
        call(
            &delete_pool,
            "DELETE",
            "/me",
            &ck(&uploading.token),
            Some(json!({"confirm":"DELETE"})),
        )
        .await
    });
    // 刪帳號不等 origin PUT：退役先提交，慢的 PUT 不會卡住刪除
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), &mut deleting)
            .await
            .expect("account deletion must not wait for an in-flight origin PUT")
            .unwrap()
            .0,
        StatusCode::OK
    );
    // 假 origin 不檢查 If-Match：等同 PUT 晚於刪除才落地；發佈後複查看到已退役就補刪並回 409
    wait_until("key retired by account deletion", || {
        let (pool, key) = (pool.clone(), active_key.clone());
        async move {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM user_media WHERE object_key=$1 AND deleted_at IS NOT NULL)")
                .bind(key).fetch_one(&pool).await.unwrap()
        }
    })
    .await;
    release.notify_one();
    let response = tokio::time::timeout(Duration::from_secs(5), confirm)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        response.0,
        StatusCode::CONFLICT,
        "發佈後複查須補刪已退役圖片: {}",
        response.2
    );
    let mut removed = Vec::new();
    for _ in 0..3 {
        removed.push(
            tokio::time::timeout(Duration::from_secs(5), received.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    removed.sort();
    let mut expected = vec![
        format!("/media/{active_key}"),
        format!("/media/{active_key}"),
        format!("/media/incoming/{}/{active_key}", upload_owner),
    ];
    expected.sort();
    assert_eq!(
        removed, expected,
        "刪帳號與發佈後複查各刪一次，暫存檔也清掉"
    );
    // 發佈圍欄未過期前不算刪完；過期後的 drain 再刪一次才標記 purged_at
    let purged = || async {
        sqlx::query_scalar::<_, bool>(
            "SELECT bool_and(purged_at IS NOT NULL) FROM user_media WHERE object_key=$1",
        )
        .bind(&active_key)
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert!(!purged().await, "圍欄未過期不可標記已刪");
    sqlx::query(
        "UPDATE user_media SET publishing_until=now() - interval '1 second' WHERE object_key=$1",
    )
    .bind(&active_key)
    .execute(&pool)
    .await
    .unwrap();
    // 背景 drain（複查的 delete_objects）可能先做完；drain 互斥，本輪結束時之前的刪除都已送出
    assert!(wishly_api::uploads::drain_media(&pool).await);
    assert!(purged().await, "圍欄過期後的刪除須標記已刪");
    let mut extra = 0;
    while let Ok(path) = received.try_recv() {
        assert_eq!(path, format!("/media/{active_key}"));
        extra += 1;
    }
    assert!(extra >= 1, "圍欄過期後須再刪一次");
    // The extractor can authenticate before deletion commits. The mutation must
    // revalidate after waiting for the media lock, before touching item rows.
    let stale = user(&pool, "Stale session").await;
    let stale_list = wishlist(&pool, stale.id, ListOpts::default()).await;
    let stale_item = item(&pool, stale_list.id, "x", 1).await;
    let mut barrier = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(913472, hashtext($1))")
        .bind(stale.id.to_string())
        .execute(&mut *barrier)
        .await
        .unwrap();
    let delete_pool = pool.clone();
    let delete_token = stale.token.clone();
    let deleting = tokio::spawn(async move {
        call(
            &delete_pool,
            "DELETE",
            "/me",
            &ck(&delete_token),
            Some(json!({"confirm":"DELETE"})),
        )
        .await
    });
    wait_for_media_waiters(&pool, 913472, 1).await;
    let edit_pool = pool.clone();
    let updating = tokio::spawn(async move {
        call(
            &edit_pool,
            "PATCH",
            &format!("/items/{stale_item}"),
            &ck(&stale.token),
            Some(json!({"image_key":format!("items/{}.jpg", Uuid::new_v4())})),
        )
        .await
    });
    wait_for_media_waiters(&pool, 913472, 2).await;
    barrier.commit().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), deleting)
            .await
            .unwrap()
            .unwrap()
            .0,
        StatusCode::OK
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), updating)
            .await
            .unwrap()
            .unwrap()
            .0,
        StatusCode::UNAUTHORIZED
    );
    let image: Option<String> =
        sqlx::query_scalar("SELECT image_key FROM wishlist_items WHERE id=$1")
            .bind(stale_item)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(image.is_none());
    // A slow origin PUT must not let media waiters exhaust the production pool.
    let reader = user(&pool, "Unrelated reader").await;
    let busy = user(&pool, "Busy upload").await;
    let busy_list = wishlist(&pool, busy.id, ListOpts::default()).await;
    let response = call(
        &pool,
        "POST",
        "/uploads/presign",
        &ck(&busy.token),
        Some(json!({"purpose":"item","content_type":"image/jpeg","content_length":1})),
    )
    .await;
    assert_eq!(response.0, StatusCode::OK);
    let busy_key = response.2["object_key"].as_str().unwrap().to_owned();
    let release = Arc::new(Notify::new());
    *put_gate.lock().await = Some((busy_key.clone(), release.clone()));
    let production = sqlx::postgres::PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(3))
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let confirm_pool = production.clone();
    let confirm_token = busy.token.clone();
    let confirm = tokio::spawn(async move {
        call(
            &confirm_pool,
            "POST",
            "/uploads/confirm",
            &ck(&confirm_token),
            Some(json!({"object_key":busy_key})),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), got_put.recv())
        .await
        .unwrap()
        .unwrap();
    let mut waiting = Vec::new();
    for _ in 0..9 {
        let edit_pool = production.clone();
        let edit_token = busy.token.clone();
        waiting.push(tokio::spawn(async move {
            call(
                &edit_pool,
                "PATCH",
                &format!("/wishlists/{}", busy_list.id),
                &ck(&edit_token),
                Some(json!({"title":"queued"})),
            )
            .await
        }));
    }
    // origin PUT 不在任何交易或鎖內：同一使用者的編輯與無關請求都照常完成
    let unrelated = tokio::time::timeout(
        Duration::from_secs(2),
        call(&production, "GET", "/me", &ck(&reader.token), None),
    )
    .await
    .unwrap();
    assert_eq!(unrelated.0, StatusCode::OK, "{}", unrelated.2);
    for waiter in waiting {
        let response = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("edits must not wait for a slow origin PUT")
            .unwrap();
        assert_eq!(response.0, StatusCode::OK, "{}", response.2);
    }
    assert!(!confirm.is_finished());
    release.notify_one();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), confirm)
            .await
            .unwrap()
            .unwrap()
            .0,
        StatusCode::OK
    );
    assert!(received.recv().await.unwrap().contains("/incoming/"));
    // A worker whose retirement lock times out must retry instead of dropping cleanup.
    let retry_key = format!("items/{}.jpg", Uuid::new_v4());
    sqlx::query("INSERT INTO user_media (user_id,object_key) VALUES ($1,$2)")
        .bind(reader.id)
        .bind(&retry_key)
        .execute(&pool)
        .await
        .unwrap();
    let mut barrier = pool.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(913473, hashtext($1))")
        .bind(&retry_key)
        .execute(&mut *barrier)
        .await
        .unwrap();
    wishly_api::uploads::delete_objects(pool.clone(), vec![retry_key.clone()]);
    wait_for_media_waiters(&pool, 913473, 1).await;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND classid=913473 AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())")
                .fetch_one(&pool).await.unwrap();
            if waiting == 0 { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("background retirement also needs bounded lock acquisition");
    barrier.commit().await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap(),
        format!("/media/{retry_key}")
    );
    production.close().await;
    server.abort();
}

async fn wait_for_media_waiters(pool: &PgPool, class: i32, count: i64) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_locks WHERE locktype='advisory' AND classid=$1::oid AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database())")
                .bind(class).fetch_one(pool).await.unwrap();
            if waiting >= count { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("requests should wait for the media lock");
}
