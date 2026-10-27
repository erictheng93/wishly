//! 點數記帳核心（P2-A，無批次 / 無金流）：所有 balance 異動都經 `post`，balance 與 point_ledger 同交易同步。
//! 鎖序（docs/04 D22-12）：wishlists（FOR SHARE）→ point_wallets（FOR UPDATE，多個時依 id 排序）→ wishlist_items → contributions。
//! 因此 `release` / `refund_partial` 會先鎖錢包，呼叫端必須在「動 wishlist_items 之前」呼叫它們。
//! 點數來源：營運人工發放（grant / adjustment）與儲值（topup，見 topup.rs：webhook 驗簽後呼叫 `post`）。
//! 點數批次（point_lots，D55）：`post` 依 kind 建批次 / 依順序扣批次 / 退回原批次，balance 仍是快取（Σ lots.remaining = balance）。
use crate::error::AppError;
use serde_json::json;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub type Tx<'a> = Transaction<'a, Postgres>;

/// 衍生 display_status（D22-6）。需在查詢中以 `i` 代表 wishlist_items、`po` 代表 LEFT JOIN 的 purchase_orders。
pub const DISPLAY_STATUS_SQL: &str = "CASE
    WHEN i.funding_status IS NULL THEN NULL
    WHEN i.funding_status = 'open' THEN 'open'
    WHEN i.funding_status = 'expired' THEN 'expired'
    WHEN i.funding_status = 'fulfilled' OR po.status = 'delivered' THEN 'delivered'
    WHEN po.status = 'shipped' THEN 'shipped'
    WHEN po.status = 'placed' THEN 'ordered'
    ELSE 'funded' END";

pub struct Entry<'a> {
    pub kind: &'a str, // ledger_entry_type：grant | pledge | release | refund | adjustment | topup | cash_refund
    pub ref_type: &'a str, // contribution | manual | topup
    pub ref_id: Option<Uuid>,
    pub note: Option<&'a str>,
    pub actor: Option<Uuid>, // 營運人員（grant / adjustment）
}

impl<'a> Entry<'a> {
    pub fn contribution(kind: &'a str, id: Uuid) -> Self {
        Entry {
            kind,
            ref_type: "contribution",
            ref_id: Some(id),
            note: None,
            actor: None,
        }
    }
}

