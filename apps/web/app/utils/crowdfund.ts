// P2-A 心願眾籌：純函式（文案對應、時間格式、錯誤歸因、表單轉換），可單元測試
const T = (k: string, p?: Record<string, any>) => useNuxtApp().$i18n.t(k, p ?? {})
/** 目前語系對應的 Intl locale（zh-Hant→zh-TW） */
export const intlLocale = (): string => ({ 'zh-Hant': 'zh-TW', en: 'en', vi: 'vi', id: 'id' } as Record<string, string>)[String(useNuxtApp().$i18n.locale.value)] ?? 'zh-TW'
// 文案對照表：值用 getter 延遲到讀取時才翻譯（語系依請求而定，不能在模組載入時求值）；仍是可列舉的一般物件，spread / v-for 可用
const lazyMap = (prefix: string, keys: string[]): Record<string, string> =>
  Object.defineProperties({}, Object.fromEntries(keys.map(k => [k, { enumerable: true, get: () => T(`${prefix}.${k}`) }])))
export const DISPLAY_STATUS = lazyMap('guest.status', ['open', 'funded', 'ordered', 'shipped', 'delivered', 'expired'])
export const displayStatusLabel = (s?: string | null) => (s && DISPLAY_STATUS[s]) || ''

/** 優先用後端的 display_status；沒有時（例如 dashboard 只回 funding_status）用 funding_status + 採購單狀態推導 */
export function deriveDisplayStatus(item: { display_status?: string | null, funding_status?: string | null }, order?: { status?: string } | null): string | null {
  if (item.display_status) return item.display_status
  const f = item.funding_status
  if (f === 'open' || f === 'expired') return f
  if (f === 'fulfilled') return 'delivered'
  if (f === 'funded') return ({ placed: 'ordered', shipped: 'shipped', delivered: 'delivered' } as Record<string, string>)[order?.status ?? ''] ?? 'funded'
  return null
}

export const CONTRIBUTION_STATUS = lazyMap('guest.contribStatus', ['pledged', 'captured', 'released', 'reallocated'])
export const LEDGER_TYPE = lazyMap('guest.ledger', ['grant', 'pledge', 'release', 'refund', 'adjustment'])
export const ORDER_STATUS = lazyMap('guest.order', ['pending', 'placed', 'shipped', 'delivered', 'failed', 'cancelled'])
export const label = (m: Record<string, string>, k?: string | null) => (k && m[k]) || k || ''

const tfs: Record<string, Intl.DateTimeFormat> = {}
const tf = (loc: string) => (tfs[loc] ??= new Intl.DateTimeFormat(loc, { timeZone: 'Asia/Taipei', year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }))
/** 依目前語系、Asia/Taipei，zh-TW 例如 2026/10/08 13:10 */
export const fmtTime = (iso?: string | null) => (iso ? tf(intlLocale()).format(new Date(iso)).replace(/\s/g, ' ') : '') // ICU 可能用不換行空白分隔日期與時間
export const fmtPts = (n?: number | null) => (n == null ? '–' : n.toLocaleString('en-US'))

/** 公開頁 SSE item.updated 合併進卡片。眾籌品項用 pledged/target 重算進度，數量品項維持原算法 */
export function mergeItemEvent(i: any, d: any) {
  const m = { ...i, ...d, id: i.id }
  if (i.funding_mode === 'crowdfund') {
    const t = m.target_points, p = m.pledged_points
    if (t > 0 && p != null) { m.remaining_points = Math.max(0, t - p); m.progress_percent = Math.min(100, Math.floor(p / t * 100)) }
  } else m.progress_percent = Math.min(100, Math.round(m.qty_claimed / m.qty_needed * 100))
  return m
}

