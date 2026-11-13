//! relation_of 合併成單一查詢的回歸：各關係看到的鍵、清單 / 連結 / 日期數量釘死。
mod common;
use common::{call, ck, user, wishlist, ListOpts, TestUser};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = "../../db/migrations")]
async fn profile_visible_shape_per_relation(pool: PgPool) {
    let a = user(&pool, "A").await;
    sqlx::query("UPDATE users SET handle='alice', birth_year=1990, birthday='05-01', birthday_visibility='friends', interests='{travel}', gift_likes='{tea}', age_vis='friends', interests_vis='close', gift_prefs_vis='public' WHERE id=$1")
        .bind(a.id).execute(&pool).await.unwrap();
    for v in ["public", "friends", "selected", "private"] {
        let l = wishlist(
            &pool,
            a.id,
            ListOpts {
                visibility: Some(v),
                ..Default::default()
            },
        )
        .await;
        common::item(&pool, l.id, "x", 1).await;
    }
    for (t, v) in [
        ("p", "public"),
        ("f", "friends"),
        ("c", "close"),
        ("s", "private"),
    ] {
        sqlx::query("INSERT INTO user_dates(user_id,title,month_day,recurring,visibility) VALUES($1,$2,'06-15',true,$3::profile_vis)")
            .bind(a.id).bind(t).bind(v).execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO user_links(user_id,title,url,position) VALUES($1,'t','https://e.com',0)",
    )
    .bind(a.id)
    .execute(&pool)
    .await
    .unwrap();
    let mut u = vec![];
    for n in ["N", "F", "C", "B", "I", "R", "O"] {
        u.push(user(&pool, n).await);
    }
    let [n, f, c, b, i, r, o] = <[TestUser; 7]>::try_from(u).ok().unwrap();
    common::befriend(&pool, a.id, f.id).await;
    common::befriend(&pool, a.id, c.id).await;
    for (sql, x, y) in [
        (
            "INSERT INTO close_friends(owner_id,friend_id) VALUES($1,$2)",
            a.id,
            c.id,
        ),
        (
            "INSERT INTO user_blocks(blocker_id,blocked_id) VALUES($1,$2)",
            a.id,
            b.id,
        ),
        (
            "INSERT INTO user_blocks(blocker_id,blocked_id) VALUES($1,$2)",
            i.id,
            a.id,
        ),
        (
            "INSERT INTO friend_requests(from_user,to_user) VALUES($1,$2)",
            r.id,
            a.id,
        ),
        (
            "INSERT INTO friend_requests(from_user,to_user) VALUES($1,$2)",
            a.id,
            o.id,
        ),
    ] as [(&str, Uuid, Uuid); 5]
    {
        sqlx::query(sql)
            .bind(x)
            .bind(y)
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut out = vec![];
    for (name, tok) in [
        ("anon", None),
        ("stranger", Some(&n)),
        ("friend", Some(&f)),
        ("close", Some(&c)),
        ("blockedByOwner", Some(&b)),
        ("iBlocked", Some(&i)),
        ("incoming", Some(&r)),
        ("outgoing", Some(&o)),
        ("self", Some(&a)),
    ] {
        let h = tok.map(|t| ck(&t.token)).unwrap_or_default();
        let (s, _, v) = call(&pool, "GET", "/users/alice", &h, None).await;
        let mut keys: Vec<&str> = v["user"]
            .as_object()
            .map(|m| m.keys().map(String::as_str).collect())
            .unwrap_or_default();
        keys.sort();
        let mut vis: Vec<&str> = v["wishlists"]
            .as_array()
            .map(|x| {
                x.iter()
                    .map(|w| w["visibility"].as_str().unwrap())
                    .collect()
            })
            .unwrap_or_default();
        vis.sort();
        out.push(format!(
            "{name}|{}|{}|{}|{}|{keys:?}|{vis:?}|{}|{}",
            s.as_u16(),
            v["relation"],
            v["request_id"].is_null(),
            v["mutual"]["count"],
            v["user"]["links"].as_array().map_or(0, Vec::len),
            v["user"]["dates"].as_array().map_or(0, Vec::len)
        ));
    }
    let got = out.join("\n");
    assert_eq!(got, EXPECTED);
}

const EXPECTED: &str = r#"anon|200|"anonymous"|true|null|["avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "links"]|["public"]|1|1
stranger|200|"none"|true|0|["avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "links"]|["public"]|1|1
friend|200|"friend"|true|0|["age", "avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "links"]|["friends", "public"]|1|2
close|200|"friend"|true|0|["age", "avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "interests", "links"]|["friends", "public", "selected"]|1|3
blockedByOwner|404|null|true|null|[]|[]|0|0
iBlocked|200|"blocked"|true|null|["avatar_url", "bio", "birthday", "display_name", "handle", "id"]|[]|0|0
incoming|200|"outgoing"|false|0|["avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "links"]|["public"]|1|1
outgoing|200|"incoming"|false|0|["avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "links"]|["public"]|1|1
self|200|"self"|true|null|["age", "avatar_url", "bio", "birthday", "cover_image_url", "cover_preset", "dates", "display_name", "gift_dislikes", "gift_likes", "handle", "id", "interests", "links"]|["friends", "public", "selected"]|1|4"#;