/// 取得（不存在則建立）使用者錢包 id。
pub async fn ensure_wallet(tx: &mut Tx<'_>, user_id: Uuid) -> Result<Uuid, AppError> {
    sqlx::query("INSERT INTO point_wallets (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING")
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    Ok(
        sqlx::query_scalar("SELECT id FROM point_wallets WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

/// 依 id 排序鎖定錢包（多錢包操作一律先呼叫，避免互鎖）。
pub async fn lock_wallets(tx: &mut Tx<'_>, ids: &[Uuid]) -> Result<(), AppError> {
    sqlx::query("SELECT id FROM point_wallets WHERE id = ANY($1) ORDER BY id FOR UPDATE")
        .bind(ids)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// balance += delta 並依批次寫 ledger（每個受影響批次一列，帶 lot_id），回傳 balance_after。
/// 扣到負數 → 409 INSUFFICIENT_POINTS（附 balance）。不檢查 frozen：凍結只擋使用者主動認捐（由呼叫端檢查），退點仍要能入帳。
/// 批次規則（D23 / D55；呼叫端須已持有錢包列鎖——下面第一句 UPDATE 就會取得，所以 lots 不另鎖）：
/// - 正向 topup：建 source=topup 批次（可退現金，(source, source_ref_id) 唯一防重複入帳）；
/// - 正向 release / refund：依該認捐的 allocations 逆序退回原批次（topup → adjustment → gift_card → reward_bonus）；
/// - 其他正向（grant 等）：建 source=adjustment 批次（不可退現金）；
/// - 負向（pledge / 負數 adjustment）：reward_bonus（最早到期）→ gift_card → adjustment → topup，pledge 另寫 allocations。
pub async fn post(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    delta: i64,
    e: Entry<'_>,
) -> Result<i64, AppError> {
    if delta == 0 {
        return balance(tx, wallet_id).await;
    }
    let after: Option<i64> = sqlx::query_scalar(
        "UPDATE point_wallets SET balance = balance + $2 WHERE id = $1 AND balance + $2 >= 0 RETURNING balance")
        .bind(wallet_id).bind(delta).fetch_optional(&mut **tx).await?;
    let Some(after) = after else {
        let b = balance(tx, wallet_id).await?;
        return Err(insufficient(b));
    };
    match (delta > 0, e.kind) {
        (true, "release" | "refund") => {
            return return_to_allocations(tx, wallet_id, delta, after, &e).await;
        }
        (true, kind) => {
            let (source, refundable) = if kind == "topup" {
                ("topup", true)
            } else {
                ("adjustment", false)
            };
            let lot: Uuid = sqlx::query_scalar(
                "INSERT INTO point_lots (wallet_id, source, source_ref_id, original_points, remaining_points, cash_refundable)
                 VALUES ($1, $2::point_lot_source, $3, $4, $4, $5) RETURNING id")
                .bind(wallet_id).bind(source).bind(e.ref_id).bind(delta).bind(refundable).fetch_one(&mut **tx).await?;
            ledger(tx, wallet_id, lot, delta, after, &e).await?;
        }
        (false, _) => debit(tx, wallet_id, -delta, after, &e).await?,
    }
    Ok(after)
}

fn insufficient(balance: i64) -> AppError {
    AppError::Extra {
        status: 409,
        code: "INSUFFICIENT_POINTS",
        detail: format!("點數餘額不足（目前 {balance} 點）"),
        extra: json!({ "balance": balance }),
    }
}

pub(crate) async fn ledger(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    lot_id: Uuid,
    delta: i64,
    balance_after: i64,
    e: &Entry<'_>,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO point_ledger (wallet_id, lot_id, delta, balance_after, entry_type, ref_type, ref_id, note, actor_id)
                 VALUES ($1, $2, $3, $4, $5::ledger_entry_type, $6, $7, $8, $9)")
        .bind(wallet_id).bind(lot_id).bind(delta).bind(balance_after).bind(e.kind).bind(e.ref_type).bind(e.ref_id).bind(e.note).bind(e.actor)
        .execute(&mut **tx).await?;
    Ok(())
}

/// 依扣點順序從批次扣 `need` 點；`after` 是整筆扣完後的 balance，每列 balance_after 逐筆還原。
async fn debit(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    need: i64,
    after: i64,
    e: &Entry<'_>,
) -> Result<(), AppError> {
    let lots: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT id, remaining_points FROM point_lots
          WHERE wallet_id = $1 AND remaining_points > 0 AND (expires_at IS NULL OR expires_at > now())
          ORDER BY CASE source WHEN 'reward_bonus' THEN 0 WHEN 'gift_card' THEN 1 WHEN 'adjustment' THEN 2 ELSE 3 END,
                   expires_at NULLS LAST, created_at, id")
        .bind(wallet_id).fetch_all(&mut **tx).await?;
    let (mut left, mut running) = (need, after + need);
    for (lot, rem) in lots {
        if left == 0 {
            break;
        }
        let take = rem.min(left);
        sqlx::query("UPDATE point_lots SET remaining_points = remaining_points - $2 WHERE id = $1")
            .bind(lot)
            .bind(take)
            .execute(&mut **tx)
            .await?;
        running -= take;
        ledger(tx, wallet_id, lot, -take, running, e).await?;
        if e.kind == "pledge" {
            sqlx::query("INSERT INTO contribution_lot_allocations (contribution_id, lot_id, points) VALUES ($1, $2, $3)")
                .bind(e.ref_id).bind(lot).bind(take).execute(&mut **tx).await?;
        }
        left -= take;
    }
    if left > 0 {
        return Err(insufficient(after + need)); // 批次可用點數不足（例如已到期）；整筆 rollback
    }
    Ok(())
}

/// 把 `delta` 點依認捐的 allocations 逆序退回原批次（只退尚未退回的部分）。
async fn return_to_allocations(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    delta: i64,
    after: i64,
    e: &Entry<'_>,
) -> Result<i64, AppError> {
    let allocs: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT a.lot_id, a.points - a.refunded_points FROM contribution_lot_allocations a JOIN point_lots l ON l.id = a.lot_id
          WHERE a.contribution_id = $1 AND a.points > a.refunded_points
          ORDER BY CASE l.source WHEN 'topup' THEN 0 WHEN 'adjustment' THEN 1 WHEN 'gift_card' THEN 2 ELSE 3 END, l.created_at DESC, l.id DESC")
        .bind(e.ref_id).fetch_all(&mut **tx).await?;
    let (mut left, mut running) = (delta, after - delta);
    for (lot, avail) in allocs {
        if left == 0 {
            break;
        }
        let take = avail.min(left);
        sqlx::query("UPDATE contribution_lot_allocations SET refunded_points = refunded_points + $3 WHERE contribution_id = $1 AND lot_id = $2")
            .bind(e.ref_id).bind(lot).bind(take).execute(&mut **tx).await?;
        sqlx::query("UPDATE point_lots SET remaining_points = remaining_points + $2 WHERE id = $1")
            .bind(lot)
            .bind(take)
            .execute(&mut **tx)
            .await?;
        running += take;
        ledger(tx, wallet_id, lot, take, running, e).await?;
        running = offset_charged_back(tx, wallet_id, lot, take, running).await?;
        left -= take;
    }
    if left > 0 {
        return Err(AppError::problem(
            500,
            "INTERNAL_ERROR",
            "贊助的點數批次分配不足以退回",
        ));
    }
    Ok(running)
}

/// 退回批次 `lot` 的 `take` 點（已入帳）後呼叫：若該批次是已拒付 topup 的，且欠款未結清（open / written_off），
/// 這些點數拒付時已算進欠款，不可復活——改記為抵銷欠款（recovered_points += take），並把點數再扣出批次與錢包，回傳新的 running。
/// 欠款已 settled（使用者已用現金還清）則不動，點數正常歸使用者。
/// 單一 UPDATE 判斷並抵銷，不與 debt_patch 競態；抵銷超過 amount 會觸發 CHECK 整筆 rollback（不變式守衛）。
/// 鎖序：呼叫端已持有錢包列鎖 → wallet_debts 列；debt_patch 只動欠款列、charge_back 是 topups → 錢包 → 欠款，無環。
async fn offset_charged_back(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    lot: Uuid,
    take: i64,
    running: i64,
) -> Result<i64, AppError> {
    let debt: Option<Uuid> = sqlx::query_scalar(
        "UPDATE wallet_debts d SET recovered_points = d.recovered_points + $2,
                status = CASE WHEN d.status = 'open' AND d.recovered_points + $2 = d.amount THEN 'settled'::wallet_debt_status ELSE d.status END,
                resolved_at = CASE WHEN d.status = 'open' AND d.recovered_points + $2 = d.amount THEN now() ELSE d.resolved_at END,
                note = CASE WHEN d.status = 'open' AND d.recovered_points + $2 = d.amount THEN coalesce(d.note, '退回點數已全數抵銷') ELSE d.note END
          FROM point_lots l JOIN topups t ON t.id = l.source_ref_id
          WHERE l.id = $1 AND l.source = 'topup' AND t.status = 'charged_back' AND d.topup_id = t.id AND d.status IN ('open', 'written_off')
          RETURNING d.id")
        .bind(lot).bind(take).fetch_optional(&mut **tx).await?;
    let Some(debt) = debt else {
        return Ok(running);
    };
    sqlx::query("UPDATE point_lots SET remaining_points = remaining_points - $2 WHERE id = $1")
        .bind(lot)
        .bind(take)
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE point_wallets SET balance = balance - $2 WHERE id = $1")
        .bind(wallet_id)
        .bind(take)
        .execute(&mut **tx)
        .await?;
    let running = running - take;
    let e = Entry {
        kind: "adjustment",
        ref_type: "wallet_debt", // 不落在 point_ledger_topup_uk / ref_uk 的部分唯一索引內，同一欠款可多次抵銷
        ref_id: Some(debt),
        note: Some("信用卡拒付：退回點數抵銷欠款"),
        actor: None,
    };
    ledger(tx, wallet_id, lot, -take, running, &e).await?;
    Ok(running)
}

/// 現金退款申請扣點（4.3h）：只從 cash_refundable 批次先進先出扣；呼叫端須已持有錢包列鎖並檢查未凍結。
/// 餘額不足 → 409 INSUFFICIENT_POINTS；balance 足夠但可退現金的點數不足 → 409 NOT_CASH_REFUNDABLE（附 refundable_points，不扣任何批次）。
/// 回傳扣自各批次的 [(lot_id, points)]，呼叫端存入 cash_refund_requests.lot_allocations。
pub async fn cash_refund_debit(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    amount: i64,
    request_id: Uuid,
) -> Result<Vec<(Uuid, i64)>, AppError> {
    let bal = balance(tx, wallet_id).await?;
    if bal < amount {
        return Err(insufficient(bal));
    }
    let lots: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT id, remaining_points FROM point_lots
          WHERE wallet_id = $1 AND cash_refundable AND remaining_points > 0 ORDER BY created_at, id")
        .bind(wallet_id).fetch_all(&mut **tx).await?;
    let refundable: i64 = lots.iter().map(|l| l.1).sum();
    if refundable < amount {
        return Err(AppError::Extra {
            status: 409,
            code: "NOT_CASH_REFUNDABLE",
            detail: format!("可退現金的點數只有 {refundable} 點（禮物卡與贊助點數不可退現金）"),
            extra: json!({ "refundable_points": refundable, "balance": bal }),
        });
    }
    sqlx::query("UPDATE point_wallets SET balance = balance - $2 WHERE id = $1")
        .bind(wallet_id)
        .bind(amount)
        .execute(&mut **tx)
        .await?;
    let e = Entry {
        kind: "cash_refund",
        ref_type: "cash_refund_request",
        ref_id: Some(request_id),
        note: None,
        actor: None,
    };
    let (mut left, mut running, mut out) = (amount, bal, vec![]);
    for (lot, rem) in lots {
        if left == 0 {
            break;
        }
        let take = rem.min(left);
        sqlx::query("UPDATE point_lots SET remaining_points = remaining_points - $2 WHERE id = $1")
            .bind(lot)
            .bind(take)
            .execute(&mut **tx)
            .await?;
        running -= take;
        ledger(tx, wallet_id, lot, -take, running, &e).await?;
        out.push((lot, take));
        left -= take;
    }
    Ok(out)
}

/// 現金退款被拒絕：依申請時記錄的 lot_allocations 退回原批次，ledger 記 adjustment（4.3i）。
/// 呼叫端須已持有錢包列鎖，並以申請狀態守衛確保只退一次。
pub async fn cash_refund_restore(
    tx: &mut Tx<'_>,
    wallet_id: Uuid,
    request_id: Uuid,
    allocations: &[(Uuid, i64)],
    staff: Uuid,
) -> Result<i64, AppError> {
    let total: i64 = allocations.iter().map(|a| a.1).sum();
    let after: i64 = sqlx::query_scalar(
        "UPDATE point_wallets SET balance = balance + $2 WHERE id = $1 RETURNING balance",
    )
    .bind(wallet_id)
    .bind(total)
    .fetch_one(&mut **tx)
    .await?;
    let e = Entry {
        kind: "adjustment",
        ref_type: "cash_refund_request",
        ref_id: Some(request_id),
        note: Some("現金退款申請被拒絕，點數退回原批次"),
        actor: Some(staff),
    };
    let mut running = after - total;
    for (lot, pts) in allocations {
        sqlx::query("UPDATE point_lots SET remaining_points = remaining_points + $2 WHERE id = $1")
            .bind(lot)
            .bind(pts)
            .execute(&mut **tx)
            .await?;
        running += pts;
        ledger(tx, wallet_id, *lot, *pts, running, &e).await?;
        running = offset_charged_back(tx, wallet_id, *lot, *pts, running).await?;
    }
    Ok(running)
}

async fn balance(tx: &mut Tx<'_>, wallet_id: Uuid) -> Result<i64, AppError> {
    Ok(
        sqlx::query_scalar("SELECT balance FROM point_wallets WHERE id = $1")
            .bind(wallet_id)
            .fetch_one(&mut **tx)
            .await?,
    )
}

/// 退回的一筆認捐
pub struct Released {
    pub contribution_id: Uuid,
    pub item_id: Uuid,
    pub user_id: Uuid,
    pub back: i64,
}

/// 把 `ids` 中狀態屬於 `from`（pledged / captured）的認捐整筆退回：status=released、refunded_points=points，
/// 未退部分（points - refunded_points）以 ledger release 退回錢包。已不符狀態者略過（冪等）。
/// 不更新 wishlist_items.pledged_points：呼叫端在之後自行扣減（鎖序：錢包先、品項後）。
pub async fn release(
    tx: &mut Tx<'_>,
    ids: &[Uuid],
    from: &[&str],
) -> Result<Vec<Released>, AppError> {
    let from: Vec<String> = from.iter().map(|s| s.to_string()).collect();
    let wallets: Vec<Uuid> =
        sqlx::query_scalar("SELECT DISTINCT wallet_id FROM contributions WHERE id = ANY($1)")
            .bind(ids)
            .fetch_all(&mut **tx)
            .await?;
    lock_wallets(tx, &wallets).await?;
    let rows: Vec<(Uuid, Uuid, Uuid, Uuid, i64)> = sqlx::query_as(
        "WITH c AS (SELECT id, points - refunded_points AS back FROM contributions
                     WHERE id = ANY($1) AND status::text = ANY($2) ORDER BY wallet_id, id FOR UPDATE)
         UPDATE contributions x SET status = 'released', released_at = now(), refunded_points = x.points
           FROM c WHERE x.id = c.id
         RETURNING x.id, x.item_id, x.user_id, x.wallet_id, c.back")
        .bind(ids).bind(&from).fetch_all(&mut **tx).await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, item_id, user_id, wallet_id, back) in rows {
        post(tx, wallet_id, back, Entry::contribution("release", id)).await?;
        out.push(Released {
            contribution_id: id,
            item_id,
            user_id,
            back,
        });
    }
    Ok(out)
}

/// 最大餘數法：把 diff 依 weights 比例分成整數，總和恆等於 diff。
/// 同餘數時索引小者優先（呼叫端依 created_at, id 排序傳入 = 先認捐者優先）。
pub fn largest_remainder(diff: i64, weights: &[i64]) -> Vec<i64> {
    let total: i128 = weights.iter().map(|&w| w as i128).sum();
    if total == 0 || diff == 0 {
        return vec![0; weights.len()];
    }
    let mut shares: Vec<i64> = weights
        .iter()
        .map(|&w| (diff as i128 * w as i128 / total) as i64)
        .collect();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by_key(|&k| {
        (
            std::cmp::Reverse(diff as i128 * weights[k] as i128 % total),
            k,
        )
    });
    let leftover = diff - shares.iter().sum::<i64>();
    for &k in order.iter().take(leftover as usize) {
        shares[k] += 1;
    }
    shares
}

/// 4.3f：實際花費低於 target 時，差額 diff 依各 captured 認捐的 points 比例退回（ledger refund，每筆認捐只會發生一次）。
/// 回傳實際退回總額（= diff）。呼叫端須在動 wishlist_items / purchase_orders 狀態之前呼叫（先鎖錢包）。
pub async fn refund_partial(tx: &mut Tx<'_>, item_id: Uuid, diff: i64) -> Result<i64, AppError> {
    if diff <= 0 {
        return Ok(0);
    }
    let wallets: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT wallet_id FROM contributions WHERE item_id = $1 AND status = 'captured'",
    )
    .bind(item_id)
    .fetch_all(&mut **tx)
    .await?;
    lock_wallets(tx, &wallets).await?;
    let rows: Vec<(Uuid, Uuid, i64)> = sqlx::query_as(
        "SELECT id, wallet_id, points FROM contributions WHERE item_id = $1 AND status = 'captured' ORDER BY created_at, id FOR UPDATE")
        .bind(item_id).fetch_all(&mut **tx).await?;
    let shares = largest_remainder(diff, &rows.iter().map(|r| r.2).collect::<Vec<_>>());
    let mut total = 0;
    for ((id, wallet_id, _), share) in rows.iter().zip(shares) {
        if share == 0 {
            continue;
        }
        sqlx::query(
            "UPDATE contributions SET refunded_points = refunded_points + $2 WHERE id = $1",
        )
        .bind(id)
        .bind(share)
        .execute(&mut **tx)
        .await?;
        post(tx, *wallet_id, share, Entry::contribution("refund", *id)).await?;
        total += share;
    }
    Ok(total)
}

/// 4.3b ⑤：品項剛轉 funded（呼叫端已在同交易把 funding_status 設為 funded）後呼叫：
/// 全部 pledged → captured，並以收件資訊快照建立（或重用）採購單。回傳採購單 id。
pub async fn capture_and_order(tx: &mut Tx<'_>, item_id: Uuid) -> Result<Uuid, AppError> {
    sqlx::query("UPDATE contributions SET status = 'captured', captured_at = now() WHERE item_id = $1 AND status = 'pledged'")
        .bind(item_id).execute(&mut **tx).await?;
    let addr: Option<(Vec<u8>, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT s.recipient_name_enc, s.phone_enc, s.address_enc FROM shipping_addresses s
           JOIN wishlist_items i ON i.wishlist_id = s.wishlist_id WHERE i.id = $1",
    )
    .bind(item_id)
    .fetch_optional(&mut **tx)
    .await?;
    let (n, p, a) = addr.ok_or_else(|| {
        AppError::problem(409, "SHIPPING_ADDRESS_REQUIRED", "清單尚未填寫收件資訊")
    })?;
    let open = |b: &[u8]| {
        crate::sealed::open_str(b)
            .ok_or_else(|| AppError::problem(500, "INTERNAL_ERROR", "收件資訊無法解密"))
    };
    let snapshot = crate::sealed::seal(
        json!({ "recipient_name": open(&n)?, "phone": open(&p)?, "address": open(&a)? })
            .to_string()
            .as_bytes(),
    );
    Ok(sqlx::query_scalar(
        "INSERT INTO purchase_orders (item_id, fulfillment_type, amount, shipping_address_snapshot)
         SELECT id, fulfillment_type, target_points, $2 FROM wishlist_items WHERE id = $1
         ON CONFLICT (item_id) DO UPDATE SET status = 'pending', failure_reason = NULL, amount = EXCLUDED.amount,
           shipping_address_snapshot = EXCLUDED.shipping_address_snapshot, merchant_order_id = NULL, tracking_no = NULL,
           operator_user_id = NULL, placed_at = NULL, shipped_at = NULL, delivered_at = NULL
         RETURNING id")
        .bind(item_id).bind(snapshot).fetch_one(&mut **tx).await?)
}

#[cfg(test)]
mod tests {
    use super::largest_remainder as lr;
    #[test]
    fn largest_remainder_sums_to_diff() {
        assert_eq!(lr(100, &[1600, 6000]), vec![21, 79]); // 契約 4.3f 範例
        assert_eq!(lr(10, &[1, 1, 1]), vec![4, 3, 3]); // 同餘數先認捐者優先
        assert_eq!(lr(0, &[5, 5]), vec![0, 0]);
        for (d, w) in [
            (7, vec![3, 3, 3, 1]),
            (999, vec![1, 2, 3, 5, 8]),
            (1, vec![100, 100]),
        ] {
            assert_eq!(lr(d, &w).iter().sum::<i64>(), d);
        }
    }
}
