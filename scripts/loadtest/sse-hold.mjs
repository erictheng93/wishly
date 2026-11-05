// 模擬在線使用者開著的 SSE 長連線（分享頁即時更新）。與 users.js 同時跑：
//   node scripts/loadtest/sse-hold.mjs http://127.0.0.1:8080 <訪客連線數> <users> [爬升秒數] [會員連線數]
// 訪客：分享頁的 /public/wishlists/{slug}/events；會員：站內通知 /me/notifications/stream（ws_session=lt-<n>）。
// 每條連線用不同的 CF-Connecting-IP；被伺服器關閉就重連。每 10 秒印出：開著的連線、累計失敗（非 200）。
import http from 'node:http'

const [base = 'http://127.0.0.1:8080', nStr = '500', usersStr = '2000', rampStr = '60', mStr = '0'] = process.argv.slice(2)
const N = +nStr, USERS = +usersStr, RAMP = +rampStr * 1000, M = +mStr
const u = new URL(base)
const agent = new http.Agent({ keepAlive: false, maxSockets: Infinity })
let open = 0, fail = 0, opened = 0
const codes = {}

function connect(i) {
  // i < N：訪客分享頁；i >= N：會員站內通知
  const member = i >= N
  const path = member ? '/api/v1/me/notifications/stream'
    : `/api/v1/public/wishlists/${'pu' + String(1 + (i * 7919) % USERS).padStart(8, '0')}/events`
  const headers = { 'CF-Connecting-IP': `10.${member ? 201 : 200}.${(i >> 8) & 255}.${i & 255}`, Accept: 'text/event-stream' }
  if (member) headers.Cookie = `ws_session=lt-${1 + (i - N) % USERS}`
  const req = http.get({ host: u.hostname, port: u.port, agent, path, headers }, (res) => {
    if (res.statusCode !== 200) {
      fail++; codes[res.statusCode] = (codes[res.statusCode] || 0) + 1
      res.resume(); res.on('end', () => setTimeout(() => connect(i), 5000)); return
    }
    open++; opened++
    res.on('data', () => {})
    res.on('close', () => { open--; setTimeout(() => connect(i), 1000) })
  })
  req.on('error', () => { fail++; codes.err = (codes.err || 0) + 1; setTimeout(() => connect(i), 5000) })
}

for (let i = 0; i < N + M; i++) setTimeout(() => connect(i), (RAMP * i) / (N + M))
setInterval(() => console.log(`${new Date().toISOString()} sse open=${open} opened=${opened} fail=${fail} ${JSON.stringify(codes)}`), 10000)
