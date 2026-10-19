import { describe, expect, it } from 'vitest'
import { amountError, awaitingPaymentInfo, buildBody, emptyForm, formErrors, idemKeys, isCitizenCert, isDonateCode, isMobileBarcode, isTaxId, paymentRows, remaining, retryMinutes, topupError } from '../app/utils/topup'
import { t } from './setup'

describe('amountError', () => {
  it.each(['100', '50000', '500', ' 1000 '])('接受 %s', v => expect(amountError(v)).toBeNull())
  it.each(['99', '50001', '0'])('範圍外 %s', v => expect(amountError(v)).toBe('me.topup.err.amountRange'))
  it.each(['', 'abc', '1e3', '100.5', '-100', '1,000'])('非整數 %s', v => expect(amountError(v)).toBe('me.topup.err.amountInt'))
})

describe('載具格式（與後端 parse_invoice 一致）', () => {
  it('手機條碼：/ + 7 碼大寫英數與 + - .', () => {
    expect(isMobileBarcode('/ABC1234')).toBe(true)
    expect(isMobileBarcode('/AB+-.12')).toBe(true)
    for (const bad of ['ABC12345', '/abc1234', '/ABC123', '/ABC12345', '/AB C123', ''])
      expect(isMobileBarcode(bad), bad).toBe(false)
  })
  it('自然人憑證：2 大寫 + 14 數字', () => {
    expect(isCitizenCert('AB12345678901234')).toBe(true)
    for (const bad of ['A112345678901234', 'ab12345678901234', 'AB1234567890123', 'AB123456789012345', 'ABC2345678901234'])
      expect(isCitizenCert(bad), bad).toBe(false)
  })
  it('捐贈碼 3–7 碼數字', () => {
    for (const ok of ['123', '1234567']) expect(isDonateCode(ok)).toBe(true)
    for (const bad of ['12', '12345678', '12a', '']) expect(isDonateCode(bad), bad).toBe(false)
  })
  it('統編 8 碼數字', () => {
    expect(isTaxId('12345678')).toBe(true)
    for (const bad of ['1234567', '123456789', '1234567a', '']) expect(isTaxId(bad), bad).toBe(false)
  })
})

describe('formErrors / buildBody', () => {
  const f = (o: object) => ({ ...emptyForm(), amount: '500', ...o })
  it('預設會員載具、只送 carrier_type', () => {
    expect(formErrors(f({}))).toEqual({})
    expect(buildBody(f({}))).toEqual({ amount: 500, method: 'card', invoice: { carrier_type: 'member' } })
  })
  it('逐欄顯示錯誤，只檢查目前載具', () => {
    expect(formErrors(f({ amount: '5', carrier_type: 'mobile_barcode', carrier_id: 'x' }))).toEqual({ amount: 'me.topup.err.amountRange', carrier_id: 'me.topup.err.mobile' })
    expect(formErrors(f({ carrier_type: 'donate', carrier_id: 'junk', donate_code: '12' }))).toEqual({ donate_code: 'me.topup.err.donate' })
    expect(formErrors(f({ carrier_type: 'b2b', buyer_tax_id: '1' }))).toEqual({ buyer_tax_id: 'me.topup.err.taxId' })
    expect(formErrors(f({ carrier_type: 'citizen_cert', carrier_id: 'AB1' }))).toEqual({ carrier_id: 'me.topup.err.cert' })
  })
  it('只送該載具的欄位（切換載具後殘留的欄位不外送）', () => {
    const b = buildBody(f({ carrier_type: 'b2b', buyer_tax_id: ' 12345678 ', buyer_name: ' 某公司 ', carrier_id: '/ABC1234', donate_code: '123' }))
    expect(b.invoice).toEqual({ carrier_type: 'b2b', buyer_tax_id: '12345678', buyer_name: '某公司' })
    expect(buildBody(f({ carrier_type: 'b2b', buyer_tax_id: '12345678' })).invoice).not.toHaveProperty('buyer_name')
    expect(buildBody(f({ carrier_type: 'donate', donate_code: '168' })).invoice).toEqual({ carrier_type: 'donate', donate_code: '168' })
    expect(buildBody(f({ carrier_type: 'mobile_barcode', carrier_id: '/ABC1234', method: 'atm' }))).toEqual({ amount: 500, method: 'atm', invoice: { carrier_type: 'mobile_barcode', carrier_id: '/ABC1234' } })
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
    const r = run({ code: 'VALIDATION_FAILED', status: 422, errors: [{ pointer: '/invoice/carrier_id', detail: '手機條碼格式錯' }, { pointer: '/other', detail: 'x' }] })
    expect(r.fields).toEqual({ carrier_id: '手機條碼格式錯' })
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
