// 現金退款（/me/refund）的純函式：金額驗證、請求內容、錯誤碼對應、狀態顯示。
// 契約：POST /wallet/cash-refunds（amount 1–50000，Idempotency-Key 必填）；只有儲值購買的點數可退現金（D55）。
import { retryMinutes } from './topup'

export const REFUND_MIN = 1
export const REFUND_MAX = 50_000
export const REFUND_STATUSES = ['requested', 'processing', 'completed', 'rejected'] as const
export type RefundStatus = typeof REFUND_STATUSES[number]

/** 回 i18n key（me.refund.err.*），合法回 null。refundable＝目前可退現金餘額；只收整數（不接受 1e3、小數、空白） */
export function refundAmountError(v: string, refundable: number): string | null {
  if (!/^\d+$/.test(v.trim())) return 'me.refund.err.amountInt'
  const n = Number(v)
  if (n < REFUND_MIN || n > REFUND_MAX) return 'me.refund.err.amountRange'
  return n > refundable ? 'me.refund.err.amountOver' : null
}

export const buildRefundBody = (amount: string) => ({ amount: Number(amount) })

/** 沒有 status 欄位（舊後端）視為 active */
export const isFrozen = (w?: { status?: string } | null) => w?.status === 'frozen'

/** 還在審核（requested / processing）的申請數 */
export const pendingRefunds = (rows: { status: string }[]) => rows.filter(r => r.status === 'requested' || r.status === 'processing').length

/** 狀態 → i18n key；未知狀態回 null（呼叫端退回顯示原字串，不崩） */
export const refundStatusKey = (s: string) => (REFUND_STATUSES as readonly string[]).includes(s) ? `me.refund.status.${s}` : null

/** 錯誤碼 → 文案（t / fmt 由呼叫端傳入，方便單元測試）。frozen 為 true 時呼叫端要隱藏表單；reloadWallet 時要重讀餘額 */
export function refundError(e: { status?: number, code?: string, detail?: string, body?: any }, t: (k: string, p?: any) => string, fmt: (n: number) => string = n => String(n)): { msg: string, frozen?: boolean, reloadWallet?: boolean } {
  switch (e.code) {
    case 'WALLET_FROZEN': return { msg: t('me.refund.err.frozen'), frozen: true }
    case 'NOT_CASH_REFUNDABLE': return { msg: t('me.refund.err.notRefundable', { pts: fmt(Number(e.body?.refundable_points) || 0) }), reloadWallet: true }
    case 'INSUFFICIENT_POINTS': return { msg: t('me.refund.err.insufficient', { bal: fmt(Number(e.body?.balance) || 0) }), reloadWallet: true }
    case 'RATE_LIMITED': { const min = retryMinutes(e.body?.retry_after); return { msg: min ? t('me.refund.err.rate', { min }) : t('me.refund.err.rateNoMin') } }
    case 'VALIDATION_FAILED': return { msg: t('me.refund.err.validation') }
    case 'NETWORK': return { msg: t('me.refund.err.network') }
  }
  if ((e.status ?? 0) >= 500) return { msg: t('me.refund.err.server') }
  return { msg: e.detail || t('me.refund.err.generic') }
}
