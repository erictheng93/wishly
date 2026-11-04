// 虛擬使用者壓測（k6）。模擬真實瀏覽行為：訪客看分享頁 / 探索頁，會員看儀表板、好友清單、偶爾捐助再取消。
// 每個 VU = 一位同時在線的使用者，每次「換頁」之間有 3–8 秒思考時間。SSE 長連線另由 sse-hold.mjs 模擬。
//
//   k6 run -e BASE=http://127.0.0.1:8080 -e USERS=2000 -e PEAK=800 scripts/loadtest/users.js
//
// 需要：API 以 TRUSTED_PROXY=cloudflare 啟動（每個 VU 以 CF-Connecting-IP 帶自己的 IP，避免全部算同一 IP 被限流），
// DB 已執行 seed.sql（USERS 與 seed 的 users 相同）。
import http from 'k6/http'
import { check, sleep, group } from 'k6'

const BASE = (__ENV.BASE || 'http://127.0.0.1:8080') + '/api/v1'
const USERS = +(__ENV.USERS || 2000)
const PEAK = +(__ENV.PEAK || 500)
const HOLD = __ENV.HOLD || '5m'
const RAMP = __ENV.RAMP || '1m'
const MEMBER_RATIO = +(__ENV.MEMBER_RATIO || 0.4)
const THINK_MIN = +(__ENV.THINK_MIN || 3)
const THINK_MAX = +(__ENV.THINK_MAX || 8)

export const options = {
  discardResponseBodies: false,
  scenarios: {
    users: {
      executor: 'ramping-vus',
      startVUs: 0,
      stages: [
        { duration: RAMP, target: PEAK },
        { duration: HOLD, target: PEAK },
        { duration: '10s', target: 0 },
      ],
      gracefulRampDown: '10s',
    },
  },
  thresholds: {
    'http_req_duration{phase:hold}': ['p(95)<500', 'p(99)<1000'],
    'http_req_failed{phase:hold}': ['rate<0.01'],
  },
  summaryTrendStats: ['avg', 'med', 'p(90)', 'p(95)', 'p(99)', 'max'],
}

// 401（訪客的 /me）與 409（品項已被認捐完）是正常業務回應，不算錯誤
http.setResponseCallback(http.expectedStatuses({ min: 200, max: 399 }, 401, 409))

const t0 = Date.now()
const rampMs = parseDur(RAMP)
function parseDur(s) { const m = /^(\d+)(ms|s|m)$/.exec(s); return m ? +m[1] * { ms: 1, s: 1000, m: 60000 }[m[2]] : 60000 }
function phase() { return Date.now() - t0 < rampMs ? 'ramp' : 'hold' }

const uuidv4 = () => 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => { const r = Math.random() * 16 | 0; return (c === 'x' ? r : (r & 3 | 8)).toString(16) })
const rnd = (a, b) => a + Math.floor(Math.random() * (b - a + 1))
const think = () => sleep(THINK_MIN + Math.random() * (THINK_MAX - THINK_MIN))
const slug = (p, n) => p + String(n).padStart(8, '0')

function hdr(extra) {
  const v = __VU
  return Object.assign({
    'CF-Connecting-IP': `10.${(v >> 16) & 255}.${(v >> 8) & 255}.${v & 255}`,
    'Accept-Language': 'zh-TW',
    'Origin': 'http://localhost:3000',
  }, extra || {})
}
function get(path, tag, extra) {
  return http.get(BASE + path, { headers: hdr(extra), tags: { name: tag, phase: phase() } })
}
function send(method, path, body, tag, extra) {
  return http.request(method, BASE + path, body == null ? null : JSON.stringify(body), {
    headers: hdr(Object.assign({ 'Content-Type': 'application/json' }, extra)), tags: { name: tag, phase: phase() },
  })
}

// ---- 訪客：打開別人分享的連結、逛探索頁 ----
const etags = {}
function visitor() {
  const s = slug('pu', rnd(1, USERS))
  group('share page', () => {
    // SSR（Pages Worker）與瀏覽器各抓一次：SSR 無快取，瀏覽器 useFetch 帶 ETag
    const r = get(`/public/wishlists/${s}`, 'GET /public/wishlists/:slug')
    check(r, { 'list 200': (x) => x.status === 200 })
    const et = r.headers['Etag']
    if (et) etags[s] = et
    get(`/public/wishlists/${s}`, 'GET /public/wishlists/:slug (etag)', et ? { 'If-None-Match': et } : {})
    get('/public/status', 'GET /public/status')
    get('/me', 'GET /me (anon)')
    send('POST', `/public/wishlists/${s}/views`, {}, 'POST /views')
  })
  think()
  if (Math.random() < 0.5) {
    group('explore', () => {
      get(`/public/wishlists?page=${rnd(1, 3)}`, 'GET /public/wishlists (explore)')
    })
    think()
  }
}

// ---- 會員：儀表板、自己的清單、好友清單、偶爾捐助 ----
function member() {
  const n = ((__VU - 1) % USERS) + 1
  const auth = { Cookie: `ws_session=lt-${n}` }
  let lists
  group('dashboard', () => {
    get('/me', 'GET /me', auth)
    get('/me/onboarding', 'GET /me/onboarding', auth)
    get('/friends/requests/count', 'GET /friends/requests/count', auth)
    get('/me/notifications?limit=20', 'GET /me/notifications', auth)
    const r = get('/wishlists?limit=20&status=draft,active,closed', 'GET /wishlists', auth)
    check(r, { 'wishlists 200': (x) => x.status === 200 })
    try { lists = r.json('wishlists') } catch (_) { lists = null }
  })
  think()
  if (lists && lists.length) {
    const w = lists[rnd(0, lists.length - 1)]
    group('my list', () => {
      get(`/wishlists/${w.id}`, 'GET /wishlists/:id', auth)
      get(`/wishlists/${w.id}/dashboard`, 'GET /wishlists/:id/dashboard', auth)
    })
    think()
  }
  // 好友的 friends 清單（seed：n 與 n±1 互為好友）
  const f = n % USERS + 1
  let items = null
  group('friend list', () => {
    const r = get(`/restricted/wishlists/${slug(Math.random() < 0.5 ? 'fr' : 'pu', f)}`, 'GET /restricted/wishlists/:slug', auth)
    check(r, { 'friend list 200': (x) => x.status === 200 })
    try { items = r.json('items') } catch (_) { items = null }
    get('/me', 'GET /me', auth)
  })
  think()
  // 約 1/5 的回合會捐助一個還沒滿的品項，之後取消（讓庫存回補、不撞每小時限流）
  if (items && Math.random() < 0.2) {
    const open = items.filter((i) => i.funding_mode === 'quantity' && i.qty_remaining > 0)
    if (open.length) {
      const it = open[rnd(0, open.length - 1)]
      let claimId
      group('claim', () => {
        const r = send('POST', `/items/${it.id}/claims`, { qty: 1 }, 'POST /items/:id/claims', Object.assign({ 'Idempotency-Key': uuidv4() }, auth))
        check(r, { 'claim 201/409': (x) => x.status === 201 || x.status === 409 })
        if (r.status === 201) claimId = r.json('claim.id')
      })
      think()
      if (claimId) {
        send('DELETE', `/claims/${claimId}`, null, 'DELETE /claims/:id', Object.assign({ 'Idempotency-Key': uuidv4() }, auth))
        think()
      }
    }
  }
}

export default function () {
  // VU 固定身分：前 MEMBER_RATIO 比例為會員，其餘為訪客
  if ((__VU % 100) < MEMBER_RATIO * 100) member()
  else visitor()
}
