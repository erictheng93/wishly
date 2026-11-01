// 訪客端契約 mock（docs 04 §5）。node mocks/server.mjs  → http://localhost:8080
// 救災：/s/relief、/e、/e/quake、/e/quake-old（301 到 quake）、/o/helpers。
// slug: demo=正常 / closed=已結束 / gone=410 / boom=500 / sseoff=清單正常但 events 回 500（測輪詢備援）/ 其他=404。恢復權杖 "good" 有效。
// 登入態：帶 ws_session cookie 即視為會員；cookie 值為 staff（ws_session=staff）則為營運人員，可開 /admin/cash-refunds、/admin/wallet-debts、/admin/topups（假資料，id 為 stale 的列一律 409）。
// 現金退款 /me/refund 需先儲值並在模擬綠界付款後才有可退餘額；MOCK_FROZEN=1 node mocks/server.mjs 可模擬錢包凍結。
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
  // 支持品項：指向另一份公開清單，只有累計成果、沒有進度條；「前往支持」連到 /s/demo?via=i5（mock 只認 demo / closed / sseoff）
  { id: 'i5', title: '一起幫助災區的朋友', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: null, priority: 'medium', qty_needed: 1, qty_claimed: 0,
    funding_mode: 'support', support: { ended: false, target: { slug: 'demo', title: '花蓮救災物資', cover_image_url: null, cover_preset: 'gift', owner_name: '慈善協會' }, points: 1200, qty: 18 } },
]
const view = i => { const r = i.qty_needed - i.qty_claimed; return { funding_mode: 'quantity', ...i, qty_remaining: r, is_fully_claimed: r <= 0, progress_percent: i.target_points ? Math.floor(i.pledged_points / i.target_points * 100) : Math.round(i.qty_claimed / i.qty_needed * 100) } }
const mockUser = { name: '登入會員' }, guests = new Map(), claims = new Map(), idem = new Map(), sse = new Set(), topups = [], refunds = []
let n = 0
const walletFrozen = process.env.MOCK_FROZEN === '1' // MOCK_FROZEN=1 node mocks/server.mjs → 錢包凍結（橫幅與 WALLET_FROZEN）
// 後台假資料（重啟即重置）。id 為 'stale' 的列一律回 409 INVALID_STATE_TRANSITION（模擬別人已處理）
const iso = ms => new Date(Date.now() + ms).toISOString()
const usr = (i, name) => ({ id: 'u' + i, display_name: name, email: `${name}@example.com` })
const adminRefunds = [
  { id: 'cr-a', amount: 2000, status: 'requested', user: usr(2, '小明'), lot_allocations: [], reject_reason: null, processed_by: null, created_at: iso(-3 * 36e5), completed_at: null },
  { id: 'cr-b', amount: 800, status: 'processing', user: usr(3, '小華'), lot_allocations: [], reject_reason: null, processed_by: 'staff1', created_at: iso(-30 * 36e5), completed_at: null },
  { id: 'cr-c', amount: 500, status: 'requested', user: usr(4, '阿美'), lot_allocations: [], reject_reason: null, processed_by: null, created_at: iso(-36e5), completed_at: null },
  { id: 'stale', amount: 100, status: 'requested', user: usr(5, '過期單'), lot_allocations: [], reject_reason: null, processed_by: null, created_at: iso(-2 * 36e5), completed_at: null },
  { id: 'cr-d', amount: 1200, status: 'completed', user: usr(6, '已完成'), lot_allocations: [], reject_reason: null, processed_by: 'staff1', created_at: iso(-90 * 36e5), completed_at: iso(-60 * 36e5) },
  { id: 'cr-e', amount: 300, status: 'rejected', user: usr(7, '被拒絕'), lot_allocations: [], reject_reason: '退款帳戶資料不符', processed_by: 'staff1', created_at: iso(-80 * 36e5), completed_at: iso(-70 * 36e5) },
]
const debts = [
  { id: 'd-a', amount: 1500, status: 'open', topup_id: 'tp-x1', user: usr(8, '欠款甲'), note: null, resolved_at: null, created_at: iso(-48 * 36e5) },
  { id: 'd-b', amount: 400, status: 'open', topup_id: 'tp-x2', user: usr(9, '欠款乙'), note: null, resolved_at: null, created_at: iso(-24 * 36e5) },
  { id: 'stale', amount: 50, status: 'open', topup_id: 'tp-x3', user: usr(10, '已被結案'), note: null, resolved_at: null, created_at: iso(-12 * 36e5) },
  { id: 'd-c', amount: 700, status: 'settled', topup_id: 'tp-x4', user: usr(11, '已追回'), note: '已匯款', resolved_at: iso(-6 * 36e5), created_at: iso(-96 * 36e5) },
]
const adminTopups = [
  { id: 'tpd-a', amount: 3000, method: 'card', status: 'disputed', provider: 'ecpay', provider_txn_id: 'T0001', created_at: iso(-72 * 36e5), paid_at: iso(-71 * 36e5), user: usr(12, '爭議甲'), wallet: { id: 'w12', status: 'frozen' } },
  { id: 'tpd-b', amount: 1000, method: 'card', status: 'disputed', provider: 'ecpay', provider_txn_id: 'T0002', created_at: iso(-60 * 36e5), paid_at: iso(-59 * 36e5), user: usr(13, '爭議乙'), wallet: { id: 'w13', status: 'frozen' } },
  { id: 'tpd-c', amount: 2000, method: 'card', status: 'paid', provider: 'ecpay', provider_txn_id: 'T0003', created_at: iso(-30 * 36e5), paid_at: iso(-29 * 36e5), user: usr(14, '已入帳丙'), wallet: { id: 'w14', status: 'active' } },
  { id: 'stale', amount: 500, method: 'card', status: 'paid', provider: 'ecpay', provider_txn_id: 'T0004', created_at: iso(-20 * 36e5), paid_at: iso(-19 * 36e5), user: usr(15, '過期丁'), wallet: { id: 'w15', status: 'active' } },
  { id: 'tpd-d', amount: 800, method: 'card', status: 'charged_back', provider: 'ecpay', provider_txn_id: 'T0005', created_at: iso(-200 * 36e5), paid_at: iso(-199 * 36e5), user: usr(16, '已拒付戊'), wallet: { id: 'w16', status: 'frozen' } },
]

