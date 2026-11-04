//! 檢舉後台第二階段：結案回饋通知、篩選 / 分組、處分紀錄、組織檢舉（0062）。
mod common;
use axum::http::StatusCode;
use common::{call, ck, staff, user};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

async fn report_user(pool: &PgPool, target: Uuid, reporter: Option<Uuid>, status: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO content_reports (reported_user_id, reason, reporter_user_id, status, handled_at) VALUES ($1,'scam',$2,$3::report_status, CASE WHEN $3='open' THEN NULL ELSE now() END) RETURNING id")
        .bind(target).bind(reporter).bind(status).fetch_one(pool).await.unwrap()
}
async fn report_list(
    pool: &PgPool,
    wid: Uuid,
    reporter: Option<Uuid>,
    guest: Option<Uuid>,
) -> Uuid {
    sqlx::query_scalar("INSERT INTO content_reports (wishlist_id, reason, reporter_user_id, reporter_guest_id) VALUES ($1,'scam',$2,$3) RETURNING id")
        .bind(wid).bind(reporter).bind(guest).fetch_one(pool).await.unwrap()
}
async fn report_org(pool: &PgPool, org: Uuid, reporter: Option<Uuid>, status: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO content_reports (reported_org_id, reason, reporter_user_id, status, handled_at) VALUES ($1,'scam',$2,$3::report_status, CASE WHEN $3='open' THEN NULL ELSE now() END) RETURNING id")
        .bind(org).bind(reporter).bind(status).fetch_one(pool).await.unwrap()
}
async fn guest(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO guests (guest_token_hash, display_name) VALUES ($1,'g') RETURNING id",
    )
    .bind(wishly_api::session::hash_token(&Uuid::new_v4().to_string()))
    .fetch_one(pool)
    .await
    .unwrap()
}
async fn status_of(pool: &PgPool, id: Uuid) -> String {
    sqlx::query_scalar("SELECT status::text FROM content_reports WHERE id=$1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}
/// (user_id, payload) of all report.resolved
async fn resolved(pool: &PgPool) -> Vec<(Uuid, Value)> {
    sqlx::query_as(
        "SELECT user_id, payload FROM notifications WHERE kind='report.resolved' ORDER BY user_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}
async fn get_s(pool: &PgPool, tok: &str, uri: String) -> Value {
    get(pool, tok, &uri).await
}
async fn get(pool: &PgPool, tok: &str, uri: &str) -> Value {
    let (s, _, v) = call(pool, "GET", uri, &ck(tok), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn handle_notifies_user_reporter_only_with_outcome(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (owner, rep) = (user(&pool, "owner").await, user(&pool, "rep").await);
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    let g = guest(&pool).await;
    let r1 = report_list(&pool, l.id, Some(rep.id), None).await;
    let r2 = report_list(&pool, l.id, None, Some(g)).await;
    let r3 = report_list(&pool, l.id, None, None).await;
    for r in [r2, r3] {
        let (s, ..) = call(
            &pool,
            "PATCH",
            &format!("/admin/reports/{r}"),
            &ck(&st.token),
            Some(json!({"status":"actioned","note":"秘密備註"})),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
    }
    assert!(resolved(&pool).await.is_empty(), "訪客 / 匿名不通知");
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/reports/{r1}"),
        &ck(&st.token),
        Some(json!({"status":"dismissed","note":"秘密備註"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        resolved(&pool).await,
        vec![(rep.id, json!({"outcome":"dismissed"}))]
    );
    // 重複處理 409，不再多發
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/reports/{r1}"),
        &ck(&st.token),
        Some(json!({"status":"actioned"})),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(resolved(&pool).await.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn moderate_bulk_close_one_per_reporter(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (owner, a, b) = (
        user(&pool, "owner").await,
        user(&pool, "a").await,
        user(&pool, "b").await,
    );
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    let g = guest(&pool).await;
    for (u, gu) in [
        (Some(a.id), None),
        (Some(a.id), None),
        (Some(b.id), None),
        (None, Some(g)),
    ] {
        report_list(&pool, l.id, u, gu).await;
    }
    let dismissed = report_list(&pool, l.id, Some(owner.id), None).await;
    sqlx::query("UPDATE content_reports SET status='dismissed', handled_at=now() WHERE id=$1")
        .bind(dismissed)
        .execute(&pool)
        .await
        .unwrap();
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/wishlists/{}/moderation", l.id),
        &ck(&st.token),
        Some(json!({"moderation_status":"hidden","reason":"違規"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let n = resolved(&pool).await;
    let mut want = vec![a.id, b.id];
    want.sort();
    assert_eq!(n.iter().map(|x| x.0).collect::<Vec<_>>(), want);
    assert!(n.iter().all(|x| x.1 == json!({"outcome":"actioned"})));
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn suspend_bulk_close_notifies_once_per_reporter(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (bad, a) = (user(&pool, "bad").await, user(&pool, "a").await);
    report_user(&pool, bad.id, Some(a.id), "open").await;
    report_user(&pool, bad.id, Some(a.id), "open").await;
    report_user(&pool, bad.id, None, "open").await;
    let (s, ..) = call(
        &pool,
        "PATCH",
        &format!("/admin/users/{}", bad.id),
        &ck(&st.token),
        Some(json!({"blocked":true,"reason":"違規"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        resolved(&pool).await,
        vec![(a.id, json!({"outcome":"actioned"}))]
    );
    // 再次更新（已停權）不重發
    call(
        &pool,
        "PATCH",
        &format!("/admin/users/{}", bad.id),
        &ck(&st.token),
        Some(json!({"blocked":true,"reason":"again"})),
    )
    .await;
    assert_eq!(resolved(&pool).await.len(), 1);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_reject_closes_only_open_reports_and_notifies(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (owner, a) = (user(&pool, "owner").await, user(&pool, "a").await);
    let o = common::org(&pool, owner.id, "verified").await;
    let open1 = report_org(&pool, o, Some(a.id), "open").await;
    let open2 = report_org(&pool, o, Some(a.id), "open").await;
    let anon = report_org(&pool, o, None, "open").await;
    let dismissed = report_org(&pool, o, Some(owner.id), "dismissed").await;
    let (s, _, b) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st.token),
        Some(json!({"decision":"reject","note":"假冒"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    for r in [open1, open2, anon] {
        assert_eq!(status_of(&pool, r).await, "actioned");
    }
    assert_eq!(
        status_of(&pool, dismissed).await,
        "dismissed",
        "已駁回的檢舉不可被撤銷組織覆寫"
    );
    let note: Option<String> =
        sqlx::query_scalar("SELECT resolution_note FROM content_reports WHERE id=$1")
            .bind(open1)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(note.as_deref(), Some("假冒"));
    assert_eq!(
        resolved(&pool).await,
        vec![(a.id, json!({"outcome":"actioned"}))]
    );
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_reject_before_verified_leaves_reports(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let owner = user(&pool, "owner").await;
    let o = common::org(&pool, owner.id, "pending").await;
    let r = report_org(&pool, o, None, "open").await;
    let (s, ..) = call(
        &pool,
        "POST",
        &format!("/admin/orgs/{o}/review"),
        &ck(&st.token),
        Some(json!({"decision":"reject","note":"不符"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(status_of(&pool, r).await, "open");
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn list_filters_reason_dates_and_targets(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (owner, bad) = (user(&pool, "owner").await, user(&pool, "bad").await);
    let l = common::wishlist(&pool, owner.id, Default::default()).await;
    let o = common::org(&pool, owner.id, "verified").await;
    let early = report_list(&pool, l.id, None, None).await; // 台北 3/10 23:59:59
    let late = report_list(&pool, l.id, None, None).await; // 台北 3/11 00:00:00
    sqlx::query("UPDATE content_reports SET created_at='2026-03-10 15:59:59+00' WHERE id=$1")
        .bind(early)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE content_reports SET created_at='2026-03-10 16:00:00+00', reason='copyright' WHERE id=$1").bind(late).execute(&pool).await.unwrap();
    let u = report_user(&pool, bad.id, None, "open").await;
    let og = report_org(&pool, o, None, "open").await;
    let ids = |v: &Value| {
        let mut x: Vec<String> = v["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        x.sort();
        x
    };
    let want = |a: &[Uuid]| {
        let mut x: Vec<String> = a.iter().map(|i| i.to_string()).collect();
        x.sort();
        x
    };
    let q = |s: &str| get_s(&pool, &st.token, format!("/admin/reports{s}"));
    assert_eq!(ids(&q("?reason=copyright").await), want(&[late]));
    assert_eq!(
        ids(&q("?from=2026-03-11&to=2026-03-11").await),
        want(&[late])
    );
    assert_eq!(
        ids(&q("?from=2026-03-10&to=2026-03-10").await),
        want(&[early])
    );
    assert_eq!(ids(&q("?to=2026-03-10").await), want(&[early]));
    assert_eq!(
        ids(&q(&format!("?wishlist_id={}", l.id)).await),
        want(&[early, late])
    );
    assert_eq!(ids(&q(&format!("?user_id={}", bad.id)).await), want(&[u]));
    assert_eq!(ids(&q(&format!("?org_id={o}")).await), want(&[og]));
    assert_eq!(ids(&q("?kind=org").await), want(&[og]));
    let org = &q("?kind=org").await["data"][0]["org"];
    assert_eq!(org["verification_status"], "verified");
    assert_eq!(org["display_name"], "測試協會");
    for bad_q in ["?reason=x", "?from=2026-13-01", "?to=abc", "?kind=x"] {
        let (s, ..) = call(
            &pool,
            "GET",
            &format!("/admin/reports{bad_q}"),
            &ck(&st.token),
            None,
        )
        .await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{bad_q}");
    }

    // targets：清單 2 件 > 使用者 1 件 / 組織 1 件（同件數依 latest_at 新到舊）
    let t = get(&pool, &st.token, "/admin/reports/targets").await;
    let d = t["data"].as_array().unwrap();
    assert_eq!(d.len(), 3);
    assert_eq!(
        (d[0]["kind"].as_str(), d[0]["count"].as_i64()),
        (Some("wishlist"), Some(2))
    );
    assert_eq!(d[0]["reasons"], json!(["copyright", "scam"]));
    assert_eq!(d[0]["wishlist"]["owner"]["id"], json!(owner.id));
    assert!(d[0]["user"].is_null() && d[0]["org"].is_null());
    assert!(d[0]["first_at"].as_str() < d[0]["latest_at"].as_str());
    assert_eq!(d[1]["kind"], "org"); // 較晚建立
    assert_eq!(d[1]["org"]["id"], json!(o));
    assert_eq!(d[2]["kind"], "user");
    assert_eq!(d[2]["user"]["suspended"], false);
    assert!(t["next_offset"].is_null());
    // 篩選與分頁
    let p = get(&pool, &st.token, "/admin/reports/targets?limit=2").await;
    assert_eq!(
        (
            p["data"].as_array().unwrap().len(),
            p["next_offset"].as_i64()
        ),
        (2, Some(2))
    );
    let p = get(&pool, &st.token, "/admin/reports/targets?limit=2&offset=2").await;
    assert_eq!(
        (
            p["data"].as_array().unwrap().len(),
            p["next_offset"].is_null()
        ),
        (1, true)
    );
    let f = get(
        &pool,
        &st.token,
        "/admin/reports/targets?kind=wishlist&reason=scam",
    )
    .await;
    assert_eq!(
        (
            f["data"].as_array().unwrap().len(),
            f["data"][0]["count"].as_i64()
        ),
        (1, Some(1))
    );
    let f = get(
        &pool,
        &st.token,
        "/admin/reports/targets?from=2026-03-11&to=2026-03-11",
    )
    .await;
    assert_eq!(f["data"][0]["count"], 1);
    let (s, ..) = call(
        &pool,
        "GET",
        "/admin/reports/targets?status=x",
        &ck(&st.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let plain = user(&pool, "plain").await;
    let (s, ..) = call(
        &pool,
        "GET",
        "/admin/reports/targets",
        &ck(&plain.token),
        None,
    )
    .await;
    assert_ne!(s, StatusCode::OK);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn moderation_history_counts_recent_and_suspensions(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (bad, other) = (user(&pool, "bad").await, user(&pool, "other").await);
    let l = common::wishlist(&pool, bad.id, Default::default()).await;
    report_user(&pool, bad.id, None, "open").await;
    report_user(&pool, bad.id, None, "dismissed").await;
    report_list(&pool, l.id, None, None).await;
    report_user(&pool, other.id, None, "open").await; // 別人的不算
    let o = common::org(&pool, other.id, "verified").await;
    report_org(&pool, o, None, "open").await; // 組織檢舉不進使用者歷史
    let u = format!("/admin/users/{}", bad.id);
    // 只改 is_staff 不算停權事件；停權 → 解除
    call(
        &pool,
        "PATCH",
        &u,
        &ck(&st.token),
        Some(json!({"is_staff":false,"reason":"noop"})),
    )
    .await;
    call(
        &pool,
        "PATCH",
        &u,
        &ck(&st.token),
        Some(json!({"blocked":true,"reason":"違規"})),
    )
    .await;
    call(
        &pool,
        "PATCH",
        &u,
        &ck(&st.token),
        Some(json!({"blocked":false,"reason":"誤判"})),
    )
    .await;

    let h = get(
        &pool,
        &st.token,
        &format!("/admin/users/{}/moderation-history", bad.id),
    )
    .await;
    // 停權只結案針對使用者本人的 open 檢舉，清單檢舉仍 open
    assert_eq!(
        h["counts"],
        json!({"total": 3, "open": 1, "actioned": 1, "dismissed": 1})
    );
    let rec = h["recent"].as_array().unwrap();
    assert_eq!(rec.len(), 3);
    let wl = rec.iter().find(|r| r["target"] == "wishlist").unwrap();
    assert_eq!(wl["wishlist"]["id"], json!(l.id));
    assert!(rec
        .iter()
        .filter(|r| r["target"] == "user")
        .all(|r| r["wishlist"].is_null()));
    let sus = h["suspensions"].as_array().unwrap();
    assert_eq!(sus.len(), 2);
    assert_eq!(
        (sus[0]["blocked"].clone(), sus[0]["reason"].clone()),
        (json!(false), json!("誤判"))
    );
    assert_eq!(
        (sus[1]["blocked"].clone(), sus[1]["reason"].clone()),
        (json!(true), json!("違規"))
    );
    assert_eq!(sus[0]["staff"]["id"], json!(st.id));
    let mut keys: Vec<_> = sus[0].as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["at", "blocked", "reason", "staff"]);

    let (s, ..) = call(
        &pool,
        "GET",
        &format!("/admin/users/{}/moderation-history", Uuid::new_v4()),
        &ck(&st.token),
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "../../db/migrations")]
async fn org_report_create_verified_only_dedupe_and_admin_output(pool: PgPool) {
    let st = staff(&pool, "s").await;
    let (owner, rep) = (user(&pool, "owner").await, user(&pool, "rep").await);
    let ver = common::org(&pool, owner.id, "verified").await;
    let pend = common::org(&pool, owner.id, "pending").await;
    let slug_of = |id: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT slug::text FROM organizations WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let (vs, ps) = (slug_of(ver).await, slug_of(pend).await);
    sqlx::query("UPDATE organizations SET intro='簡介', website='https://org.example' WHERE id=$1")
        .bind(ver)
        .execute(&pool)
        .await
        .unwrap();

    let (s, ..) = call(
        &pool,
        "POST",
        &format!("/public/orgs/{ps}/reports"),
        &[],
        Some(json!({"reason":"scam"})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let url = format!("/public/orgs/{vs}/reports");
    let (s, ..) = call(&pool, "POST", &url, &[], Some(json!({"reason":"nope"}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    let (s, _, a) = call(
        &pool,
        "POST",
        &url,
        &ck(&rep.token),
        Some(json!({"reason":"scam","detail":"假的"})),
    )
    .await;
    assert_eq!(
        (s, a["status"].as_str()),
        (StatusCode::CREATED, Some("open"))
    );
    let (_, _, a2) = call(
        &pool,
        "POST",
        &url,
        &ck(&rep.token),
        Some(json!({"reason":"scam"})),
    )
    .await;
    assert_eq!(a2["id"], a["id"], "24 小時內同人重送回同一筆");
    let (s, ..) = call(&pool, "POST", &url, &[], Some(json!({"reason":"other"}))).await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, ..) = call(&pool, "POST", &url, &[], Some(json!({"reason":"other"}))).await;
    assert_eq!(s, StatusCode::TOO_MANY_REQUESTS, "匿名同組織 24 小時 1 筆");
    sqlx::query("UPDATE organizations SET display_name='改名', intro='新' WHERE id=$1")
        .bind(ver)
        .execute(&pool)
        .await
        .unwrap();

    let all = get(&pool, &st.token, "/admin/reports?kind=org").await;
    let rows = all["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let r = rows.iter().find(|r| r["id"] == a["id"]).unwrap();
    assert_eq!(r["org"]["slug"], json!(vs));
    assert_eq!(r["org"]["display_name"], "改名");
    assert!(r["wishlist"].is_null() && r["user"].is_null());
    assert_eq!(r["target_open_count"], 2);
    assert_eq!(r["snapshot"]["display_name"], "測試協會");
    assert_eq!(r["snapshot"]["intro"], "簡介");
    assert!(r["snapshot"]["logo_url"].is_null() && r["snapshot"].get("logo_key").is_none());
    assert_eq!(r["detail"], "假的");
    assert_eq!(r["reporter_user"]["id"], json!(rep.id));
    // 單目標 CHECK：不能同時指向兩種
    let both = sqlx::query("INSERT INTO content_reports (reported_org_id, reported_user_id, reason) VALUES ($1,$2,'scam')").bind(ver).bind(owner.id).execute(&pool).await;
    assert!(both.is_err());
}
