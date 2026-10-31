// 綠界模擬（只在本機流程模擬使用）：
// 1. `node qa/role-flows/ecpay-sim.mjs` 啟動 QueryTradeInfo 替身（:8090），API 以 ECPAY_QUERY_URL=http://127.0.0.1:8090/query 啟動；
//    替身從 DB 查 topups 金額，一律回「已付款」，並用 stage 金鑰簽 CheckMacValue。
// 2. 付款人「付款完成」以 ecpayPaid(merchantTradeNo) 模擬：送出帶簽章的付款通知到 POST /webhooks/payments/ecpay。
// 金鑰為 config.rs 的綠界測試環境公開值（3002607），非機密。
import { createHash } from 'node:crypto'
import { createServer } from 'node:http'
import { fileURLToPath } from 'node:url'
import { API, sql, q } from './lib.mjs'

const HASH_KEY = process.env.ECPAY_HASH_KEY ?? 'pwFHCqoQZGmho4w6'
const HASH_IV = process.env.ECPAY_HASH_IV ?? 'EkRm7iFT261dpevs'
const MERCHANT = process.env.ECPAY_MERCHANT_ID ?? '3002607'

// 對應 topup.rs 的 url_encode（.NET 風格）
const enc = s => [...Buffer.from(s, 'utf8')].map(b => {
  const c = String.fromCharCode(b)
  if (/[A-Za-z0-9\-_.!*()]/.test(c)) return c
  if (c === ' ') return '+'
  return '%' + b.toString(16).padStart(2, '0')
}).join('')
export function checkMac(params) {
  const body = Object.entries(params).filter(([k]) => k !== 'CheckMacValue')
    .sort(([a], [b]) => a.toLowerCase() < b.toLowerCase() ? -1 : a.toLowerCase() > b.toLowerCase() ? 1 : 0)
    .map(([k, v]) => `${k}=${v}`).join('&')
  return createHash('sha256').update(enc(`HashKey=${HASH_KEY}&${body}&HashIV=${HASH_IV}`).toLowerCase()).digest('hex').toUpperCase()
}
const signed = p => ({ ...p, CheckMacValue: checkMac(p) })
const txnOf = no => 'SIM' + no.slice(-12)

/** 模擬綠界付款成功通知；回傳 { status, text } */
export async function ecpayPaid(merchantTradeNo, { amount, rtnCode = '1' } = {}) {
  const amt = amount ?? sql(`SELECT amount FROM topups WHERE merchant_trade_no = '${q(merchantTradeNo)}'`)
  const form = signed({ MerchantID: MERCHANT, MerchantTradeNo: merchantTradeNo, RtnCode: rtnCode, RtnMsg: rtnCode === '1' ? '交易成功' : '交易失敗', TradeNo: txnOf(merchantTradeNo), TradeAmt: String(amt), PaymentDate: new Date().toISOString().slice(0, 19).replace('T', ' '), PaymentType: 'Credit_CreditCard', SimulatePaid: '0' })
  const r = await fetch(`${API}/api/v1/webhooks/payments/ecpay`, { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded' }, body: new URLSearchParams(form) })
  return { status: r.status, text: await r.text() }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  createServer((req, res) => {
    let raw = ''
    req.on('data', c => raw += c)
    req.on('end', () => {
      const p = Object.fromEntries(new URLSearchParams(raw))
      const no = p.MerchantTradeNo ?? ''
      const amt = sql(`SELECT amount FROM topups WHERE merchant_trade_no = '${q(no)}'`)
      const out = signed({ MerchantID: MERCHANT, MerchantTradeNo: no, TradeNo: txnOf(no), TradeAmt: amt || '0', TradeStatus: amt ? '1' : '0', PaymentType: 'Credit_CreditCard' })
      res.writeHead(200, { 'content-type': 'text/plain' }).end(new URLSearchParams(out).toString())
    })
  }).listen(8090, '127.0.0.1', () => console.log('ECPay QueryTradeInfo 替身 :8090'))
}
