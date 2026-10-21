//! 公開頁 SSE 即時更新。（擁有者儀表板 GET /wishlists/{id}/dashboard 由 wishlists.rs 實作。）
//!
//! SSE 機制：捐助 / 清單變更後呼叫 `dashboard::notify(executor, wishlist_id)`（Postgres
//! `pg_notify('wishlist_events', id)`，在交易內呼叫則 commit 後才送出）。每個 process 只開一條
//! PgListener（首位訂閱者出現時惰性啟動），轉成 tokio broadcast；各 SSE 連線收到自己清單的 id
//! 就重新查詢並只推「有變動」的品項 + wishlist.updated。事件只含數量彙總，不含捐助者。
use crate::{error::AppError, AppState};
use axum::{
    extract::{Path, State},
    http::{header, request::Parts, HeaderValue},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::get,
    Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, OnceCell, Semaphore, SemaphorePermit};
use uuid::Uuid;

const CHANNEL: &str = "wishlist_events";

pub fn routes() -> Router<AppState> {
    Router::new().route("/public/wishlists/{slug}/events", get(events))
}

/// 給 claims / wishlists 模組呼叫：通知 SSE 訂閱者此清單有變動。
pub async fn notify<'e, E: sqlx::PgExecutor<'e>>(
    ex: E,
    wishlist_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(CHANNEL)
        .bind(wishlist_id.to_string())
        .execute(ex)
        .await?;
    Ok(())
}

// ---------- SSE ----------

/// 匿名 SSE 連線上限：全域與每個來源 IP（超過就拒絕，避免耗盡 DB pool / 記憶體）。
const MAX_SSE_TOTAL: usize = 2000;
const MAX_SSE_PER_IP: u32 = 16;
static SSE_TOTAL: Semaphore = Semaphore::const_new(MAX_SSE_TOTAL);
static SSE_PER_IP: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

/// 持有期間佔用一個全域名額與該 IP 一個計數；drop 時歸還。
struct SseSlot {
    _permit: SemaphorePermit<'static>,
    ip: String,
}

impl SseSlot {
    fn acquire(ip: String) -> Result<Self, AppError> {
        let permit = SSE_TOTAL
            .try_acquire()
            .map_err(|_| AppError::problem(503, "SSE_CAPACITY", "即時更新連線已滿，請稍後再試"))?;
        let mut m = SSE_PER_IP.lock().unwrap();
        let n = m
            .get_or_insert_with(HashMap::new)
            .entry(ip.clone())
            .or_insert(0);
        if *n >= MAX_SSE_PER_IP {
            return Err(AppError::problem(429, "RATE_LIMITED", "即時更新連線過多"));
        }
        *n += 1;
        Ok(SseSlot {
            _permit: permit,
            ip,
        })
    }
}

impl Drop for SseSlot {
    fn drop(&mut self) {
        let mut g = SSE_PER_IP.lock().unwrap();
        if let Some(m) = g.as_mut() {
            if let Some(n) = m.get_mut(&self.ip) {
                *n -= 1;
                if *n == 0 {
                    m.remove(&self.ip);
                }
            }
        }
    }
}

/// 每次 pg_notify 收到時配一個遞增序號；同一 (清單, 序號) 的 snapshot 只查一次，由所有訂閱者共用。
static NOTIFY_SEQ: AtomicU64 = AtomicU64::new(0);
type SnapCell = Arc<OnceCell<Option<Arc<Snap>>>>;
static MEMO: Mutex<Option<HashMap<Uuid, (u64, SnapCell)>>> = Mutex::new(None);
/// snapshot() 實際打到 DB 的次數（測試用，驗證 DB 工作量不隨訂閱者數成長）。
pub static SNAPSHOT_QUERIES: AtomicU64 = AtomicU64::new(0);

async fn snapshot_shared(
    pool: &PgPool,
    wid: Uuid,
    seq: u64,
) -> Result<Option<Arc<Snap>>, sqlx::Error> {
    let cell = {
        let mut g = MEMO.lock().unwrap();
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() > 1024 {
            m.clear(); // ponytail: 粗暴清空，下次重建；有量再改 LRU
        }
        match m.get(&wid) {
            Some((q, c)) if *q == seq => c.clone(),
            Some((q, _)) if *q > seq => Arc::default(), // 落後的訂閱者：自己查，不覆蓋較新的
            _ => {
                let c = SnapCell::default();
                m.insert(wid, (seq, c.clone()));
                c
            }
        }
    };
    cell.get_or_try_init(|| async { Ok(snapshot(pool, wid).await?.map(Arc::new)) })
        .await
        .cloned()
}

static BUS: OnceCell<broadcast::Sender<(Uuid, u64)>> = OnceCell::const_new();

