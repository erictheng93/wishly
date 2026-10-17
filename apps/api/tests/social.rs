//! 社群：好友（申請/邀請/解除）、個人頁清單與捐助可見性、handle 與捐助 visibility
mod common;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use common::befriend;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wishly_api::{app, AppState};

/// (id, session token, email)
async fn handle_user(pool: &PgPool, name: &str, handle: Option<&str>) -> (Uuid, String, String) {
    let common::TestUser { id, token } = common::user(pool, name).await;
    let email: String =
        sqlx::query_scalar("UPDATE users SET handle=$2 WHERE id=$1 RETURNING email")
            .bind(id)
            .bind(handle)
            .fetch_one(pool)
            .await
            .unwrap();
    (id, token, email)
}

/// 清單 + 一個品項；回 (wishlist_id, item_id)
async fn list(pool: &PgPool, owner: Uuid, vis: &'static str, surprise: bool) -> (Uuid, Uuid) {
    let list = common::wishlist(
        pool,
        owner,
        common::ListOpts {
            visibility: Some(vis),
            surprise,
            event_offset_days: Some(30),
            ..Default::default()
        },
    )
    .await;
    let wid = list.id;
    sqlx::query("UPDATE wishlists SET title=$2 WHERE id=$1")
        .bind(wid)
        .bind(format!("清單-{vis}"))
        .execute(pool)
        .await
        .unwrap();
    let it = common::item(pool, wid, "奶瓶", 3).await;
    (wid, it)
}

/// 直接寫入使用者捐助
async fn user_claim(pool: &PgPool, item: Uuid, user: Uuid, vis: &str) -> Uuid {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed=qty_claimed+1 WHERE id=$1")
        .bind(item)
        .execute(&mut *tx)
        .await
        .unwrap();
    let id = sqlx::query_scalar("INSERT INTO claims (item_id, user_id, claimer_name, qty, visibility) VALUES ($1, $2, 'x', 1, $3::text::share_level) RETURNING id")
        .bind(item).bind(user).bind(vis).fetch_one(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    id
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friend_request_flow_and_no_enumeration(pool: PgPool) {
    let (a, ta, _) = handle_user(&pool, "A", Some("alice")).await;
    let (b, tb, eb) = handle_user(&pool, "B", Some("bobby")).await;
    // 不存在的帳號與存在的帳號回應相同
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "nobody@example.com" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "ghost" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "alice" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    ); // 對自己
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": eb.to_uppercase() }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "@bobby" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    ); // 重複
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "bobby" }))
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/friends",
        &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(v["incoming"].as_array().unwrap().len(), 1);
    assert_eq!(v["incoming"][0]["user"]["handle"], "alice");
    let (_, _, v2) = common::call(
        &pool,
        "GET",
        "/friends",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(v2["outgoing"].as_array().unwrap().len(), 1);
    // 寄件者不能自己接受
    let rid = v["incoming"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/friends/requests/{rid}/accept"),
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (s, _, v) = common::call(
        &pool,
        "POST",
        &format!("/friends/requests/{rid}/accept"),
        &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["friend"]["id"], json!(a));
    assert!(crate_are_friends(&pool, a, b).await);
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/friends",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(v["friends"][0]["handle"], "bobby");
    assert!(v["outgoing"].as_array().unwrap().is_empty());
    // 已是好友再申請：不動作
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "bobby" }))
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    common::assert_all_reconciled(&pool).await;
}

