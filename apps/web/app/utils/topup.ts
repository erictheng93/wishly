// 儲值（/me/topup）的純函式：表單驗證、請求內容、Idempotency-Key 重用、錯誤碼對應、取號資訊格式化。
// shortcut: 發票只支援會員載具（後端電子發票尚未串接，其他載具回 422）；開放後再補載具選擇與逐欄驗證（手機條碼 /[A-Z0-9+\-.]{7}、憑證 2 大寫+14 數字、捐贈碼 3–7 數字、統編 8 數字）。
export const TOPUP_MIN = 100
export const TOPUP_MAX = 50_000
export const WALLET_CAP = 50_000
export const QUICK_AMOUNTS = [500, 1000, 2000, 5000]
export const TOPUP_METHODS = ['card', 'cvs_code', 'cvs_barcode', 'atm'] as const
export type Method = typeof TOPUP_METHODS[number]

export interface TopupForm { amount: string, method: Method }
export const emptyForm = (): TopupForm => ({ amount: '', method: 'card' })

/** 回 i18n key（me.topup.err.*），合法回 null；只收整數（不接受 1e3、小數、空白） */
export function amountError(v: string): string | null {
  if (!/^\d+$/.test(v.trim())) return 'me.topup.err.amountInt'
  const n = Number(v)
  return n >= TOPUP_MIN && n <= TOPUP_MAX ? null : 'me.topup.err.amountRange'
}

type Field = 'amount'
/** 逐欄錯誤：欄位名 → i18n key */
export function formErrors(f: TopupForm): Partial<Record<Field, string>> {
  const e: Partial<Record<Field, string>> = {}
  const a = amountError(f.amount); if (a) e.amount = a
  return e
}

/** 後端 errors[].pointer → 表單欄位 */
export const POINTER_FIELD: Record<string, Field> = { '/amount': 'amount' }

export const buildBody = (f: TopupForm) => ({ amount: Number(f.amount), method: f.method, invoice: { carrier_type: 'member' } })

/** 「同一份表單內容」→ 同一個 Idempotency-Key：送出的 body 不變就重用，改變才換新；成功後 reset */
export function idemKeys(gen: () => string = () => crypto.randomUUID()) {
  let fp = '', key = ''
  return {
    keyFor(body: unknown) { const s = JSON.stringify(body); if (s !== fp || !key) { fp = s; key = gen() } return key },
    reset() { fp = ''; key = '' },
  }
}

/** Retry-After 秒數 → 約幾分鐘（至少 1）；讀不到（跨網域 CORS 未公開該標頭）回 null */
export const retryMinutes = (sec?: number | string | null): number | null => {
  const n = Number(sec)
  return Number.isFinite(n) && n > 0 ? Math.max(1, Math.ceil(n / 60)) : null
}

/** 錯誤碼 → 文案（t / fmt 由呼叫端傳入，方便單元測試）。fields 為後端 errors[] 對應到的逐欄訊息 */
export function topupError(e: { status?: number, code?: string, detail?: string, body?: any, errors?: any[] }, t: (k: string, p?: any) => string, fmt: (n: number) => string = n => String(n)): { msg: string, disabled?: boolean, fields: Partial<Record<Field, string>> } {
  const fields: Partial<Record<Field, string>> = {}
  for (const x of e.errors ?? []) { const f = POINTER_FIELD[x.pointer]; if (f && x.detail) fields[f] = x.detail }
  switch (e.code) {
    case 'TOPUP_DISABLED': return { msg: t('me.topup.err.disabled'), disabled: true, fields }
    case 'WALLET_FROZEN': return { msg: t('me.frozenBanner'), fields }
    case 'WALLET_CAP_EXCEEDED': return { msg: t('me.topup.err.cap', { cap: fmt(WALLET_CAP) }), fields }
    case 'TOPUP_LIMIT_EXCEEDED': return { msg: t('me.topup.err.limit', { limit: fmt(e.body?.limit_amount ?? 0), remaining: fmt(e.body?.remaining_amount ?? 0) }), fields }
    case 'RATE_LIMITED': { const min = retryMinutes(e.body?.retry_after); return { msg: min ? t('me.topup.err.rate', { min }) : t('me.topup.err.rateNoMin'), fields } }
    case 'VALIDATION_FAILED': return { msg: t('me.topup.err.validation'), fields }
    case 'NETWORK': return { msg: t('me.topup.err.network'), fields }
  }
  if ((e.status ?? 0) >= 500) return { msg: t('me.topup.err.server'), fields }
  return { msg: e.detail || t('me.topup.err.generic'), fields }
}

export interface PaymentInfo { type: 'cvs_code' | 'cvs_barcode' | 'atm', expire_at?: string, payment_no?: string, barcode1?: string, barcode2?: string, barcode3?: string, bank_code?: string, account?: string }
/** 取號資訊 → 顯示列（label 的 i18n 子 key、值）；尚未取號回空陣列 */
export function paymentRows(p: PaymentInfo | null | undefined): { key: string, value: string }[] {
  if (!p) return []
  if (p.type === 'cvs_code') return [{ key: 'paymentNo', value: p.payment_no ?? '' }]
  if (p.type === 'cvs_barcode') return [{ key: 'barcode1', value: p.barcode1 ?? '' }, { key: 'barcode2', value: p.barcode2 ?? '' }, { key: 'barcode3', value: p.barcode3 ?? '' }]
  return [{ key: 'bankCode', value: p.bank_code ?? '' }, { key: 'account', value: p.account ?? '' }]
}

/** 還在等綠界取號通知：pending 的超商 / ATM 單且 payment_info 仍為 null */
export const awaitingPaymentInfo = (t: { method: string, status: string, payment_info?: unknown }) =>
  t.status === 'pending' && t.method !== 'card' && !t.payment_info

/** 剩餘時間 → { d, h, m }；已到期回 null */
export function remaining(iso: string | null | undefined, now: number): { d: number, h: number, m: number } | null {
  const ms = iso ? Date.parse(iso) - now : NaN
  if (!Number.isFinite(ms) || ms <= 0) return null
  const m = Math.ceil(ms / 60_000)
  return { d: Math.floor(m / 1440), h: Math.floor((m % 1440) / 60), m: m % 60 }
}
