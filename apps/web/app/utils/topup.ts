// 儲值（/me/topup）的純函式：表單驗證、請求內容、Idempotency-Key 重用、錯誤碼對應、取號資訊格式化。
// 驗證規則與後端 apps/api/src/topup.rs 的 parse_invoice 一致（契約 POST /wallet/topups）。
export const TOPUP_MIN = 100
export const TOPUP_MAX = 50_000
export const WALLET_CAP = 50_000
export const QUICK_AMOUNTS = [500, 1000, 2000, 5000]
export const TOPUP_METHODS = ['card', 'cvs_code', 'cvs_barcode', 'atm'] as const
export const TOPUP_CARRIERS = ['member', 'mobile_barcode', 'citizen_cert', 'donate', 'b2b'] as const
export type Method = typeof TOPUP_METHODS[number]
export type Carrier = typeof TOPUP_CARRIERS[number]

export interface TopupForm { amount: string, method: Method, carrier_type: Carrier, carrier_id: string, donate_code: string, buyer_tax_id: string, buyer_name: string }
export const emptyForm = (): TopupForm => ({ amount: '', method: 'card', carrier_type: 'member', carrier_id: '', donate_code: '', buyer_tax_id: '', buyer_name: '' })

/** 回 i18n key（me.topup.err.*），合法回 null；只收整數（不接受 1e3、小數、空白） */
export function amountError(v: string): string | null {
  if (!/^\d+$/.test(v.trim())) return 'me.topup.err.amountInt'
  const n = Number(v)
  return n >= TOPUP_MIN && n <= TOPUP_MAX ? null : 'me.topup.err.amountRange'
}

// 手機條碼：/ + 7 碼（大寫英數與 + - .）；自然人憑證：2 大寫 + 14 數字；捐贈碼 3–7 數字；統編 8 數字
export const isMobileBarcode = (s: string) => /^\/[A-Z0-9+\-.]{7}$/.test(s)
export const isCitizenCert = (s: string) => /^[A-Z]{2}\d{14}$/.test(s)
export const isDonateCode = (s: string) => /^\d{3,7}$/.test(s)
export const isTaxId = (s: string) => /^\d{8}$/.test(s)

type Field = 'amount' | 'carrier_id' | 'donate_code' | 'buyer_tax_id'
/** 逐欄錯誤：欄位名 → i18n key。只檢查目前載具會送出的欄位 */
export function formErrors(f: TopupForm): Partial<Record<Field, string>> {
  const e: Partial<Record<Field, string>> = {}
  const a = amountError(f.amount); if (a) e.amount = a
  if (f.carrier_type === 'mobile_barcode' && !isMobileBarcode(f.carrier_id.trim())) e.carrier_id = 'me.topup.err.mobile'
  if (f.carrier_type === 'citizen_cert' && !isCitizenCert(f.carrier_id.trim())) e.carrier_id = 'me.topup.err.cert'
  if (f.carrier_type === 'donate' && !isDonateCode(f.donate_code.trim())) e.donate_code = 'me.topup.err.donate'
  if (f.carrier_type === 'b2b' && !isTaxId(f.buyer_tax_id.trim())) e.buyer_tax_id = 'me.topup.err.taxId'
  return e
}

/** 後端 errors[].pointer → 表單欄位 */
export const POINTER_FIELD: Record<string, Field> = { '/amount': 'amount', '/invoice/carrier_id': 'carrier_id', '/invoice/donate_code': 'donate_code', '/invoice/buyer_tax_id': 'buyer_tax_id' }

export function buildBody(f: TopupForm) {
  const invoice: Record<string, string> = { carrier_type: f.carrier_type }
  if (f.carrier_type === 'mobile_barcode' || f.carrier_type === 'citizen_cert') invoice.carrier_id = f.carrier_id.trim()
  if (f.carrier_type === 'donate') invoice.donate_code = f.donate_code.trim()
  if (f.carrier_type === 'b2b') { invoice.buyer_tax_id = f.buyer_tax_id.trim(); if (f.buyer_name.trim()) invoice.buyer_name = f.buyer_name.trim() }
  return { amount: Number(f.amount), method: f.method, invoice }
}

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