// 救災（org-relief）：事件 quake（2 個據點）、已合併的舊事件 quake-old → quake、救災清單 relief
const quake = { id: 'ev1', slug: 'quake', name: '花蓮強震', hazard_type: 'earthquake', counties: ['花蓮縣'], area_note: null, occurred_on: '2026-10-20', description: '花蓮地區強震，多處需要物資。', merged_into: null }
const reliefOrg = { slug: 'helpers', display_name: '好幫手協會' }
const reliefSites = [
  { slug: 'relief', title: '花蓮物資站', site_address: '花蓮縣花蓮市中山路 1 號', site_status: 'active', org: reliefOrg, needs: { critical: 2, high: 1, medium: 0, low: 0 } },
  { slug: 'relief2', title: '壽豐物資站', site_address: '花蓮縣壽豐鄉中山路 2 號', site_status: 'active', org: reliefOrg, needs: { critical: 0, high: 3, medium: 1, low: 0 } },
]
const reliefItems = [
  { id: 'r1', title: '瓶裝水', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: null, priority: null, qty_needed: 100, qty_claimed: 20, category: 'water', urgency: 'critical' },
  { id: 'r2', title: '毛毯', brand: null, spec: null, image_url: null, product_url: null, unit_price_amount: null, priority: null, qty_needed: 50, qty_claimed: 10, category: 'shelter', urgency: 'high' },
]

