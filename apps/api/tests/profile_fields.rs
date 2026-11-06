//! 0068 個人欄位：PATCH /me 驗證、GET /users/{handle} 依關係過濾、封面圖引用與帳號刪除 / 匯出。
mod common;
use axum::http::StatusCode;
use chrono::{Datelike, Duration, Utc};
use common::{befriend, call, ck, user};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn patch(pool: &PgPool, tok: &str, b: Value) -> (StatusCode, Value) {
    let (s, _, v) = call(pool, "PATCH", "/me", &ck(tok), Some(b)).await;
    (s, v)
}
async fn handle(pool: &PgPool, id: Uuid, h: &str) {
    sqlx::query("UPDATE users SET handle=$2 WHERE id=$1")
        .bind(id)
        .bind(h)
        .execute(pool)
        .await
        .unwrap();
}
async fn view(pool: &PgPool, tok: Option<&str>) -> (StatusCode, Value) {
    let h = tok.map(ck).unwrap_or_default();
    let (s, _, v) = call(pool, "GET", "/users/alice", &h, None).await;
    (s, v)
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn patch_validates_gift_lists(pool: PgPool) {
    let a = user(&pool, "A").await;
    let many: Vec<String> = (0..21).map(|i| format!("x{i}")).collect();
    assert_eq!(
        patch(&pool, &a.token, json!({"gift_likes": many})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, v) = patch(
        &pool,
        &a.token,
        json!({"gift_dislikes": ["ok", "x".repeat(41)]}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(v["errors"][0]["pointer"], "/gift_dislikes/1");
    let (s, v) = patch(&pool, &a.token, json!({"gift_likes": [" a ", "a"]})).await;
    assert_eq!((s, &v["gift_likes"]), (StatusCode::OK, &json!(["a"])));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn interests_must_be_in_library(pool: PgPool) {
    let a = user(&pool, "A").await;
    assert_eq!(
        patch(&pool, &a.token, json!({"interests": ["not-a-tag"]}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let eleven: Vec<&str> = wishly_api::interests::INTERESTS[..11].to_vec();
    assert_eq!(
        patch(&pool, &a.token, json!({"interests": eleven})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let (s, v) = patch(
        &pool,
        &a.token,
        json!({"interests": ["travel", "tea", "diy"]}),
    )
    .await;
    assert_eq!(
        (s, v["interests"].as_array().unwrap().len()),
        (StatusCode::OK, 3)
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn vis_rules(pool: PgPool) {
    let a = user(&pool, "A").await;
    for k in ["age_vis", "home_address_vis"] {
        assert_eq!(
            patch(&pool, &a.token, json!({ k: "public" })).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    let (s, _) = patch(
        &pool,
        &a.token,
        json!({"birth_year": Utc::now().year() + 1}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, v) = patch(
        &pool,
        &a.token,
        json!({"birth_year": 1990, "age_vis": "close"}),
    )
    .await;
    assert_eq!(
        (s, v["birth_year"].clone(), v["age_vis"].clone()),
        (StatusCode::OK, json!(1990), json!("close"))
    );
    let (_, v) = patch(&pool, &a.token, json!({"birth_year": null})).await;
    assert!(v["birth_year"].is_null());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn visibility_matrix(pool: PgPool) {
    let a = user(&pool, "A").await;
    handle(&pool, a.id, "alice").await;
    let (s, _) = patch(
        &pool,
        &a.token,
        json!({"gift_likes": ["tea"], "gift_prefs_vis": "close", "interests": ["travel"], "interests_vis": "friends",
               "birth_year": 1990, "birthday": "05-01", "age_vis": "friends"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let nf = user(&pool, "N").await;
    let f = user(&pool, "F").await;
    let c = user(&pool, "C").await;
    befriend(&pool, a.id, f.id).await;
    befriend(&pool, a.id, c.id).await;
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) VALUES ($1, $2)")
        .bind(a.id)
        .bind(c.id)
        .execute(&pool)
        .await
        .unwrap();
    let has = |v: &Value, k: &str| v["user"].get(k).is_some();
    for t in [None, Some(nf.token.as_str())] {
        let (_, v) = view(&pool, t).await;
        assert!(!has(&v, "gift_likes") && !has(&v, "interests") && !has(&v, "age"));
    }
    let (_, v) = view(&pool, Some(&f.token)).await;
    assert!(has(&v, "interests") && has(&v, "age") && !has(&v, "gift_likes"));
    for t in [&c.token, &a.token] {
        let (_, v) = view(&pool, Some(t)).await;
        assert!(has(&v, "interests") && has(&v, "age") && has(&v, "gift_likes"));
    }
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn age_computed_from_birthday(pool: PgPool) {
    let a = user(&pool, "A").await;
    handle(&pool, a.id, "alice").await;
    let t = Utc::now().date_naive() + Duration::days(1);
    patch(&pool, &a.token, json!({"birth_year": Utc::now().year() - 30, "birthday": t.format("%m-%d").to_string(), "age_vis": "friends"})).await;
    let (_, v) = view(&pool, Some(&a.token)).await;
    // 明天是 1/1 時 birthday 落在隔年，仍是「今天未到生日」
    assert_eq!(v["user"]["age"], 29);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn blocked_profile_hides_rich_fields(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    handle(&pool, b.id, "alice").await;
    patch(&pool, &b.token, json!({"interests": ["tea"]})).await;
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
        .bind(b.id)
        .bind(a.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(view(&pool, Some(&a.token)).await.0, StatusCode::NOT_FOUND);
    sqlx::query("DELETE FROM user_blocks")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO user_blocks (blocker_id, blocked_id) VALUES ($1, $2)")
        .bind(a.id)
        .bind(b.id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, v) = view(&pool, Some(&a.token)).await;
    assert_eq!(s, StatusCode::OK);
    let mut keys: Vec<&String> = v["user"].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "avatar_url",
            "bio",
            "birthday",
            "display_name",
            "handle",
            "id"
        ]
        .iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .iter()
        .collect::<Vec<_>>()
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn close_field_hidden_after_unfriend(pool: PgPool) {
    let a = user(&pool, "A").await;
    let c = user(&pool, "C").await;
    handle(&pool, a.id, "alice").await;
    befriend(&pool, a.id, c.id).await;
    sqlx::query("INSERT INTO close_friends (owner_id, friend_id) VALUES ($1, $2)")
        .bind(a.id)
        .bind(c.id)
        .execute(&pool)
        .await
        .unwrap();
    patch(
        &pool,
        &a.token,
        json!({"gift_likes": ["tea"], "gift_prefs_vis": "close"}),
    )
    .await;
    assert!(view(&pool, Some(&c.token)).await.1["user"]
        .get("gift_likes")
        .is_some());
    let (s, _, _) = call(
        &pool,
        "DELETE",
        &format!("/friends/{}", c.id),
        &ck(&a.token),
        None,
    )
    .await;
    assert!(s.is_success(), "{s}");
    assert!(view(&pool, Some(&c.token)).await.1["user"]
        .get("gift_likes")
        .is_none());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn cover_key_not_retired_while_referenced(pool: PgPool) {
    let a = user(&pool, "A").await;
    let k1 = format!("covers/{}.jpg", Uuid::new_v4());
    let k2 = format!("covers/{}.jpg", Uuid::new_v4());
    let (s, v) = patch(&pool, &a.token, json!({"cover_image_key": k1})).await;
    assert_eq!(
        (s, v["cover_image_key"].clone()),
        (StatusCode::OK, json!(k1))
    );
    assert!(v["cover_image_url"].is_string());
    sqlx::query("UPDATE user_media SET retire_requested_at = now() WHERE object_key=$1")
        .bind(&k1)
        .execute(&pool)
        .await
        .unwrap();
    wishly_api::uploads::drain_media(&pool).await;
    let live = |k: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT deleted_at IS NULL FROM user_media WHERE object_key=$1",
            )
            .bind(k)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    assert!(live(k1.clone()).await);
    // 換成 preset：舊圖排入退役並被清掉，且 preset 與圖二選一
    let (_, v) = patch(&pool, &a.token, json!({"cover_preset": "gift"})).await;
    assert!(v["cover_image_key"].is_null() && v["cover_preset"] == "gift");
    let (s, _) = patch(
        &pool,
        &a.token,
        json!({"cover_preset": "gift", "cover_image_key": k2}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let pending: bool = sqlx::query_scalar("SELECT retire_requested_at IS NOT NULL OR deleted_at IS NOT NULL FROM user_media WHERE object_key=$1").bind(&k1).fetch_one(&pool).await.unwrap();
    assert!(pending);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn account_delete_clears_profile_fields(pool: PgPool) {
    let a = user(&pool, "A").await;
    patch(&pool, &a.token, json!({"gift_likes": ["tea"], "interests": ["tea"], "birth_year": 1990, "cover_preset": "gift"})).await;
    let (s, _, _) = call(
        &pool,
        "DELETE",
        "/me",
        &ck(&a.token),
        Some(json!({"confirm": "DELETE"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (n, m, y, c): (usize, usize, Option<i16>, Option<String>) = sqlx::query_as("SELECT cardinality(gift_likes), cardinality(interests), birth_year, cover_preset FROM users WHERE id=$1")
        .bind(a.id).fetch_one(&pool).await.map(|r: (i32, i32, Option<i16>, Option<String>)| (r.0 as usize, r.1 as usize, r.2, r.3)).unwrap();
    assert_eq!((n, m, y, c), (0, 0, None, None));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn export_includes_profile_fields(pool: PgPool) {
    let a = user(&pool, "A").await;
    patch(
        &pool,
        &a.token,
        json!({"gift_likes": ["tea"], "interests": ["diy"], "birth_year": 1990}),
    )
    .await;
    let (_, _, v) = call(&pool, "GET", "/me/export", &ck(&a.token), None).await;
    assert_eq!(v["user"]["gift_likes"], json!(["tea"]));
    assert_eq!(v["user"]["interests"], json!(["diy"]));
    assert_eq!(v["user"]["birth_year"], 1990);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn patch_rejects_without_partial_write(pool: PgPool) {
    let a = user(&pool, "A").await;
    let (s, _) = patch(
        &pool,
        &a.token,
        json!({"gift_likes": ["ok"], "interests": ["bad"]}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _) = patch(
        &pool,
        &a.token,
        json!({"gift_likes": ["ok"], "cover_preset": "nope"}),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let n: i32 = sqlx::query_scalar("SELECT cardinality(gift_likes) FROM users WHERE id=$1")
        .bind(a.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
}
