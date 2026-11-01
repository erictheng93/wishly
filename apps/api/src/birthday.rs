//! 生日清單通知排程（D65）：生日前 7 天通知看得到清單的好友，前 21 天提醒壽星本人開清單；每年一次。
//! 無路由，由 `ratelimit::spawn_cleanup` 每小時呼叫 `birthday_tick`。
use chrono::{Datelike, NaiveDate};
use sqlx::PgPool;
use uuid::Uuid;

const FRIEND_DAYS: i64 = 7;
const NUDGE_DAYS: i64 = 21;

pub async fn birthday_tick(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let today = (chrono::Utc::now() + chrono::Duration::hours(8)).date_naive(); // 台北日期
    birthday_tick_at(pool, today).await
}

/// 生日所屬年份：今天起 `days` 天後的那一天所在年（跨年時為明年）
fn year_of(today: NaiveDate, days: i64) -> i16 {
    (today + chrono::Duration::days(days)).year() as i16
}

/// 回傳排入的通知筆數。補跑邏輯：只要在窗口內（含停機後晚到），且該年尚未寄過，就排一次。
/// 朋友通知以清單為單位（同一人可開多份，對象各自不同），冪等靠 wishlist_birthday_sent 主鍵；壽星提醒靠條件式 UPDATE。多實例同時跑也只有一個拿到 RETURNING。
pub async fn birthday_tick_at(pool: &PgPool, today: NaiveDate) -> Result<u64, sqlx::Error> {
    let mut queued = 0;

    let owners: Vec<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "SELECT u.id, u.birthday, w.id, w.visibility::text
           FROM users u JOIN wishlists w ON w.owner_id = u.id
          WHERE w.birthday_notify AND w.deleted_at IS NULL AND w.status = 'active' AND w.org_id IS NULL
            AND w.moderation_status = 'ok'
            AND u.birthday IS NOT NULL AND u.birthday_visibility::text <> 'private'
            AND u.deleted_at IS NULL AND u.blocked_at IS NULL")
        .fetch_all(pool).await?;
    for (owner, bday, wid, vis) in owners {
        let Some(d) = crate::friends::days_until(today, &bday).filter(|d| *d <= FRIEND_DAYS) else {
            continue;
        };
        let mut tx = pool.begin().await?;
        let won = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO wishlist_birthday_sent (wishlist_id, year) VALUES ($1, $2) ON CONFLICT DO NOTHING RETURNING wishlist_id")
            .bind(wid).bind(year_of(today, d)).fetch_optional(&mut *tx).await?;
        if won.is_some() {
            queued += sqlx::query(
                "INSERT INTO notifications (user_id, channel, kind, payload)
                 SELECT r.id, 'email', 'birthday.friend', jsonb_build_object('wishlist_id', $2::uuid, 'owner_id', $1::uuid, 'birthday', $3::text)
                   FROM users r
                  WHERE r.id <> $1 AND r.deleted_at IS NULL AND r.blocked_at IS NULL
                    AND (COALESCE(r.notification_prefs->>'email_social', 'true') <> 'false' OR COALESCE(r.notification_prefs->>'inbox_social', 'true') <> 'false')
                    AND EXISTS (SELECT 1 FROM friendships f WHERE f.user_a = LEAST(r.id, $1) AND f.user_b = GREATEST(r.id, $1))
                    AND ($4 <> 'selected' OR EXISTS (SELECT 1 FROM wishlist_allowed_users a WHERE a.wishlist_id = $2 AND a.user_id = r.id))")
                .bind(owner).bind(wid).bind(&bday).bind(&vis)
                .execute(&mut *tx).await?.rows_affected();
        }
        tx.commit().await?;
    }

    let nudges: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT u.id, u.birthday FROM users u
          WHERE u.birthday IS NOT NULL AND u.birthday_visibility::text <> 'private'
            AND u.deleted_at IS NULL AND u.blocked_at IS NULL
            AND (COALESCE(u.notification_prefs->>'email_social', 'true') <> 'false' OR COALESCE(u.notification_prefs->>'inbox_social', 'true') <> 'false')
            AND NOT EXISTS (SELECT 1 FROM wishlists w WHERE w.owner_id = u.id AND w.birthday_notify AND w.deleted_at IS NULL AND w.status = 'active')")
        .fetch_all(pool).await?;
    for (uid, bday) in nudges {
        let Some(d) = crate::friends::days_until(today, &bday).filter(|d| *d <= NUDGE_DAYS) else {
            continue;
        };
        queued += sqlx::query(
            "WITH w AS (UPDATE users SET birthday_nudged_year = $2 WHERE id = $1 AND birthday_nudged_year IS DISTINCT FROM $2 RETURNING id)
             INSERT INTO notifications (user_id, channel, kind, payload)
             SELECT id, 'email', 'birthday.nudge', jsonb_build_object('birthday', $3::text) FROM w")
            .bind(uid).bind(year_of(today, d)).bind(&bday)
            .execute(pool).await?.rows_affected();
    }
    Ok(queued)
}