/** 贊助失敗 → 文案；points 表示要把輸入框改成這個值 */
export function contributionErr(code: string, body: any = {}, fallback = T('guest.contribErr.fallback')): { msg: string, points?: number, wallet?: boolean } {
  switch (code) {
    case 'INSUFFICIENT_POINTS': return { msg: T('guest.contribErr.insufficient', { n: fmtPts(body.balance ?? 0) }), wallet: true }
    case 'CROWDFUND_TARGET_EXCEEDED': {
      const r = Number(body.remaining_points ?? 0)
      return r > 0 ? { msg: T('guest.contribErr.almost', { n: fmtPts(r) }), points: r } : { msg: T('guest.contribErr.funded') }
    }
    case 'ITEM_FUNDED': return { msg: T('guest.contribErr.funded') }
    case 'FUNDING_EXPIRED': return { msg: T('guest.contribErr.expired') }
    case 'WALLET_FROZEN': return { msg: T('guest.contribErr.frozen') }
    case 'WISHLIST_CLOSED': return { msg: T('guest.contribErr.closed') }
    case 'WISHLIST_REMOVED': return { msg: T('guest.contribErr.removed') }
    case 'FUNDING_MODE_MISMATCH': return { msg: T('guest.contribErr.mismatch') }
    case 'IDEMPOTENCY_CONFLICT': return { msg: T('guest.dupRequest') }
    case 'RATE_LIMITED': return { msg: T('guest.contribErr.rate') }
    case 'NETWORK': return { msg: T('guest.contribErr.network') }
    default: return { msg: fallback }
  }
}
/** 快選金額：只列小於剩餘的幾檔，「剩餘全額」由畫面另外加 */
export const quickAmounts = (remaining: number) => [100, 500, 1000].filter(n => n < remaining)

// datetime-local 以台北時間輸入 / 顯示（與後端「活動日 23:59 台北時間」一致）
export const toTaipeiInput = (iso?: string | null) => (iso ? new Date(Date.parse(iso) + 8 * 36e5).toISOString().slice(0, 16) : '')
export const fromTaipeiInput = (v?: string | null) => (v ? `${v}:00+08:00` : null)

// --- 後台採購單 ---
export const ORDER_NEXT: Record<string, string[]> = { pending: ['placed', 'failed', 'cancelled'], placed: ['shipped', 'failed', 'cancelled'], shipped: ['delivered'] }
export const orderActions = (status: string) => ORDER_NEXT[status] ?? []
export interface OrderForm { merchant_order_id: string, amount: string | number, tracking_no: string, failure_reason: string }
/** 依目標狀態組 PATCH body 並做前端驗證（後端仍會再驗） */
export function buildOrderPatch(to: string, f: OrderForm, target: number | null): { body?: any, error?: string } {
  if (to === 'placed') {
    const amount = Number(f.amount)
    if (!f.merchant_order_id.trim()) return { error: T('guest.orderErr.orderNo') }
    if (f.amount === '' || !Number.isInteger(amount) || amount <= 0) return { error: T('guest.orderErr.amount') }
    if (target != null && amount > target) return { error: T('guest.orderErr.over', { n: fmtPts(target) }) }
    return { body: { status: to, merchant_order_id: f.merchant_order_id.trim(), amount } }
  }
  if (to === 'shipped') return { body: { status: to, ...(f.tracking_no.trim() ? { tracking_no: f.tracking_no.trim() } : {}) } }
  if (to === 'failed' || to === 'cancelled') {
    if (!f.failure_reason.trim()) return { error: T('guest.reasonRequired') }
    return { body: { status: to, failure_reason: f.failure_reason.trim() } }
  }
  return { body: { status: to } }
}

// --- 後台錢包 ---
export function grantError(points: string | number, reason: string): string {
  const n = Number(points)
  if (points === '' || !Number.isInteger(n) || n === 0) return T('guest.grantErr.points')
  if (Math.abs(n) > 1_000_000) return T('guest.grantErr.max')
  if (!reason.trim()) return T('guest.reasonRequired')
  return ''
}
/** /admin/wallets 列：同時容忍巢狀（user + wallet）與扁平兩種 shape */
export function normWalletRow(r: any) {
  return {
    user_id: r.user?.id ?? r.user_id ?? null, display_name: r.user?.display_name ?? r.display_name ?? '', email: r.user?.email ?? r.email ?? '',
    wallet_id: r.wallet?.id ?? r.wallet_id ?? null, balance: r.wallet?.balance ?? r.balance ?? 0, status: r.wallet?.status ?? r.status ?? 'active',
  }
}
