//! Q9：好友邀請可用使用者 ID（UUID）指定對象；語意與 Email / @handle 相同（找不到人回 404）。
mod common;

use axum::http::StatusCode;
use common::{befriend, ck, user};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn req(pool: &PgPool, token: &str, to: &str) -> (StatusCode, Value) {
    let (s, _, b) = common::call(
        pool,
        "POST",
        "/friends/requests",
        &ck(token),
        Some(json!({ "to": to })),
    )
    .await;
    (s, b)
}

async fn pending(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM friend_requests")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn friends(pool: &PgPool, a: Uuid, b: Uuid) -> bool {
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM friendships WHERE user_a=$1 AND user_b=$2)")
        .bind(a)
        .bind(b)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn request_by_id_reaches_target_incoming(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await; // 沒有 handle
    let (s, body) = req(&pool, &a.token, &b.id.to_string()).await;
    assert_eq!(
        (s, body),
        (StatusCode::ACCEPTED, json!({ "status": "requested" }))
    );
    let (_, _, list) = common::call(&pool, "GET", "/friends", &ck(&b.token), None).await;
    let incoming = list["incoming"].as_array().unwrap();
    assert_eq!(incoming.len(), 1, "{list}");
    assert_eq!(incoming[0]["user"]["id"], json!(a.id), "{list}");

    // 不存在的 UUID 回 404，且不寫入任何資料
    let (s2, body2) = req(&pool, &a.token, &Uuid::new_v4().to_string()).await;
    assert_eq!(
        (s2, body2["code"].clone()),
        (StatusCode::NOT_FOUND, json!("USER_NOT_FOUND"))
    );
    assert_eq!(pending(&pool).await, 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn request_by_id_self_friend_deleted(pool: PgPool) {
    let a = user(&pool, "A").await;
    let f = user(&pool, "F").await;
    let gone = user(&pool, "Gone").await;
    befriend(&pool, a.id, f.id).await;
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&pool)
        .await
        .unwrap();
    let (s, _) = req(&pool, &a.token, &a.id.to_string()).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, body) = req(&pool, &a.token, &f.id.to_string()).await;
    assert_eq!(
        (s, body),
        (StatusCode::ACCEPTED, json!({ "status": "friends" }))
    );
    let (s, _) = req(&pool, &a.token, &gone.id.to_string()).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(pending(&pool).await, 0);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mutual_request_by_id_auto_befriends(pool: PgPool) {
    let a = user(&pool, "A").await;
    let b = user(&pool, "B").await;
    req(&pool, &a.token, &b.id.to_string()).await;
    assert!(!friends(&pool, a.id, b.id).await);
    // 大寫 UUID 也要能解析
    let (_, body) = req(&pool, &b.token, &a.id.to_string().to_uppercase()).await;
    assert_eq!(body, json!({ "status": "friends" }));
    assert!(friends(&pool, a.id, b.id).await);
    assert_eq!(pending(&pool).await, 0);
}

async fn search(pool: &PgPool, token: &str, q: &str) -> Vec<Value> {
    let (s, _, b) = common::call(
        pool,
        "GET",
        &format!("/friends/search?q={q}"),
        &ck(token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    b["users"].as_array().unwrap().clone()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn search_by_name_handle_email_with_relation_and_mutual(pool: PgPool) {
    let me = user(&pool, "Eric Theng").await;
    let fr = user(&pool, "Sherine Theng").await;
    let mid = user(&pool, "Mutual Pal").await;
    let stranger = user(&pool, "Hui Theng").await;
    let gone = user(&pool, "Gone Theng").await;
    let no_email = user(&pool, "Theng Yin").await; // LINE 登入：沒有 Email、沒有 handle
    let other = user(&pool, "Zed").await;
    befriend(&pool, me.id, fr.id).await;
    befriend(&pool, me.id, mid.id).await;
    befriend(&pool, stranger.id, mid.id).await;
    sqlx::query("UPDATE users SET handle = 'zed_x', email = 'zed@example.com' WHERE id = $1")
        .bind(other.id)
        .execute(&pool)
        .await
        .unwrap();
    // handle 有值但不相符時比較結果是 false、沒值時是 NULL：兩者都不可排到共同好友多的人前面
    sqlx::query("UPDATE users SET handle = 'huitheng' WHERE id = $1")
        .bind(stranger.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET email = NULL WHERE id = $1")
        .bind(no_email.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET deleted_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&pool)
        .await
        .unwrap();
    req(&pool, &me.token, &other.id.to_string()).await;

    let r = search(&pool, &me.token, "theng").await;
    let names: Vec<_> = r
        .iter()
        .map(|u| u["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["Eric Theng", "Sherine Theng", "Hui Theng", "Theng Yin"],
        "{r:?}"
    ); // 自己/好友優先、再依共同好友；已刪除不出現
    assert_eq!(r[0]["relation"], "self");
    assert_eq!(r[1]["relation"], "friend");
    assert_eq!(
        (r[2]["relation"].clone(), r[2]["mutual"].clone()),
        (json!("none"), json!(1))
    );

    // handle 前綴（可帶 @）、Email 完全相符；看得到已送出的邀請
    for q in ["%40zed", "ZED_", "zed%40example.com"] {
        let r = search(&pool, &me.token, q).await;
        assert_eq!(r.len(), 1, "{q}: {r:?}");
        assert_eq!(r[0]["relation"], "outgoing");
    }
    // LIKE 萬用字元要跳脫；Email 只接受完全相符
    assert!(search(&pool, &me.token, "%25").await.is_empty());
    assert!(search(&pool, &me.token, "example.com").await.is_empty());
    assert_eq!(
        search(&pool, &other.token, "eric").await[0]["relation"],
        "incoming"
    );
    assert!(search(&pool, &me.token, "").await.is_empty());
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn search_never_reveals_email_by_partial_match(pool: PgPool) {
    let me = user(&pool, "Me").await;
    let t = user(&pool, "Tina").await;
    let e = user(&pool, "tina0827@gmail.com").await; // 把信箱當顯示名稱
    sqlx::query("UPDATE users SET email = 'tina@example.com' WHERE id = $1")
        .bind(t.id)
        .execute(&pool)
        .await
        .unwrap();
    // 名稱含 @ 的人不參與部分比對
    for q in ["gmail", "tina0827", "0827%40gm"] {
        let r = search(&pool, &me.token, q).await;
        assert!(r.iter().all(|u| u["id"] != json!(e.id)), "{q}: {r:?}");
    }
    // 打 Email 片段（中間有 @）不做名稱 / handle 部分比對
    assert!(search(&pool, &me.token, "tina%40exa").await.is_empty());
    // 完全相符才找得到
    assert_eq!(
        search(&pool, &me.token, "tina%40example.com").await[0]["id"],
        json!(t.id)
    );
    assert_eq!(
        search(&pool, &me.token, "TINA0827%40gmail.com").await[0]["id"],
        json!(e.id)
    );
    // 一般名稱照樣可部分比對
    assert_eq!(search(&pool, &me.token, "tin").await.len(), 1);
}

fn names(r: &[Value]) -> Vec<&str> {
    r.iter()
        .map(|u| u["display_name"].as_str().unwrap())
        .collect()
}

async fn set_handle(pool: &PgPool, id: Uuid, h: &str) {
    sqlx::query("UPDATE users SET handle = $2 WHERE id = $1")
        .bind(id)
        .bind(h)
        .execute(pool)
        .await
        .unwrap();
}

/// 排序層級：自己 → 好友 → 共同好友多者 → 陌生人；同層依比對品質（完全 → 前綴 → 字首 → 包含）
#[sqlx::test(migrations = "../../db/migrations")]
async fn search_ranks_self_friend_mutual_stranger(pool: PgPool) {
    let me = user(&pool, "Amy Chen").await;
    let fr = user(&pool, "Zoe Amy").await; // 好友，比對只是「字首」也排在陌生人前
    let m1 = user(&pool, "Pal").await;
    let m2 = user(&pool, "Pal2").await;
    let two = user(&pool, "Lamya").await; // 兩位共同好友（包含命中）
    let one = user(&pool, "Amy Lin").await; // 一位共同好友（前綴命中）
    let s_contains = user(&pool, "Kamy").await;
    let s_word = user(&pool, "Lee Amy").await;
    let s_prefix = user(&pool, "amya").await;
    befriend(&pool, me.id, fr.id).await;
    befriend(&pool, me.id, m1.id).await;
    befriend(&pool, me.id, m2.id).await;
    befriend(&pool, two.id, m1.id).await;
    befriend(&pool, two.id, m2.id).await;
    befriend(&pool, one.id, m1.id).await;
    let _ = (s_contains, s_word, s_prefix);

    let r = search(&pool, &me.token, "AMY").await; // 不分大小寫
    assert_eq!(
        names(&r),
        ["Amy Chen", "Zoe Amy", "Lamya", "Amy Lin", "amya", "Lee Amy", "Kamy"],
        "{r:?}"
    );
    assert_eq!(r[0]["relation"], "self");
    assert_eq!(r[1]["relation"], "friend");
    assert_eq!(
        (r[2]["mutual"].clone(), r[3]["mutual"].clone()),
        (json!(2), json!(1))
    );
}

/// 多詞：每個詞都要命中名稱（包含）或 handle（前綴），順序不拘
#[sqlx::test(migrations = "../../db/migrations")]
async fn search_matches_every_word(pool: PgPool) {
    let me = user(&pool, "Me").await;
    let a = user(&pool, "Eric Theng").await;
    let b = user(&pool, "Eric Wu").await;
    set_handle(&pool, b.id, "ericwu").await;
    let _ = a;
    assert_eq!(
        names(&search(&pool, &me.token, "eric%20th").await),
        ["Eric Theng"]
    );
    assert_eq!(
        names(&search(&pool, &me.token, "theng%20%20ERIC").await),
        ["Eric Theng"]
    );
    // handle 前綴可與名稱詞混用；只有 @ 的詞忽略
    assert_eq!(
        names(&search(&pool, &me.token, "%40ericw%20eric%20%40").await),
        ["Eric Wu"]
    );
    // 只打 @ 不能列出所有有 handle 的人
    assert!(search(&pool, &me.token, "%40").await.is_empty());
    assert!(search(&pool, &me.token, "eric%20zzz").await.is_empty());
}

/// handle 完全相符的陌生人一定入選，不會被 8 筆好友擠掉；入選後仍依層級排序
#[sqlx::test(migrations = "../../db/migrations")]
async fn search_exact_handle_survives_limit(pool: PgPool) {
    let me = user(&pool, "Me").await;
    for i in 0..9 {
        let f = user(&pool, &format!("Bob {i}")).await;
        befriend(&pool, me.id, f.id).await;
    }
    let target = user(&pool, "Someone").await;
    set_handle(&pool, target.id, "bob").await;
    let r = search(&pool, &me.token, "bob").await;
    assert_eq!(r.len(), 8, "{r:?}");
    assert_eq!(r[7]["id"], json!(target.id), "{r:?}");
    assert!(r[..7].iter().all(|u| u["relation"] == "friend"));
}
