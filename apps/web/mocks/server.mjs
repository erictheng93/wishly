// 訪客端契約 mock（docs 04 §5）。node mocks/server.mjs  → http://localhost:8080
// slug: demo=正常 / closed=已結束 / gone=410 / boom=500 / sseoff=清單正常但 events 回 500（測輪詢備援）/ 其他=404。恢復權杖 "good" 有效。
import http from 'node:http'
import { randomBytes } from 'node:crypto'

const items = [
  { id: 'i1', title: '玻璃奶瓶 240ml', brand: 'Pigeon', spec: '寬口徑 / 3 入裝', image_url: null, product_url: 'https://example.com', unit_price_amount: 450, priority: 'high', qty_needed: 10, qty_claimed: 6 },
  { id: 'i2', title: 'NB 尿布 2 包', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: 300, priority: 'medium', qty_needed: 2, qty_claimed: 1 },
  { id: 'i3', title: '奶粉 1 號 800g', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: 800, priority: 'low', qty_needed: 2, qty_claimed: 2 },
  // 心願眾籌品項（P2-A）：沒有後端也能預覽卡片；/me 回 401，所以點「心願眾籌」會導到登入頁
  { id: 'i4', title: 'Combi 嬰兒推車', brand: 'Combi', spec: null, image_url: null, product_url: null, unit_price_amount: null, priority: 'high', qty_needed: 1, qty_claimed: 0,
    funding_mode: 'crowdfund', target_points: 9800, pledged_points: 6300, remaining_points: 3500, funding_status: 'open', display_status: 'open', funding_deadline: '2026-12-19T15:59:00Z',
    contributors: [{ display_name: '阿明', points: 3000 }, { display_name: '匿名朋友', points: 1300 }, { display_name: '小華', points: 2000 }] },
]
const view = i => { const r = i.qty_needed - i.qty_claimed; return { funding_mode: 'quantity', ...i, qty_remaining: r, is_fully_claimed: r <= 0, progress_percent: i.target_points ? Math.floor(i.pledged_points / i.target_points * 100) : Math.round(i.qty_claimed / i.qty_needed * 100) } }
const mockUser = { name: '登入會員' }, guests = new Map(), claims = new Map(), idem = new Map(), sse = new Set(), topups = []
let n = 0

const send = (res, st, body, type = 'application/json') => { res.writeHead(st, { 'content-type': type }); res.end(body === undefined ? '' : JSON.stringify(body)) }
const err = (res, st, code, detail, extra = {}) => send(res, st, { type: 'about:blank', title: code, status: st, code, detail, ...extra }, 'application/problem+json')
const claimOut = c => ({ id: c.id, item_id: c.item_id, qty: c.qty, status: c.status, claimer_name: c.name, note: c.note, expires_at: null, created_at: c.at, updated_at: c.at, purchased_at: null, delivered_at: null, cancelled_at: null })
const broadcast = i => { const v = view(i); for (const r of sse) r.write(`event: item.updated\ndata: ${JSON.stringify({ item_id: i.id, qty_needed: v.qty_needed, qty_claimed: v.qty_claimed, qty_remaining: v.qty_remaining, is_fully_claimed: v.is_fully_claimed })}\n\n`) }