async fn bus(pool: &PgPool) -> Result<&'static broadcast::Sender<(Uuid, u64)>, sqlx::Error> {
    BUS.get_or_try_init(|| async {
        // 回應 SSE 之前完成 LISTEN，避免首個捐助通知在 listener 啟動前遺失。
        let mut listener = sqlx::postgres::PgListener::connect_with(pool).await?;
        listener.listen(CHANNEL).await?;
        let (tx, _) = broadcast::channel(256);
        let (pool, t) = (pool.clone(), tx.clone());
        tokio::spawn(async move {
            loop {
                while let Ok(n) = listener.recv().await {
                    if let Ok(id) = n.payload().parse() {
                        let _ = t.send((id, NOTIFY_SEQ.fetch_add(1, Ordering::Relaxed) + 1));
                    }
                }
                // PgListener::recv 原生支援 pool 關閉事件；關閉後不可再重連。
                if pool.is_closed() {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await; // 斷線重連
                loop {
                    if pool.is_closed() {
                        return;
                    }
                    if let Ok(mut next) = sqlx::postgres::PgListener::connect_with(&pool).await {
                        if next.listen(CHANNEL).await.is_ok() {
                            listener = next;
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        });
        Ok::<_, sqlx::Error>(tx)
    })
    .await
}

/// 單一品項的 SSE 比對單位：任何一項變動就推 item.updated（pledged_points / funding_status / display_status 為眾籌欄位）
#[derive(Clone, PartialEq)]
struct ItemSnap {
    needed: i32,
    claimed: i32,
    mode: String,
    pledged: i64,
    target: Option<i64>,
    funding_status: Option<String>,
    display_status: Option<String>,
}

struct Snap {
    owner_id: Uuid,
    visibility: String,
    status: String,
    title: String,
    hidden: bool,
    items: HashMap<Uuid, ItemSnap>,
}

async fn snapshot(pool: &PgPool, wid: Uuid) -> Result<Option<Snap>, sqlx::Error> {
    SNAPSHOT_QUERIES.fetch_add(1, Ordering::Relaxed);
    let w: Option<(String, String, String, Uuid, String)> = sqlx::query_as(
        "SELECT status::text, title, moderation_status::text, owner_id, visibility::text FROM wishlists WHERE id = $1 AND deleted_at IS NULL").bind(wid).fetch_optional(pool).await?;
    let Some((status, title, m, owner_id, visibility)) = w else {
        return Ok(None);
    };
    type SnapshotRow = (
        Uuid,
        i32,
        i32,
        String,
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
    );
    let items: Vec<SnapshotRow> = sqlx::query_as(
        sqlx::AssertSqlSafe(format!("SELECT i.id, i.qty_needed, i.qty_claimed, i.funding_mode::text, i.pledged_points, i.target_points, i.funding_status::text, ({}) AS display_status
                    FROM wishlist_items i LEFT JOIN purchase_orders po ON po.item_id = i.id WHERE i.wishlist_id = $1 AND i.deleted_at IS NULL", crate::points::DISPLAY_STATUS_SQL)))
        .bind(wid).fetch_all(pool).await?;
    Ok(Some(Snap {
        owner_id,
        visibility,
        status,
        title,
        hidden: m == "hidden",
        items: items
            .into_iter()
            .map(|i| {
                (
                    i.0,
                    ItemSnap {
                        needed: i.1,
                        claimed: i.2,
                        mode: i.3,
                        pledged: i.4,
                        target: i.5,
                        funding_status: i.6,
                        display_status: i.7,
                    },
                )
            })
            .collect(),
    }))
}

fn units(s: &ItemSnap) -> (i64, i64) {
    crate::wishlists::units(&s.mode, s.needed, s.claimed, s.funding_status.as_deref())
}

fn item_event(id: Uuid, s: &ItemSnap, deleted: bool) -> Value {
    let cf = s.mode == "crowdfund";
    let (needed, claimed) = (s.needed, s.claimed);
    let mut v = json!({ "item_id": id, "qty_needed": needed, "qty_claimed": claimed, "qty_remaining": (needed - claimed).max(0),
        "is_fully_claimed": if cf { units(s).0 >= 1 } else { claimed >= needed },
        "pledged_points": cf.then_some(s.pledged), "target_points": s.target, "remaining_points": s.target.filter(|_| cf).map(|t| (t - s.pledged).max(0)),
        "funding_status": s.funding_status, "display_status": s.display_status, "updated_at": chrono::Utc::now() });
    if deleted {
        v["deleted"] = json!(true);
    }
    v
}

async fn events(
    State(st): State<AppState>,
    Path(slug): Path<String>,
    parts: Parts,
) -> Result<Response, AppError> {
    let slot = SseSlot::acquire(crate::config::client_ip(&parts, &crate::config::get()))?;
    let w: Option<(Uuid, String, Uuid, String)> = sqlx::query_as(
        "SELECT id, moderation_status::text, owner_id, visibility::text FROM wishlists
          WHERE slug = $1 AND deleted_at IS NULL AND status IN ('active', 'closed')",
    )
    .bind(&slug)
    .fetch_optional(&st.pool)
    .await?;
    let (wid, m, owner_id, vis) = w.ok_or(AppError::NotFound)?;
    let viewer = crate::access::viewer(&parts, &st).await?;
    crate::access::check_wishlist(
        &st.pool,
        &crate::access::ListAccess {
            wishlist_id: wid,
            owner_id,
            visibility: &vis,
        },
        viewer,
    )
    .await?;
    if m == "hidden" {
        return Err(AppError::WishlistRemoved);
    }
    let mut rx = bus(&st.pool).await?.subscribe(); // 先訂閱再取快照，避免漏事件
    let mut seen = snapshot(&st.pool, wid)
        .await?
        .map(|s| s.items)
        .unwrap_or_default();
    let (tx, out) = mpsc::channel::<Result<Event, Infallible>>(16);
    let pool = st.pool.clone();
    tokio::spawn(async move {
        let _slot = slot; // 連線結束才歸還名額
        let deadline = tokio::time::sleep(Duration::from_secs(30 * 60)); // 每條連線最長 30 分鐘
        tokio::pin!(deadline);
        let mut seq = 0u64;
        let mut nseq;
        // 好友關係/指定名單變動不會 notify，定期重驗非公開清單的存取權
        let mut recheck = tokio::time::interval(Duration::from_secs(60));
        recheck.tick().await;
        loop {
            let mut periodic = false;
            tokio::select! {
                _ = &mut deadline => break,
                // 定期重驗沿用最新共用快照：清單欄位變更都會 notify，好友/指定名單由 check_wishlist 即時查
                _ = recheck.tick() => { periodic = true; nseq = NOTIFY_SEQ.load(Ordering::Relaxed); }
                _ = tx.closed() => break,
                r = rx.recv() => match r {
                    Ok((id, _)) if id != wid => continue,
                    Ok((_, n)) => nseq = n,
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(_) => nseq = NOTIFY_SEQ.load(Ordering::Relaxed), // Lagged：以最新序號重查
                },
            }
            let Ok(snap) = snapshot_shared(&pool, wid, nseq).await else {
                continue;
            };
            // 清單已刪除、封存/下架為非 active/closed，或觀看者失去存取權：不再推送，直接關閉連線
            let Some(s) = snap.filter(|s| matches!(s.status.as_str(), "active" | "closed")) else {
                break;
            };
            if s.visibility != "public"
                && crate::access::check_wishlist(
                    &pool,
                    &crate::access::ListAccess {
                        wishlist_id: wid,
                        owner_id: s.owner_id,
                        visibility: &s.visibility,
                    },
                    viewer,
                )
                .await
                .is_err()
            {
                break;
            }
            if periodic {
                continue;
            }
            let mut evs: Vec<(&str, Value)> = vec![];
            if !s.hidden {
                for (id, cur) in &s.items {
                    if seen.get(id) != Some(cur) {
                        evs.push(("item.updated", item_event(*id, cur, false)));
                    }
                }
                for (id, old) in seen.iter() {
                    if !s.items.contains_key(id) {
                        evs.push(("item.updated", item_event(*id, old, true)));
                    }
                }
            }
            let fulfilled = s
                .items
                .values()
                .filter(|i| {
                    let (c, n) = units(i);
                    c >= n
                })
                .count();
            evs.push(("wishlist.updated", json!({ "status": s.status, "title": s.title,
                "completion": { "item_count": s.items.len(), "fulfilled_count": fulfilled, "completion_pct": crate::wishlists::completion_pct(s.items.values().map(|i| units(i).0).sum(), s.items.values().map(|i| units(i).1).sum()) },
                "updated_at": chrono::Utc::now() })));
            seen = s.items.clone();
            for (name, data) in evs {
                seq += 1;
                let ev = Event::default()
                    .id(format!("{}-{seq}", chrono::Utc::now().timestamp()))
                    .event(name)
                    .data(data.to_string());
                if tx.send(Ok(ev)).await.is_err() {
                    return;
                }
            }
            if s.hidden {
                break;
            } // 下架：廣播後關閉連線
        }
    });
    // 連線一建立就先送一個 retry 事件：代理（Cloudflare Pages 同源代理等）會緩衝到第一個位元組才轉送，
    // 沒有它，要等 15 秒的第一次心跳瀏覽器才收得到回應標頭。
    let first = futures_util::stream::once(async {
        Ok::<_, std::convert::Infallible>(Event::default().retry(Duration::from_secs(5)))
    });
    let rest =
        futures_util::stream::unfold(
            out,
            |mut rx| async move { rx.recv().await.map(|e| (e, rx)) },
        );
    let stream = futures_util::StreamExt::chain(first, rest);
    let mut res = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("heartbeat"),
        )
        .into_response();
    let h = res.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    h.insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(res)
}