async fn crate_are_friends(pool: &PgPool, a: Uuid, b: Uuid) -> bool {
    wishly_api::access::are_friends(pool, a, b).await.unwrap()
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn mutual_request_auto_friends_and_decline_withdraw(pool: PgPool) {
    let (a, ta, _) = handle_user(&pool, "A", Some("alice")).await;
    let (b, tb, _) = handle_user(&pool, "B", Some("bobby")).await;
    let (_, tc, _) = handle_user(&pool, "C", Some("carol")).await;
    common::call(
        &pool,
        "POST",
        "/friends/requests",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "to": "bobby" })),
    )
    .await;
    common::call(
        &pool,
        "POST",
        "/friends/requests",
        &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "to": "alice" })),
    )
    .await;
    assert!(crate_are_friends(&pool, a, b).await);
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // 拒絕 / 撤回
    common::call(
        &pool,
        "POST",
        "/friends/requests",
        &[common::ck(&tc), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "to": "alice" })),
    )
    .await;
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/friends",
        &[common::ck(&tc), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    let rid = v["outgoing"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &format!("/friends/requests/{rid}"),
            &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    ); // 無關第三人
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &format!("/friends/requests/{rid}"),
            &[common::ck(&tc), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    ); // 撤回
    common::call(
        &pool,
        "POST",
        "/friends/requests",
        &[common::ck(&tc), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "to": "alice" })),
    )
    .await;
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/friends",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    let rid = v["incoming"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &format!("/friends/requests/{rid}"),
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    ); // 拒絕
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn friend_request_rate_limited(pool: PgPool) {
    let (_, ta, _) = handle_user(&pool, "A", Some("alice")).await;
    for _ in 0..30 {
        assert_eq!(
            common::call(
                &pool,
                "POST",
                "/friends/requests",
                &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
                Some(json!({ "to": "ghost" }))
            )
            .await
            .0,
            StatusCode::ACCEPTED
        );
    }
    assert_eq!(
        common::call(
            &pool,
            "POST",
            "/friends/requests",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "to": "ghost" }))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn invite_links(pool: PgPool) {
    let (a, ta, _) = handle_user(&pool, "A", Some("alice")).await;
    let (b, tb, _) = handle_user(&pool, "B", Some("bobby")).await;
    let (s, _, v) = common::call(
        &pool,
        "POST",
        "/friends/invites",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let tok = v["token"].as_str().unwrap().to_string();
    assert!(v["url"]
        .as_str()
        .unwrap()
        .ends_with(&format!("/invite/{tok}")));
    // 預覽免登入
    let (s, _, p) = common::call(
        &pool,
        "GET",
        &format!("/friends/invites/{tok}"),
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        (s, p["inviter"]["handle"].as_str()),
        (StatusCode::OK, Some("alice"))
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/friends/invites/nope",
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // 自己的邀請
    let (s, _, e) = common::call(
        &pool,
        "POST",
        &format!("/friends/invites/{tok}/accept"),
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        (s, e["code"].as_str()),
        (StatusCode::UNPROCESSABLE_ENTITY, Some("SELF_INVITE"))
    );
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/friends/invites/{tok}/accept"),
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // 接受（含雙向待處理申請被清掉），重複接受仍 200
    common::call(
        &pool,
        "POST",
        "/friends/requests",
        &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "to": "alice" })),
    )
    .await;
    let (s, _, v) = common::call(
        &pool,
        "POST",
        &format!("/friends/invites/{tok}/accept"),
        &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!((s, v["friend"]["id"].clone()), (StatusCode::OK, json!(a)));
    assert!(crate_are_friends(&pool, a, b).await);
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/friends/invites/{tok}/accept"),
            &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM friend_requests")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    // 新邀請撤銷舊的
    let (_, _, v2) = common::call(
        &pool,
        "POST",
        "/friends/invites",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        common::call(
            &pool,
            "GET",
            &format!("/friends/invites/{tok}"),
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let tok2 = v2["token"].as_str().unwrap();
    assert_eq!(
        common::call(
            &pool,
            "GET",
            &format!("/friends/invites/{tok2}"),
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    // 撤銷全部
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            "/friends/invites",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            &format!("/friends/invites/{tok2}"),
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // 過期
    let (_, _, v3) = common::call(
        &pool,
        "POST",
        "/friends/invites",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    let tok3 = v3["token"].as_str().unwrap();
    sqlx::query("UPDATE friend_invites SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        common::call(
            &pool,
            "GET",
            &format!("/friends/invites/{tok3}"),
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, tc, _) = handle_user(&pool, "C", None).await;
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/friends/invites/{tok3}/accept"),
            &[common::ck(&tc), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn unfriend_clears_allowed_users(pool: PgPool) {
    let (a, ta, _) = handle_user(&pool, "A", Some("alice")).await;
    let (b, _, _) = handle_user(&pool, "B", Some("bobby")).await;
    let (c, _, _) = handle_user(&pool, "C", Some("carol")).await;
    befriend(&pool, a, b).await;
    befriend(&pool, a, c).await;
    let (wa, _) = list(&pool, a, "selected", false).await;
    let (wb, _) = list(&pool, b, "selected", false).await;
    for (w, u) in [(wa, b), (wa, c), (wb, a)] {
        sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)")
            .bind(w)
            .bind(u)
            .execute(&pool)
            .await
            .unwrap();
    }
    assert_eq!(
        common::call(
            &pool,
            "DELETE",
            &format!("/friends/{b}"),
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(!crate_are_friends(&pool, a, b).await && crate_are_friends(&pool, a, c).await);
    let left: Vec<(Uuid, Uuid)> =
        sqlx::query_as("SELECT wishlist_id, user_id FROM wishlist_allowed_users")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(left, vec![(wa, c)]);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_lists_visibility(pool: PgPool) {
    let (o, to, _) = handle_user(&pool, "Owner", Some("owner")).await;
    let (f, tf, _) = handle_user(&pool, "Friend", Some("friend")).await;
    let (sel, tsel, _) = handle_user(&pool, "Sel", Some("sel")).await;
    let (_, ts, _) = handle_user(&pool, "Stranger", Some("stranger")).await;
    befriend(&pool, o, f).await;
    befriend(&pool, o, sel).await;
    for v in ["public", "link", "friends", "selected", "private"] {
        list(&pool, o, v, false).await;
    }
    let (w_sel, _) = list(&pool, o, "selected", false).await;
    sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)")
        .bind(w_sel)
        .bind(sel)
        .execute(&pool)
        .await
        .unwrap();
    // draft 不列
    sqlx::query("UPDATE wishlists SET status='draft' WHERE id=$1")
        .bind(w_sel)
        .execute(&pool)
        .await
        .unwrap();
    let (w_sel2, _) = list(&pool, o, "selected", false).await;
    sqlx::query("INSERT INTO wishlist_allowed_users (wishlist_id, user_id) VALUES ($1, $2)")
        .bind(w_sel2)
        .bind(sel)
        .execute(&pool)
        .await
        .unwrap();
    let vis = |v: &Value| -> Vec<String> {
        let mut x: Vec<String> = v["wishlists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["visibility"].as_str().unwrap().into())
            .collect();
        x.sort();
        x
    };
    let (s, _, v) = common::call(
        &pool,
        "GET",
        "/users/owner",
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        (s, v["relation"].as_str(), vis(&v)),
        (
            StatusCode::OK,
            Some("anonymous"),
            vec!["public".to_string()]
        )
    );
    assert_eq!(v["wishlists"][0]["item_count"], 1);
    assert_eq!(v["wishlists"][0]["completion_pct"], 0);
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/owner",
        &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        (v["relation"].as_str(), vis(&v)),
        (Some("none"), vec!["public".to_string()])
    );
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/OWNER",
        &[common::ck(&tf), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        (v["relation"].as_str(), vis(&v)),
        (
            Some("friend"),
            vec!["friends".to_string(), "public".to_string()]
        )
    );
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/owner",
        &[common::ck(&tsel), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(vis(&v), vec!["friends", "public", "selected"]);
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/owner",
        &[common::ck(&to), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(
        (v["relation"].as_str(), vis(&v)),
        (
            Some("self"),
            ["friends", "public", "selected", "selected"]
                .map(String::from)
                .to_vec()
        )
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/users/nobody",
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // 申請中的關係
    common::call(
        &pool,
        "POST",
        "/friends/requests",
        &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "to": "owner" })),
    )
    .await;
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/users/owner",
            &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2["relation"],
        "outgoing"
    );
    assert_eq!(
        common::call(
            &pool,
            "GET",
            "/users/stranger",
            &[common::ck(&to), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2["relation"],
        "incoming"
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_donations_visibility(pool: PgPool) {
    let (d, td, _) = handle_user(&pool, "Donor", Some("donor")).await;
    let (o, _, _) = handle_user(&pool, "Owner", Some("owner")).await;
    let (f, tf, _) = handle_user(&pool, "Friend", Some("friend")).await;
    let (_, ts, _) = handle_user(&pool, "Stranger", Some("stranger")).await;
    befriend(&pool, d, f).await; // 捐助者的好友（非清單擁有者的好友）
    let (_, pub_item) = list(&pool, o, "public", false).await;
    let (_, link_item) = list(&pool, o, "link", false).await;
    let (_, priv_item) = list(&pool, o, "private", false).await;
    let (_, pw_item) = list(&pool, o, "public", false).await;
    sqlx::query("UPDATE wishlists SET visibility='password', access_password_hash='x' WHERE id=(SELECT wishlist_id FROM wishlist_items WHERE id=$1)").bind(pw_item).execute(&pool).await.unwrap();
    let (_, locked_item) = list(&pool, o, "public", true).await;
    let (_, fr_item) = list(&pool, o, "friends", false).await;
    let (_, hidden_claim_item) = list(&pool, o, "public", false).await;
    for it in [
        pub_item,
        link_item,
        priv_item,
        pw_item,
        locked_item,
        fr_item,
    ] {
        user_claim(&pool, it, d, "public").await;
    }
    user_claim(&pool, hidden_claim_item, d, "private").await;

    let titles = |v: &Value| -> Vec<(String, bool)> {
        let mut x: Vec<_> = v["donations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["wishlist"]["title"].as_str().unwrap().to_string(),
                    c["wishlist"]["slug"].is_null(),
                )
            })
            .collect();
        x.sort();
        x
    };
    // 陌生人/匿名：public 清單與 link 清單（slug 為 null）；排除 private/password/驚喜鎖定/friends（非擁有者好友）/claim 私人
    let expect = vec![
        ("清單-link".to_string(), true),
        ("清單-public".to_string(), false),
    ];
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(titles(&v), expect, "{v}");
    assert_eq!(v["donations"][0]["wishlist"]["owner"]["handle"], "owner");
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(titles(&v), expect);
    // 本人：多看到 claim visibility=private 的（仍受清單規則限制）
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&td), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(v["donations"].as_array().unwrap().len(), 3);
    // 捐助改 friends：陌生人看不到、捐助者好友看得到
    sqlx::query("UPDATE claims SET visibility='friends' WHERE item_id=$1")
        .bind(pub_item)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(titles(&v), vec![("清單-link".to_string(), true)]);
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&tf), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(titles(&v).len(), 2);
    // 擁有者的好友可見 friends 清單的捐助
    let (_, tg, _) = handle_user(&pool, "G", Some("gee")).await;
    let gid: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE handle='gee'")
        .fetch_one(&pool)
        .await
        .unwrap();
    befriend(&pool, gid, o).await;
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&tg), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert!(titles(&v).contains(&("清單-friends".to_string(), false)));
    // 已取消的捐助不列
    sqlx::query("UPDATE claims SET status='cancelled' WHERE item_id=$1")
        .bind(link_item)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE wishlist_items SET qty_claimed=0 WHERE id=$1")
        .bind(link_item)
        .execute(&pool)
        .await
        .unwrap();
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&tg), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert!(!titles(&v).iter().any(|t| t.0 == "清單-link"));
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn me_handle_and_default_visibility(pool: PgPool) {
    let (_, ta, _) = handle_user(&pool, "A", None).await;
    let (_, tb, _) = handle_user(&pool, "B", Some("taken")).await;
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/me",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert!(v["handle"].is_null());
    assert_eq!(v["default_claim_visibility"], "private");
    let (s, _, v) = common::call(
        &pool,
        "PATCH",
        "/me",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "handle": "Alice_01", "default_claim_visibility": "friends" })),
    )
    .await;
    assert_eq!(
        (
            s,
            v["handle"].as_str(),
            v["default_claim_visibility"].as_str()
        ),
        (StatusCode::OK, Some("alice_01"), Some("friends"))
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            "/me",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "handle": "taken" }))
        )
        .await
        .2["code"],
        "HANDLE_TAKEN"
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            "/me",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "handle": "taken" }))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    for bad in ["ab", "has space", "中文名字", &"a".repeat(31)] {
        assert_eq!(
            common::call(
                &pool,
                "PATCH",
                "/me",
                &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
                Some(json!({ "handle": bad }))
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}"
        );
    }
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            "/me",
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "default_claim_visibility": "x" }))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 只改 display_name 不動 handle；null 清除
    let (_, _, v) = common::call(
        &pool,
        "PATCH",
        "/me",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "display_name": "新名" })),
    )
    .await;
    assert_eq!(v["handle"], "alice_01");
    let (_, _, v) = common::call(
        &pool,
        "PATCH",
        "/me",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "handle": null })),
    )
    .await;
    assert!(v["handle"].is_null());
    let _ = tb;
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn claim_visibility_default_override_patch(pool: PgPool) {
    let (o, _, _) = handle_user(&pool, "O", None).await;
    let (_, ta, _) = handle_user(&pool, "A", None).await;
    common::call(
        &pool,
        "PATCH",
        "/me",
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "default_claim_visibility": "friends" })),
    )
    .await;
    let mk = |_: ()| async { list(&pool, o, "public", false).await.1 };
    let (i1, i2, i3) = (mk(()).await, mk(()).await, mk(()).await);
    // 預設（trigger）
    let (s, _, v) = common::call(
        &pool,
        "POST",
        &format!("/items/{i1}/claims"),
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "qty": 1 })),
    )
    .await;
    assert_eq!(
        (s, v["claim"]["visibility"].as_str()),
        (StatusCode::CREATED, Some("friends")),
        "{v}"
    );
    // 覆寫
    let (_, _, v) = common::call(
        &pool,
        "POST",
        &format!("/items/{i2}/claims"),
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "qty": 1, "visibility": "public" })),
    )
    .await;
    assert_eq!(v["claim"]["visibility"], "public");
    let cid = v["claim"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        common::call(
            &pool,
            "POST",
            &format!("/items/{i3}/claims"),
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "qty": 1, "visibility": "bogus" }))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // PATCH
    let (s, _, v) = common::call(
        &pool,
        "PATCH",
        &format!("/claims/{cid}"),
        &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
        Some(json!({ "visibility": "private" })),
    )
    .await;
    assert_eq!(
        (s, v["claim"]["visibility"].as_str()),
        (StatusCode::OK, Some("private")),
        "{v}"
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &format!("/claims/{cid}"),
            &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "visibility": "x" }))
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 舊訪客捐助：PATCH visibility → 422
    let (gtok, gcid) = common::legacy_guest_claim(&pool, i3, 1, None).await;
    let gcid = gcid.to_string();
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/v1/claims/{gcid}"))
        .header("x-guest-token", gtok)
        .header("content-type", "application/json")
        .body(Body::from(json!({ "visibility": "public" }).to_string()))
        .unwrap();
    assert_eq!(
        app(AppState { pool: pool.clone() })
            .oneshot(req)
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // 別人不能改
    let (_, tb, _) = handle_user(&pool, "B", None).await;
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &format!("/claims/{cid}"),
            &[common::ck(&tb), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "visibility": "public" }))
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn public_page_masks_non_public_user_claims(pool: PgPool) {
    let (o, _, _) = handle_user(&pool, "O", None).await;
    let (d, _, _) = handle_user(&pool, "D", None).await;
    let (e, _, _) = handle_user(&pool, "E", None).await;
    let (wid, it) = list(&pool, o, "link", false).await;
    sqlx::query("UPDATE wishlists SET show_claimer_names = true WHERE id = $1")
        .bind(wid)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE claims SET claimer_name = '小明' WHERE id = $1")
        .bind(user_claim(&pool, it, d, "public").await)
        .execute(&pool)
        .await
        .unwrap();
    user_claim(&pool, it, e, "friends").await;
    let slug: String = sqlx::query_scalar("SELECT slug::text FROM wishlists WHERE id = $1")
        .bind(wid)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (s, _, v) = common::call(
        &pool,
        "GET",
        &format!("/public/wishlists/{slug}"),
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let names: Vec<&str> = v["items"][0]["claimers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["小明", "匿名"]);
    common::assert_all_reconciled(&pool).await;
}

/// 直接寫入點數認捐（不經點數記帳；只測個人頁可見性）
async fn pledge_row(pool: &PgPool, wl: Uuid, user: Uuid, points: i64, anon: bool, status: &str) {
    let item: Uuid = sqlx::query_scalar(
        "INSERT INTO wishlist_items (wishlist_id, title, qty_needed, funding_mode, target_points, pledged_points, funding_status, funding_deadline, fulfillment_type)
         VALUES ($1, '推車', 1, 'crowdfund', 10000, CASE WHEN $2 IN ('pledged','captured') THEN $3::bigint ELSE 0 END, 'open', now() + interval '7 days', 'concierge') RETURNING id").bind(wl).bind(status).bind(points).fetch_one(pool).await.unwrap();
    let wallet: Uuid = sqlx::query_scalar("INSERT INTO point_wallets (user_id) VALUES ($1) ON CONFLICT (user_id) DO UPDATE SET user_id = EXCLUDED.user_id RETURNING id")
        .bind(user).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO contributions (item_id, wishlist_id, user_id, wallet_id, donor_name, points, is_anonymous, status, released_at, captured_at)
                 VALUES ($1, $2, $3, $4, 'x', $5, $6, $7::text::contribution_status, CASE WHEN $7 = 'released' THEN now() END, CASE WHEN $7 = 'captured' THEN now() END)")
        .bind(item).bind(wl).bind(user).bind(wallet).bind(points).bind(anon).bind(status).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_pledge_visibility_is_per_record(pool: PgPool) {
    let (o, _, _) = handle_user(&pool, "O", None).await;
    let (d, td, _) = handle_user(&pool, "D", Some("donor")).await;
    let (f, tf, _) = handle_user(&pool, "F", None).await;
    let (_, ts, _) = handle_user(&pool, "S", None).await;
    befriend(&pool, d, f).await;
    let (wl, _) = list(&pool, o, "public", false).await;
    pledge_row(&pool, wl, d, 300, false, "pledged").await;
    pledge_row(&pool, wl, d, 200, true, "captured").await; // 匿名：只有本人看得到
    pledge_row(&pool, wl, d, 100, false, "released").await; // 已退回：不列
    let pts = |v: &Value| -> Vec<i64> {
        v["donations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["kind"] == "pledge")
            .map(|x| x["points"].as_i64().unwrap())
            .collect()
    };
    let set = |vis: &'static str| {
        let (pool, tok) = (pool.clone(), td.clone());
        async move {
            assert_eq!(
                common::call(
                    &pool,
                    "PATCH",
                    "/me",
                    &[common::ck(&tok), common::idem(Uuid::new_v4())].concat(),
                    Some(json!({ "default_claim_visibility": vis }))
                )
                .await
                .0,
                StatusCode::OK
            );
        }
    };

    set("private").await;
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        Vec::<i64>::new()
    );
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&tf), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        Vec::<i64>::new()
    );
    let (_, _, me) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&td), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    let mut mine = pts(&me);
    mine.sort();
    assert_eq!(mine, [200, 300]);
    set("friends").await;
    // 改預設不回溯舊認捐。
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&tf), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        Vec::<i64>::new()
    );
    let cid: Uuid =
        sqlx::query_scalar("SELECT id FROM contributions WHERE user_id = $1 AND points = 300")
            .bind(d)
            .fetch_one(&pool)
            .await
            .unwrap();
    let uri = format!("/contributions/{cid}");
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &uri,
            &[common::ck(&td), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "visibility": "friends" }))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&tf), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        [300]
    );
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        Vec::<i64>::new()
    );
    set("public").await;
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[vec![], common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        Vec::<i64>::new()
    );
    // 匿名即使逐筆設為公開仍只給本人看。
    sqlx::query(
        "UPDATE contributions SET visibility = 'public' WHERE user_id = $1 AND is_anonymous",
    )
    .bind(d)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &uri,
            &[common::ck(&td), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "visibility": "public" }))
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/donor",
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(pts(&v), [300]);
    let p = v["donations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["kind"] == "pledge")
        .unwrap();
    assert_eq!(
        (p["qty"].clone(), p["status"].as_str()),
        (Value::Null, Some("pledged"))
    );
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&tf), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        [300]
    );
    assert_eq!(
        pts(&common::call(
            &pool,
            "GET",
            "/users/donor",
            &[common::ck(&ts), common::idem(Uuid::new_v4())].concat(),
            None
        )
        .await
        .2),
        [300]
    );
    assert_eq!(
        common::call(
            &pool,
            "PATCH",
            &uri,
            &[common::ck(&td), common::idem(Uuid::new_v4())].concat(),
            Some(json!({ "visibility": "private" }))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(pts(&common::call(
        &pool,
        "GET",
        "/users/donor",
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None
    )
    .await
    .2)
    .is_empty());
    let mut mine = pts(&common::call(
        &pool,
        "GET",
        "/users/donor",
        &[common::ck(&td), common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await
    .2);
    mine.sort();
    assert_eq!(mine, [200, 300]);
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_completion_counts_funded_crowdfund_item(pool: PgPool) {
    let (o, _, _) = handle_user(&pool, "O", Some("owner")).await;
    let (wl, _) = list(&pool, o, "public", false).await; // 數量品項 0/3
    let item: Uuid = sqlx::query_scalar("INSERT INTO wishlist_items (wishlist_id, title, qty_needed, funding_mode, target_points, pledged_points, funding_status, funding_deadline, fulfillment_type)
                 VALUES ($1, '推車', 1, 'crowdfund', 500, 500, 'funded', now() + interval '7 days', 'concierge') RETURNING id").bind(wl).fetch_one(&pool).await.unwrap();
    let wallet: Uuid =
        sqlx::query_scalar("INSERT INTO point_wallets(user_id) VALUES ($1) RETURNING id")
            .bind(o)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO contributions(item_id,wishlist_id,user_id,wallet_id,donor_name,points,status,captured_at) VALUES ($1,$2,$3,$4,'Owner',500,'captured',now())")
        .bind(item).bind(wl).bind(o).bind(wallet).execute(&pool).await.unwrap();
    let (_, _, v) = common::call(
        &pool,
        "GET",
        "/users/owner",
        &[vec![], common::idem(Uuid::new_v4())].concat(),
        None,
    )
    .await;
    assert_eq!(v["wishlists"][0]["completion_pct"], 25); // 已達標眾籌算 1/1：1 / (3 + 1)
    common::assert_all_reconciled(&pool).await;
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn me_bio_birthday_and_profile_visibility(pool: PgPool) {
    let (a, ta, _) = handle_user(&pool, "A", Some("alice")).await;
    let (b, tb, _) = handle_user(&pool, "B", Some("bobby")).await;
    let (_, tc, _) = handle_user(&pool, "C", Some("carol")).await;
    befriend(&pool, a, b).await;
    let patch = |body: Value| {
        let (pool, ta) = (pool.clone(), ta.clone());
        async move {
            common::call(
                &pool,
                "PATCH",
                "/me",
                &[common::ck(&ta), common::idem(Uuid::new_v4())].concat(),
                Some(body),
            )
            .await
        }
    };
    let (_, _, v) = common::call(&pool, "GET", "/me", &common::ck(&ta), None).await;
    assert!(v["bio"].is_null() && v["birthday"].is_null());
    assert_eq!(v["birthday_visibility"], "friends");

    let (s, _, v) = patch(json!({ "bio": "  喜歡咖啡  ", "birthday": "02-29" })).await;
    assert_eq!(
        (s, v["bio"].as_str(), v["birthday"].as_str()),
        (StatusCode::OK, Some("喜歡咖啡"), Some("02-29"))
    );
    for bad in [
        json!({ "birthday": "02-30" }),
        json!({ "birthday": "13-01" }),
        json!({ "birthday": "2-1" }),
        json!({ "birthday": "1990-02-01" }),
        json!({ "bio": "字".repeat(161) }),
        json!({ "birthday_visibility": "x" }),
    ] {
        assert_eq!(
            patch(bad.clone()).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{bad}"
        );
    }

    // 個人頁：bio 一律可見；生日依 birthday_visibility（預設僅好友）
    let bday = |tok: Option<String>| {
        let pool = pool.clone();
        async move {
            let h = tok.map(|t| common::ck(&t)).unwrap_or_default();
            let (_, _, v) = common::call(&pool, "GET", "/users/alice", &h, None).await;
            assert_eq!(v["user"]["bio"], "喜歡咖啡");
            v["user"]["birthday"].as_str().map(String::from)
        }
    };
    let see = |x: Option<String>| x.is_some();
    assert_eq!(
        [
            see(bday(Some(ta.clone())).await),
            see(bday(Some(tb.clone())).await),
            see(bday(Some(tc.clone())).await),
            see(bday(None).await)
        ],
        [true, true, false, false]
    );
    patch(json!({ "birthday_visibility": "public" })).await;
    assert_eq!(bday(None).await.as_deref(), Some("02-29"));
    patch(json!({ "birthday_visibility": "private" })).await;
    assert_eq!(
        [
            see(bday(Some(ta.clone())).await),
            see(bday(Some(tb.clone())).await)
        ],
        [true, false]
    );

    // 空白視同清除；null 清除生日
    let (_, _, v) = patch(json!({ "bio": "   ", "birthday": null })).await;
    assert!(v["bio"].is_null() && v["birthday"].is_null());
    let _ = tb;
}
