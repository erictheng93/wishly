import { describe, expect, it } from 'vitest'
import { amountError, awaitingPaymentInfo, buildBody, emptyForm, formErrors, idemKeys, paymentRows, remaining, retryMinutes, topupError } from '../app/utils/topup'
import { t } from './setup'

describe('amountError', () => {
  it.each(['100', '50000', '500', ' 1000 '])('接受 %s', v => expect(amountError(v)).toBeNull())
  it.each(['99', '50001', '0'])('範圍外 %s', v => expect(amountError(v)).toBe('me.topup.err.amountRange'))
  it.each(['', 'abc', '1e3', '100.5', '-100', '1,000'])('非整數 %s', v => expect(amountError(v)).toBe('me.topup.err.amountInt'))
})

describe('formErrors / buildBody', () => {
  const f = (o: object) => ({ ...emptyForm(), amount: '500', ...o })
  it('固定送會員載具', () => {
    expect(formErrors(f({}))).toEqual({})
    expect(formErrors(f({ amount: '5' }))).toEqual({ amount: 'me.topup.err.amountRange' })
    expect(buildBody(f({}))).toEqual({ amount: 500, method: 'card', invoice: { carrier_type: 'member' } })
    expect(buildBody(f({ method: 'atm' })).method).toBe('atm')
  })
})

describe('idemKeys（同一份內容重用，內容改變才換新）', () => {
  const mk = () => { let n = 0; return idemKeys(() => `k${++n}`) }
  it('相同 body 重用同一把 key（重試不重複建單）', () => {
    const k = mk(), b = { amount: 500, method: 'card' }
    expect(k.keyFor(b)).toBe('k1')
    expect(k.keyFor({ ...b })).toBe('k1')
  })
  it('內容改變就換新 key；改回去也是新的（舊 key 已失效）', () => {
    const k = mk()
    expect(k.keyFor({ amount: 500 })).toBe('k1')
    expect(k.keyFor({ amount: 1000 })).toBe('k2')
    expect(k.keyFor({ amount: 500 })).toBe('k3')
  })
  it('成功後 reset，下一次一定是新 key', () => {
    const k = mk(), b = { amount: 500 }
    k.keyFor(b); k.reset()
    expect(k.keyFor(b)).toBe('k2')
  })
})

describe('topupError', () => {
  const fmt = (n: number) => n.toLocaleString('en-US')
  const run = (e: any) => topupError(e, t, fmt)
  it('錢包上限顯示上限', () => expect(run({ code: 'WALLET_CAP_EXCEEDED' }).msg).toContain('50,000'))
  it('每日額度顯示上限與剩餘', () => {
    const m = run({ code: 'TOPUP_LIMIT_EXCEEDED', body: { limit_amount: 3000, remaining_amount: 500 } }).msg
    expect(m).toContain('3,000'); expect(m).toContain('500')
  })
  it('429 顯示約幾分鐘，讀不到 Retry-After 時不亂編', () => {
    expect(run({ code: 'RATE_LIMITED', status: 429, body: { retry_after: 1800 } }).msg).toContain('30 分鐘')
    expect(run({ code: 'RATE_LIMITED', status: 429, body: { retry_after: 61 } }).msg).toContain('2 分鐘')
    expect(run({ code: 'RATE_LIMITED', status: 429, body: {} }).msg).not.toMatch(/分鐘/)
  })
  it('TOPUP_DISABLED 要求隱藏表單', () => {
    const r = run({ code: 'TOPUP_DISABLED', status: 503 })
    expect(r.disabled).toBe(true); expect(r.msg).toBe('儲值功能尚未開放')
  })
  it('422 把 errors[] 的 pointer 對到欄位', () => {
    const r = run({ code: 'VALIDATION_FAILED', status: 422, errors: [{ pointer: '/amount', detail: '金額錯' }, { pointer: '/other', detail: 'x' }] })
    expect(r.fields).toEqual({ amount: '金額錯' })
  })
  it('WALLET_FROZEN 顯示凍結訊息', () => {
    expect(run({ code: 'WALLET_FROZEN', status: 409 }).msg).toContain('凍結')
  })
  it('網路 / 5xx / 其他', () => {
    expect(run({ code: 'NETWORK', status: 0 }).msg).toContain('連線')
    expect(run({ status: 500 }).msg).toContain('暫時無法')
    expect(run({ status: 409, detail: '後端訊息' }).msg).toBe('後端訊息')
    expect(run({}).msg).toContain('失敗')
  })
})

describe('retryMinutes', () => {
  it('進位到分鐘、缺值回 null', () => {
    expect(retryMinutes(1)).toBe(1); expect(retryMinutes(60)).toBe(1); expect(retryMinutes(61)).toBe(2); expect(retryMinutes('1800')).toBe(30)
    expect(retryMinutes(undefined)).toBeNull(); expect(retryMinutes(0)).toBeNull(); expect(retryMinutes('abc')).toBeNull()
  })
})

describe('取號資訊格式化', () => {
  it('null → 取號中', () => {
    expect(paymentRows(null)).toEqual([])
    expect(awaitingPaymentInfo({ method: 'cvs_code', status: 'pending', payment_info: null })).toBe(true)
    expect(awaitingPaymentInfo({ method: 'card', status: 'pending', payment_info: null })).toBe(false)
    expect(awaitingPaymentInfo({ method: 'atm', status: 'paid', payment_info: null })).toBe(false)
    expect(awaitingPaymentInfo({ method: 'atm', status: 'pending', payment_info: { type: 'atm' } })).toBe(false)
  })
  it('各型別的顯示列', () => {
    expect(paymentRows({ type: 'cvs_code', payment_no: 'LLL1' })).toEqual([{ key: 'paymentNo', value: 'LLL1' }])
    expect(paymentRows({ type: 'cvs_barcode', barcode1: 'a', barcode2: 'b', barcode3: 'c' }).map(r => r.value)).toEqual(['a', 'b', 'c'])
    expect(paymentRows({ type: 'atm', bank_code: '012', account: '999' })).toEqual([{ key: 'bankCode', value: '012' }, { key: 'account', value: '999' }])
  })
  it('remaining：天 / 時 / 分、到期回 null', () => {
    const now = Date.parse('2026-10-19T00:00:00Z')
    expect(remaining('2026-10-22T00:00:00Z', now)).toEqual({ d: 3, h: 0, m: 0 })
    expect(remaining('2026-10-19T02:30:00Z', now)).toEqual({ d: 0, h: 2, m: 30 })
    expect(remaining('2026-10-19T00:00:30Z', now)).toEqual({ d: 0, h: 0, m: 1 })
    expect(remaining('2026-10-18T23:59:00Z', now)).toBeNull()
    expect(remaining(null, now)).toBeNull()
  })
})