const brandList = [
  { slug: 'pigeon', name: 'Pigeon 貝親', description: '日本母嬰用品品牌，奶瓶與清潔系列。', logo_url: null, website_url: 'https://example.com' },
  { slug: 'muji', name: '無印良品', description: '簡約生活選物。', logo_url: null, website_url: 'https://example.com' },
]
const catalog = Array.from({ length: 30 }, (_, i) => ({
  ref: 'mock-' + i, title: `精選好物 ${i + 1}`, spec: i % 2 ? '米白 / M' : null, note: '編輯推薦，送禮不踩雷。', price_amount: [480, 1200, 3600][i % 3], image_url: null, product_url: 'https://example.com/p' + i,
  recipients: [['baby', 'friend', 'partner', 'elder'][i % 4]], occasions: [['birthday', 'full_month', 'housewarming', 'holiday'][i % 4]],
  brand: i % 3 === 2 ? null : { slug: brandList[i % 2].slug, name: brandList[i % 2].name },
}))

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

  // 商品目錄 / 品牌館（公開）：?brand= 的 slug 不存在的品牌回 404；MOCK_EMPTY_CATALOG=1 看空狀態
  if (p === '/public/catalog/products') {
    const q = u.searchParams, page = +(q.get('page') ?? 1)
    const band = x => ({ lt1000: x < 1000, '1000-3000': x >= 1000 && x <= 3000, gt3000: x > 3000 })
    const all = process.env.MOCK_EMPTY_CATALOG ? [] : catalog.filter(x => (!q.get('brand') || x.brand?.slug === q.get('brand')) && (!q.get('recipient') || x.recipients.includes(q.get('recipient'))) && (!q.get('occasion') || x.occasions.includes(q.get('occasion'))) && (!q.get('budget') || band(x.price_amount)[q.get('budget')]))
    return send(res, 200, { products: all.slice((page - 1) * 24, page * 24), next_page: all.length > page * 24 ? page + 1 : null })
  }
  if (p === '/public/brands') return send(res, 200, { brands: process.env.MOCK_EMPTY_CATALOG ? [] : brandList.map(({ website_url, ...b }) => b) })
  if ((m = p.match(/^\/public\/brands\/([^/]+)$/))) {
    const b = brandList.find(x => x.slug === m[1])
    return b ? send(res, 200, { brand: b }) : err(res, 404, 'NOT_FOUND', '找不到')
  }
  if (p === '/public/relief-events') return send(res, 200, { events: [quake], next_page: null })
  if ((m = p.match(/^\/public\/relief-events\/([^/]+)$/))) {
    if (m[1] === 'quake-old') return send(res, 200, { merged_into: 'quake', event: null, sites: [] })
    if (m[1] !== 'quake') return err(res, 404, 'NOT_FOUND', '找不到')
    return send(res, 200, { event: quake, sites: reliefSites, merged_into: null })
  }
  if ((m = p.match(/^\/public\/orgs\/([^/]+)$/))) {
    if (m[1] !== reliefOrg.slug) return err(res, 404, 'NOT_FOUND', '找不到')
    return send(res, 200, { ...reliefOrg, intro: '長期投入災後物資援助。', website: 'https://example.com', public_email: 'hi@example.com', public_phone: null, logo: null, sites: reliefSites.map(({ needs, org, ...x }) => ({ ...x, event: { slug: quake.slug, name: quake.name } })) })
  }
  if (p === '/public/wishlists') {
    const page = +(u.searchParams.get('page') ?? 1), ty = u.searchParams.get('type')
    const all = Array.from({ length: 30 }, (_, i) => ({ slug: i ? `demo${i}` : 'demo', title: `公開清單 ${i + 1}`, type: i % 3 ? 'registry' : 'relief', cover_image_url: null, cover_preset: ['birthday', 'wedding', 'baby', 'housewarming', 'graduation', 'holiday', 'travel', 'gift'][i % 8], event_date: null, owner: { display_name: '小愛', handle: 'xiaoai' }, item_count: 3, completion_pct: (i * 7) % 100 })).filter(w => !ty || w.type === ty)
    return send(res, 200, { wishlists: all.slice((page - 1) * 24, page * 24), next_page: all.length > page * 24 ? page + 1 : null })
  }
  if ((m = p.match(/^\/public\/wishlists\/([^/]+)$/))) {
    if (m[1] === 'boom') return err(res, 500, 'INTERNAL', '內部錯誤')
    if (m[1] === 'gone') return err(res, 410, 'WISHLIST_REMOVED', '已下架')
    if (m[1] === 'relief' || m[1] === 'relief2') {
      const site = reliefSites.find(x => x.slug === m[1]), list = reliefItems.map(view)
      return send(res, 200, { slug: m[1], type: 'relief', status: 'active', title: site.title, description: null, cover_image_url: null, cover_preset: 'gift', event_date: null, id: 'w9', owner: null, surprise_mode: false, claimers_visible: false,
        org: { ...reliefOrg, verified: true }, event: { slug: quake.slug, name: quake.name }, site_address: site.site_address, site_status: 'active',
        completion: { item_count: list.length, fulfilled_count: 0, completion_pct: 20 }, items: list, updated_at: new Date().toISOString() })
    }
    if (!['demo', 'closed', 'sseoff'].includes(m[1])) return err(res, 404, 'NOT_FOUND', '找不到')
    const list = items.map(view), done = list.filter(i => i.is_fully_claimed).length
    return send(res, 200, { slug: m[1], type: 'registry', status: m[1] === 'closed' ? 'closed' : 'active', title: '小愛的待產清單', description: '預產期 12 月，謝謝大家的心意', cover_image_url: null, cover_preset: 'baby', event_date: '2026-12-20', id: 'w1', owner: { id: 'u1', display_name: '小愛' }, surprise_mode: false, claimers_visible: false, completion: { item_count: list.length, fulfilled_count: done, completion_pct: Math.round(items.reduce((a, i) => a + i.qty_claimed, 0) / items.reduce((a, i) => a + i.qty_needed, 0) * 100) }, items: list, updated_at: new Date().toISOString() })
  }
  if (p.match(/^\/public\/wishlists\/[^/]+\/events$/)) {
    if (p.includes('/sseoff/')) return err(res, 500, 'INTERNAL', '內部錯誤')
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' }); res.write('retry: 5000\n: heartbeat\n\n')
    sse.add(res); req.on('close', () => sse.delete(res)); return
  }
  if (p.match(/^\/public\/wishlists\/[^/]+\/views$/)) return send(res, 204)
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
    if (body.qty > rem) return err(res, 409, 'ITEM_FULLY_CLAIMED', `此品項剩餘 ${rem} 件，無法再認領 ${body.qty} 件。`, { remaining: rem })
    const g = mockUser
    if ([...claims.values()].some(c => c.guest === g && c.item_id === it.id && c.status !== 'cancelled')) return err(res, 409, 'CLAIM_ALREADY_EXISTS', '你已經要送這個品項', { claim_id: [...claims.values()].find(c => c.guest === g && c.item_id === it.id).id })
    it.qty_claimed += body.qty
    const c = { id: 'c' + ++n, item_id: it.id, qty: body.qty, status: 'reserved', name: g.name, note: body.note ?? null, at: new Date().toISOString(), guest: g }
    claims.set(c.id, c); broadcast(it)
    const out = { claim: claimOut(c), item: { id: it.id, ...pick(it) } }
    idem.set(key, out); return send(res, 201, out)
  }
  if ((m = p.match(/^\/claims\/([^/]+)$/))) {
    const c = claims.get(m[1]); if (!guest) return err(res, 401, 'UNAUTHORIZED', '請先認領')
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
  const staff = /(^|;\s*)ws_session=staff\b/.test(req.headers.cookie ?? '') // 後台頁：cookie ws_session=staff 視為營運人員
  if (p === '/me' && authed) return send(res, 200, { id: 'u1', display_name: mockUser.name, email: 'mock@example.com', handle: 'mock', is_staff: staff, locale: 'zh-TW' })
  if (p.startsWith('/admin/')) {
    if (!authed) return err(res, 401, 'UNAUTHORIZED', '未登入')
    if (!staff) return err(res, 403, 'STAFF_ONLY', '僅限營運人員')
    const paged = (rows, q) => { const lim = +(q.get('limit') ?? 20), start = q.get('cursor') ? rows.findIndex(r => r.id === q.get('cursor')) + 1 : 0, pg = rows.slice(start, start + lim); return { data: pg, next_cursor: start + lim < rows.length ? pg.at(-1).id : null } }
    const invalid = (pointer, detail) => err(res, 422, 'VALIDATION_FAILED', detail, { errors: [{ pointer, code: 'INVALID', detail }] })
    const stale = () => err(res, 409, 'INVALID_STATE_TRANSITION', '此狀態無法這樣變更')
    if (p === '/admin/cash-refunds' && req.method === 'GET') { const st = u.searchParams.get('status'); return send(res, 200, paged(adminRefunds.filter(r => !st || r.status === st), u.searchParams)) }
    if ((m = p.match(/^\/admin\/cash-refunds\/([^/]+)$/)) && req.method === 'PATCH') {
      const r = adminRefunds.find(x => x.id === m[1]); if (!r) return err(res, 404, 'NOT_FOUND', '找不到')
      if (!['processing', 'completed', 'rejected'].includes(body.status)) return invalid('/status', 'status 必須是 processing | completed | rejected')
      if (body.status === 'rejected' && !String(body.reject_reason ?? '').trim()) return invalid('/reject_reason', 'reject_reason 必填')
      const ok = r.id !== 'stale' && (body.status === 'processing' ? r.status === 'requested' : body.status === 'completed' ? r.status === 'processing' : ['requested', 'processing'].includes(r.status))
      if (!ok) return stale()
      r.status = body.status; if (body.status !== 'processing') r.completed_at = new Date().toISOString(); if (body.status === 'rejected') r.reject_reason = body.reject_reason
      return send(res, 200, { id: r.id, amount: r.amount, status: r.status, reject_reason: r.reject_reason, processed_by: 'staff1', completed_at: r.completed_at })
    }
    if (p === '/admin/wallet-debts' && req.method === 'GET') { const st = u.searchParams.get('status') ?? 'open'; return send(res, 200, paged(debts.filter(d => d.status === st), u.searchParams)) }
    if ((m = p.match(/^\/admin\/wallet-debts\/([^/]+)$/)) && req.method === 'PATCH') {
      const d = debts.find(x => x.id === m[1]); if (!d) return err(res, 404, 'NOT_FOUND', '找不到')
      if (!['settled', 'written_off'].includes(body.status)) return invalid('/status', 'status 必須是 settled | written_off')
      if (!String(body.note ?? '').trim()) return invalid('/note', 'note 必填')
      if (d.status !== 'open' || d.id === 'stale') return stale()
      Object.assign(d, { status: body.status, note: body.note, resolved_at: new Date().toISOString() })
      return send(res, 200, { id: d.id, status: d.status, note: d.note, resolved_at: d.resolved_at })
    }
    if (p === '/admin/topups' && req.method === 'GET') { const st = u.searchParams.get('status'); return send(res, 200, paged(adminTopups.filter(x => !st || x.status === st), u.searchParams)) }
    if ((m = p.match(/^\/admin\/topups\/([^/]+)\/dispute$/)) && req.method === 'POST') {
      const key = req.headers['idempotency-key']; if (!key) return err(res, 400, 'IDEMPOTENCY_KEY_REQUIRED', '缺少 Idempotency-Key')
      if (idem.has(key)) { res.setHeader('idempotency-replayed', 'true'); return send(res, 200, idem.get(key)) }
      const x = adminTopups.find(t => t.id === m[1]); if (!x) return err(res, 404, 'NOT_FOUND', '找不到')
      if (!['open', 'won', 'lost'].includes(body.action)) return invalid('/action', 'action 必須是 open | won | lost')
      const from = body.action === 'open' ? ['paid'] : body.action === 'won' ? ['disputed'] : ['paid', 'disputed']
      if (x.id === 'stale' || !from.includes(x.status)) return stale()
      let out
      if (body.action === 'open') { x.status = 'disputed'; x.wallet.status = 'frozen'; out = { topup_id: x.id, status: x.status, applied: true } }
      else if (body.action === 'won') { x.status = 'paid'; x.wallet.status = 'active'; out = { topup_id: x.id, status: x.status, applied: true } }
      else {
        x.status = 'charged_back'; x.wallet.status = 'frozen'; const recovered = Math.min(x.amount, 600)
        out = { topup_id: x.id, status: x.status, applied: true, recovered, debt: x.amount - recovered }
        if (x.amount > recovered) debts.unshift({ id: 'd' + ++n, amount: x.amount - recovered, status: 'open', topup_id: x.id, user: x.user, note: null, resolved_at: null, created_at: new Date().toISOString() })
      }
      idem.set(key, out); return send(res, 200, out)
    }
  }
  if (p === '/mock-ecpay') { // 模擬綠界付款頁：收到表單 POST 後把該單標為已付款
    const t = topups.find(x => x.merchant_trade_no === body.MerchantTradeNo); if (t) t.paid_flag = true
    res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' }); return res.end(`<h1>模擬綠界</h1><p>${t ? '已付款' : '找不到訂單'}</p><a href="http://localhost:3000/me/topup">返回</a>`)
  }
  if (p.startsWith('/wallet') && !authed) return err(res, 401, 'UNAUTHORIZED', '未登入')
  if (p === '/wallet' && req.method === 'GET') { const bal = walletBalance(); return send(res, 200, { wallet: { balance: bal, held_points: 0, status: walletFrozen ? 'frozen' : 'active', cash_refundable_balance: bal }, data: [], next_cursor: null }) }
  // 現金退款（/me/refund）：可退餘額 = 已入帳儲值 - 未被拒絕的申請（要先到 /me/topup 儲值並付款）。
  // 特殊金額觸發錯誤：555=429、666=403 WALLET_FROZEN、777=409 NOT_CASH_REFUNDABLE（refundable_points 300）、888=409 INSUFFICIENT_POINTS、999=422
  if (p === '/wallet/cash-refunds' && req.method === 'GET') return send(res, 200, { requests: [...refunds].reverse().slice(0, +(u.searchParams.get('limit') ?? 20)).map(refundOut) })
  if (p === '/wallet/cash-refunds' && req.method === 'POST') {
    const key = req.headers['idempotency-key']; if (!key) return err(res, 400, 'IDEMPOTENCY_KEY_REQUIRED', '缺少 Idempotency-Key')
    if (idem.has(key)) { res.setHeader('idempotency-replayed', 'true'); return send(res, 201, idem.get(key)) }
    if (!Number.isInteger(body.amount) || body.amount < 1 || body.amount > 50000 || body.amount === 999) return err(res, 422, 'VALIDATION_FAILED', 'amount 必須介於 1 與 50000', { errors: [{ pointer: '/amount', code: 'RANGE', detail: 'amount 必須介於 1 與 50000' }] })
    if (body.amount === 555) { res.setHeader('retry-after', '1800'); return err(res, 429, 'RATE_LIMITED', '每小時最多 10 次') }
    if (body.amount === 666 || walletFrozen) return err(res, 403, 'WALLET_FROZEN', '錢包已凍結，無法申請現金退款，請聯絡客服')
    if (body.amount === 888) return err(res, 409, 'INSUFFICIENT_POINTS', '點數餘額不足（目前 50 點）', { balance: 50 })
    if (body.amount === 777 || body.amount > walletBalance()) return err(res, 409, 'NOT_CASH_REFUNDABLE', '可退現金的點數只有 300 點（禮物卡與贊助點數不可退現金）', { refundable_points: body.amount === 777 ? 300 : walletBalance(), balance: walletBalance() })
    const r = { id: 'cr' + ++n, amount: body.amount, status: 'requested', created_at: new Date().toISOString(), completed_at: null, reject_reason: null }
    refunds.push(r)
    const out = { request: refundOut(r), wallet: { balance: walletBalance() } }
    idem.set(key, out); return send(res, 201, out)
  }
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
const walletBalance = () => topups.filter(x => x.status === 'paid').reduce((a, x) => a + x.amount, 0) - refunds.filter(r => r.status !== 'rejected').reduce((a, r) => a + r.amount, 0)
const refundOut = r => ({ id: r.id, amount: r.amount, status: r.status, created_at: r.created_at, completed_at: r.completed_at, reject_reason: r.reject_reason })
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