http.createServer(async (req, res) => {
  const origin = req.headers.origin
  if (origin) Object.entries({ 'access-control-allow-origin': origin, 'access-control-allow-credentials': 'true', 'access-control-allow-headers': 'content-type,idempotency-key,x-guest-token', 'access-control-allow-methods': 'GET,POST,PATCH,DELETE,OPTIONS', vary: 'origin' }).forEach(([k, v]) => res.setHeader(k, v))
  if (req.method === 'OPTIONS') return send(res, 204)
  const u = new URL(req.url, 'http://x'), p = u.pathname.replace('/api/v1', '')
  let body = {}
  if (req.method !== 'GET') { let s = ''; for await (const c of req) s += c; try { body = JSON.parse(s || '{}') } catch { body = Object.fromEntries(new URLSearchParams(s)) } }
  const tok = req.headers['x-guest-token'], guest = tok && guests.get(tok)
  let m
  console.log(req.method, p)

  if (p === '/public/wishlists') {
    const page = +(u.searchParams.get('page') ?? 1), ty = u.searchParams.get('type')
    const all = Array.from({ length: 30 }, (_, i) => ({ slug: i ? `demo${i}` : 'demo', title: `公開清單 ${i + 1}`, type: i % 3 ? 'registry' : 'relief', cover_image_url: null, event_date: null, owner: { display_name: '小愛', handle: 'xiaoai' }, item_count: 3, completion_pct: (i * 7) % 100 })).filter(w => !ty || w.type === ty)
    return send(res, 200, { wishlists: all.slice((page - 1) * 24, page * 24), next_page: all.length > page * 24 ? page + 1 : null })
  }
  if ((m = p.match(/^\/public\/wishlists\/([^/]+)$/))) {
    if (m[1] === 'boom') return err(res, 500, 'INTERNAL', '內部錯誤')
    if (m[1] === 'gone') return err(res, 410, 'WISHLIST_REMOVED', '已下架')
    if (!['demo', 'closed', 'sseoff'].includes(m[1])) return err(res, 404, 'NOT_FOUND', '找不到')
    const list = items.map(view), done = list.filter(i => i.is_fully_claimed).length
    return send(res, 200, { slug: m[1], type: 'registry', status: m[1] === 'closed' ? 'closed' : 'active', title: '小愛的待產清單', description: '預產期 12 月，謝謝大家的心意', cover_image_url: null, event_date: '2026-12-20', id: 'w1', owner: { id: 'u1', display_name: '小愛' }, surprise_mode: false, claimers_visible: false, completion: { item_count: list.length, fulfilled_count: done, completion_pct: Math.round(items.reduce((a, i) => a + i.qty_claimed, 0) / items.reduce((a, i) => a + i.qty_needed, 0) * 100) }, items: list, updated_at: new Date().toISOString() })
  }
  if (p.match(/^\/public\/wishlists\/[^/]+\/events$/)) {
    if (p.includes('/sseoff/')) return err(res, 500, 'INTERNAL', '內部錯誤')
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' }); res.write('retry: 5000\n: heartbeat\n\n')
    sse.add(res); req.on('close', () => sse.delete(res)); return
  }
  if (p.match(/^\/public\/wishlists\/[^/]+\/reports$/)) {
    if (!body.turnstile_token) return err(res, 422, 'VALIDATION_FAILED', '驗證失敗', { errors: [{ pointer: '/turnstile_token', code: 'INVALID', detail: '驗證失敗' }] })
    return send(res, 201, { id: 'r' + ++n, status: 'open' })
  }
  if ((m = p.match(/^\/items\/([^/]+)\/claims$/)) && req.method === 'POST') {
    if (!/(^|;\s*)ws_session=/.test(req.headers.cookie ?? '') && !req.headers.authorization) return err(res, 401, 'UNAUTHORIZED', '請先登入') // 認捐需登入會員（本 mock 沒有登入流程，帶 ws_session cookie 即視為已登入）
    const key = req.headers['idempotency-key']; if (!key) return err(res, 400, 'IDEMPOTENCY_KEY_REQUIRED', '缺少 Idempotency-Key')
    if (idem.has(key)) return send(res, 201, idem.get(key))
    const it = items.find(i => i.id === m[1]); if (!it) return err(res, 404, 'NOT_FOUND', '找不到品項')
    if (!(body.qty >= 1 && body.qty <= 99)) return err(res, 422, 'VALIDATION_FAILED', 'qty 不正確', { errors: [{ pointer: '/qty', code: 'RANGE', detail: 'qty 必須介於 1 與 99' }] })
    const rem = it.qty_needed - it.qty_claimed
    if (body.qty > rem) return err(res, 409, 'ITEM_FULLY_CLAIMED', `此品項剩餘 ${rem} 件，無法再捐助 ${body.qty} 件。`, { remaining: rem })
    const g = mockUser
    if ([...claims.values()].some(c => c.guest === g && c.item_id === it.id && c.status !== 'cancelled')) return err(res, 409, 'CLAIM_ALREADY_EXISTS', '已捐助過', { claim_id: [...claims.values()].find(c => c.guest === g && c.item_id === it.id).id })
    it.qty_claimed += body.qty
    const c = { id: 'c' + ++n, item_id: it.id, qty: body.qty, status: 'reserved', name: g.name, note: body.note ?? null, at: new Date().toISOString(), guest: g }
    claims.set(c.id, c); broadcast(it)
    const out = { claim: claimOut(c), item: { id: it.id, ...pick(it) } }
    idem.set(key, out); return send(res, 201, out)
  }
  if ((m = p.match(/^\/claims\/([^/]+)$/))) {
    const c = claims.get(m[1]); if (!guest) return err(res, 401, 'UNAUTHORIZED', '請先捐助')
    if (!c || c.guest !== guest) return err(res, 404, 'NOT_FOUND', '找不到')
    const it = items.find(i => i.id === c.item_id)
    if (req.method === 'DELETE') { if (c.status !== 'cancelled') { it.qty_claimed -= c.qty; c.status = 'cancelled'; broadcast(it) } return send(res, 204) }
    if (body.qty && body.qty !== c.qty) { const d = body.qty - c.qty; if (d > it.qty_needed - it.qty_claimed) return err(res, 409, 'ITEM_FULLY_CLAIMED', '數量不足', { remaining: it.qty_needed - it.qty_claimed }); it.qty_claimed += d; c.qty = body.qty }
    if ('note' in body) c.note = body.note
    if (body.status === 'cancelled') { it.qty_claimed -= c.qty; c.status = 'cancelled' } else if (body.status) c.status = body.status
    broadcast(it); return send(res, 200, { claim: claimOut(c), item: { id: it.id, ...pick(it) } })
  }
  if (p === '/guest/me') {
    if (!guest) return err(res, 401, 'UNAUTHORIZED', '無有效 token')
    if (req.method === 'GET') return send(res, 200, { guest: { display_name: guest.name, contact: guest.contact ?? null }, claims: [...claims.values()].filter(c => c.guest === guest).reverse().map(c => { const it = items.find(i => i.id === c.item_id); return { claim: claimOut(c), item: { id: it.id, title: it.title, image_url: null }, wishlist: { slug: 'demo', title: '小愛的待產清單', event_date: '2026-12-20', status: 'active' } } }), next_cursor: null })
    if (req.method === 'PATCH') { guest.name = body.display_name ?? guest.name; return send(res, 200, { guest: { display_name: guest.name, contact: guest.contact ?? null } }) }
    if (req.method === 'DELETE') { guests.delete(tok); return send(res, 204) }
  }
  if (p === '/guest/recover') {
    if (body.token !== 'good') return err(res, 404, 'NOT_FOUND', '無效')
    const t = randomBytes(32).toString('base64url'); guests.set(t, { name: '阿明(恢復)', token: t }); return send(res, 200, { guest: { display_name: '阿明(恢復)' }, guest_token: t })
  }
  // 儲值（/me/topup）：帶 ws_session cookie 或 Authorization 視為已登入。特殊金額觸發錯誤：666=503、777=每日額度、888=錢包上限、999=429
  const authed = /(^|;\s*)ws_session=/.test(req.headers.cookie ?? '') || !!req.headers.authorization
  if (p === '/me' && authed) return send(res, 200, { id: 'u1', display_name: mockUser.name, email: 'mock@example.com', handle: 'mock', is_staff: false, locale: 'zh-TW' })
  if (p === '/mock-ecpay') { // 模擬綠界付款頁：收到表單 POST 後把該單標為已付款
    const t = topups.find(x => x.merchant_trade_no === body.MerchantTradeNo); if (t) t.paid_flag = true
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }); return res.end(`<h1>模擬綠界</h1><p>${t ? '已付款' : '找不到訂單'}</p><a href="http://localhost:3000/me/topup">返回</a>`)
  }
  if (p.startsWith('/wallet') && !authed) return err(res, 401, 'UNAUTHORIZED', '未登入')
  if (p === '/wallet' && req.method === 'GET') return send(res, 200, { wallet: { balance: topups.filter(x => x.status === 'paid').reduce((a, x) => a + x.amount, 0), held_points: 0, status: 'active', cash_refundable_balance: 0 }, data: [], next_cursor: null })
  if (p === '/wallet/contributions') return send(res, 200, { data: [], next_cursor: null })
  if (p === '/wallet/topups' && req.method === 'GET') return send(res, 200, { topups: [...topups].reverse().slice(0, +(u.searchParams.get('limit') ?? 20)).map(topupOut) })
  if ((m = p.match(/^\/wallet\/topups\/([^/]+)$/)) && req.method === 'GET') { const t = topups.find(x => x.id === m[1]); return t ? send(res, 200, { topup: topupOut(t) }) : err(res, 404, 'NOT_FOUND', '找不到') }
  if (p === '/wallet/topups' && req.method === 'POST') {
    const key = req.headers['idempotency-key']; if (!key) return err(res, 400, 'IDEMPOTENCY_KEY_REQUIRED', '缺少 Idempotency-Key')
    if (idem.has(key)) return send(res, 201, idem.get(key))
    if (body.amount === 666) return err(res, 503, 'TOPUP_DISABLED', '儲值功能尚未開放')
    if (body.amount === 777) return err(res, 409, 'TOPUP_LIMIT_EXCEEDED', '超過每日上限', { limit_amount: 3000, remaining_amount: 500 })
    if (body.amount === 888) return err(res, 409, 'WALLET_CAP_EXCEEDED', '超過錢包上限')
    if (body.amount === 999) { res.setHeader('retry-after', '1800'); return err(res, 429, 'RATE_LIMITED', '每小時最多建立 10 筆儲值單') }
    if (!(body.amount >= 100 && body.amount <= 50000) || !['card', 'cvs_code', 'cvs_barcode', 'atm'].includes(body.method)) return err(res, 422, 'VALIDATION_FAILED', '參數不正確', { errors: [{ pointer: '/amount', code: 'RANGE', detail: '金額須介於 100 至 50,000' }] })
    const t = { id: 'tp' + ++n, amount: body.amount, method: body.method, status: 'pending', created_at: new Date().toISOString(), paid_at: null, merchant_trade_no: 'T' + Date.now(), ready_at: Date.now() + 6000 }
    t.expires_at = new Date(Date.now() + (t.method === 'card' ? 30 * 60e3 : 3 * 864e5)).toISOString(); topups.push(t)
    const out = { topup: { id: t.id, amount: t.amount, method: t.method, provider: 'ecpay', status: 'pending', paid_at: null, created_at: t.created_at }, payment: { type: 'form_post', url: 'http://localhost:8080/mock-ecpay', fields: { MerchantID: '3002607', MerchantTradeNo: t.merchant_trade_no, TotalAmount: String(t.amount), CheckMacValue: 'MOCK' } } }
    idem.set(key, out); return send(res, 201, out)
  }
  if (p === '/me') return err(res, 401, 'UNAUTHORIZED', '未登入')
  err(res, 404, 'NOT_FOUND', 'no route ' + p)
}).listen(8080, () => console.log('mock on :8080'))
// 儲值單：超商 / ATM 的取號資訊建單後約 6 秒才有（模擬綠界取號通知延遲）；/mock-ecpay 收到表單 POST 即標為已付款
function topupOut(t) {
  if (t.status === 'pending' && t.paid_flag) { t.status = 'paid'; t.paid_at = new Date().toISOString() }
  const exp = t.expires_at, ready = Date.now() >= t.ready_at
  const info = t.method === 'card' || !ready ? null
    : t.method === 'cvs_code' ? { type: 'cvs_code', payment_no: 'LLL24110512345', expire_at: exp }
    : t.method === 'cvs_barcode' ? { type: 'cvs_barcode', barcode1: '1311053TW', barcode2: '3002607000012345', barcode3: '110512000000500', expire_at: exp }
    : { type: 'atm', bank_code: '012', account: '9103522175887271', expire_at: exp }
  return { id: t.id, amount: t.amount, method: t.method, status: t.status, created_at: t.created_at, paid_at: t.paid_at, expires_at: exp, payment_info: info }
}
const pick = i => ({ qty_needed: i.qty_needed, qty_claimed: i.qty_claimed, qty_remaining: i.qty_needed - i.qty_claimed })
