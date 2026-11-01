//! 支持品項：指向另一份公開清單，不計入完成度（設計見 docs/superpowers/specs/2026-11-01-support-item-design.md）。
use crate::error::AppError;
use serde_json::{json, Value};
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::HashMap;
use uuid::Uuid;

type Tx<'a> = Transaction<'a, Postgres>;

/// 清單可公開提供（別名 `w` 清單、`o` 擁有者）；與公開頁同一套規則。
pub const TARGET_LIVE_SQL: &str = concat!(
    "w.deleted_at IS NULL AND w.visibility = 'public' AND w.status = 'active' AND w.moderation_status = 'ok' \
     AND (w.closes_at IS NULL OR w.closes_at > now()) AND o.deleted_at IS NULL AND o.blocked_at IS NULL \
     AND (w.type <> 'relief' OR (",
    crate::public::live_site!(),
    "))"
);

fn err(code: &'static str, detail: &'static str) -> AppError {
    AppError::Problem {
        status: 422,
        code,
        detail: detail.into(),
        errors: Some(json!([{ "pointer": "/support_slug", "code": code, "detail": detail }])),
        retry_after: None,
    }
}

/// 以 slug 找支持對象；不存在、非公開、已下架一律同一個錯誤，避免探測他人清單。
pub async fn resolve_target(tx: &mut Tx<'_>, slug: &str, uid: Uuid) -> Result<Uuid, AppError> {
    let row: Option<(Uuid, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT w.id, can_manage_wishlist(w.owner_id, w.org_id, $2) FROM wishlists w
         JOIN users o ON o.id = w.owner_id WHERE w.slug = $1 AND {TARGET_LIVE_SQL}"
    )))
    .bind(slug)
    .bind(uid)
    .fetch_optional(&mut **tx)
    .await?;
    match row {
        None => Err(err("SUPPORT_TARGET_INVALID", "找不到可支持的清單。")),
        Some((_, true)) => Err(err("SUPPORT_TARGET_SELF", "不能支持自己管理的清單。")),
        Some((id, false)) => Ok(id),
    }
}

/// 通知尚未通知過的支持對象（清單已發佈時才呼叫）。一條語句同時標記與寫入，與業務寫入同交易；
/// 收件人：個人清單是擁有者，組織清單是有效的 owner / coordinator。
pub async fn notify_targets(tx: &mut Tx<'_>, wishlist_id: Uuid) -> Result<(), AppError> {
    sqlx::query(
        "WITH t AS (
           UPDATE wishlist_items SET support_notified_at = now()
            WHERE wishlist_id = $1 AND funding_mode = 'support' AND support_notified_at IS NULL
              AND deleted_at IS NULL AND support_wishlist_id IS NOT NULL
        RETURNING support_wishlist_id)
         INSERT INTO notifications (user_id, channel, kind, payload)
         SELECT r.uid, 'email', 'wishlist.supported',
                jsonb_build_object('wishlist_id', tw.id, 'supporter_name', COALESCE(so.display_name, su.display_name),
                                    'supporter_id', COALESCE(sw.org_id, sw.owner_id))
           FROM t
           JOIN wishlists tw ON tw.id = t.support_wishlist_id
           JOIN wishlists sw ON sw.id = $1
           JOIN users su ON su.id = sw.owner_id
           LEFT JOIN organizations so ON so.id = sw.org_id
          CROSS JOIN LATERAL (
                SELECT tw.owner_id AS uid WHERE tw.org_id IS NULL
                UNION
                SELECT m.user_id FROM org_members m
                 WHERE m.org_id = tw.org_id AND m.role IN ('owner', 'coordinator') AND m.disabled_at IS NULL) r
           JOIN users ru ON ru.id = r.uid AND ru.deleted_at IS NULL AND ru.blocked_at IS NULL
          WHERE NOT EXISTS (SELECT 1 FROM notifications n
                             WHERE n.user_id = r.uid AND n.kind = 'wishlist.supported'
                               AND n.payload->>'wishlist_id' = tw.id::text
                               AND n.payload->>'supporter_id' = COALESCE(sw.org_id, sw.owner_id)::text
                               AND n.created_at > now() - interval '24 hours')",
    )
    .bind(wishlist_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// 歸因來源驗證：只看品項指向，不看對方是否仍公開（點擊後才下架仍要記）。不成立回 None；
/// 在開交易前呼叫、不加鎖，不影響捐助路徑的鎖序。
pub async fn valid_via(
    pool: &PgPool,
    via: Option<Uuid>,
    target_wishlist: Uuid,
) -> Result<Option<Uuid>, AppError> {
    let Some(via) = via else { return Ok(None) };
    Ok(sqlx::query_scalar(
        "SELECT i.id FROM wishlist_items i WHERE i.id = $1 AND i.deleted_at IS NULL
           AND i.funding_mode = 'support' AND i.support_wishlist_id = $2",
    )
    .bind(via)
    .bind(target_wishlist)
    .fetch_optional(pool)
    .await?)
}

type SumRow = (
    Uuid,
    bool,
    bool,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    i64,
);

/// 支持品項的成果彙總（`support` 物件）。對方已不可公開時 `ended`、`target` 為 null；
/// 對方是鎖定中的驚喜清單時數字為 null。不含任何捐助者資料。
pub async fn summaries(pool: &PgPool, item_ids: &[Uuid]) -> Result<HashMap<Uuid, Value>, AppError> {
    let rows: Vec<SumRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT i.id, COALESCE(({TARGET_LIVE_SQL}), false) AS live,
                COALESCE(surprise_locked(w.surprise_mode, w.event_date), false) AS locked,
                w.slug::text, w.title, w.cover_image_key, w.cover_image_status::text, w.cover_preset,
                COALESCE(og.display_name, o.display_name),
                (SELECT COALESCE(sum(c.points - c.refunded_points), 0)::bigint FROM contributions c
                  WHERE c.via_item_id = i.id AND c.status IN ('pledged','captured')),
                (SELECT COALESCE(sum(c.qty), 0)::bigint FROM claims c
                  WHERE c.via_item_id = i.id AND c.status IN ('reserved','purchased','delivered'))
           FROM wishlist_items i
           LEFT JOIN wishlists w ON w.id = i.support_wishlist_id
           LEFT JOIN users o ON o.id = w.owner_id
           LEFT JOIN organizations og ON og.id = w.org_id
          WHERE i.id = ANY($1) AND i.funding_mode = 'support'"
    )))
    .bind(item_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, live, locked, slug, title, key, st, preset, owner, pts, qty)| {
            let target = live.then(|| json!({
                "slug": slug, "title": title, "cover_image_url": crate::public::image_url(key.as_deref(), st.as_deref().unwrap_or("")),
                "cover_preset": preset, "owner_name": owner,
            }));
            let v = json!({
                "ended": !live, "target": target,
                "points": (!locked).then_some(pts), "qty": (!locked).then_some(qty),
            });
            (id, v)
        })
        .collect())
}
