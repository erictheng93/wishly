#!/usr/bin/env node
// CDN 外洩金絲雀：部署後（或排程）對正式網域檢查「受限清單不會被 Cloudflare 快取後給陌生人看」。
// 前置：用營運帳號建一份 visibility=friends 的金絲雀清單（至少 1 個品項），另一個金絲雀帳號加為好友。
//
//   API_BASE=https://api.wishly.tw CANARY_SLUG=AbCdEf1234 \
//   CANARY_FRIEND_COOKIE='ws_session=…'   # 選填但強烈建議：模擬「好友先載入」的真實外洩路徑
//   CANARY_PUBLIC_SLUG=XyZ1234567         # 選填：一份 public 清單，順便確認公開快取仍正常
//   node scripts/cdn-canary.mjs
//
// 任一檢查失敗 → exit 1（可接到部署流程或監控告警）。不需安裝任何套件（Node 18+ 內建 fetch）。
const { API_BASE, CANARY_SLUG, CANARY_FRIEND_COOKIE, CANARY_PUBLIC_SLUG } = process.env
if (!API_BASE || !CANARY_SLUG) { console.error('需要 API_BASE 與 CANARY_SLUG'); process.exit(2) }
const base = API_BASE.replace(/\/$/, '') + '/api/v1'
const fails = []
const check = (ok, msg) => { console.log(`${ok ? '✓' : '✗'} ${msg}`); if (!ok) fails.push(msg) }

async function get(path, cookie) {
  const r = await fetch(base + path, { headers: cookie ? { cookie } : {}, redirect: 'manual' })
  let body = null
  try { body = await r.json() } catch { /* 非 JSON */ }
  return { status: r.status, cache: r.headers.get('cf-cache-status'), cc: r.headers.get('cache-control'), body }
}
const leaked = (r) => r.status === 200 || Array.isArray(r.body?.items)

// 1) 好友先載入受限路徑（若有給 cookie），讓「錯誤的強制快取規則」有機會存下內容
if (CANARY_FRIEND_COOKIE) {
  const r = await get(`/restricted/wishlists/${CANARY_SLUG}`, CANARY_FRIEND_COOKIE)
  check(r.status === 200 && Array.isArray(r.body?.items), `好友可讀受限路徑（${r.status}）——否則金絲雀帳號 / 清單設定有誤`)
  check(r.cc?.includes('no-store') ?? false, `受限路徑回 no-store（cache-control: ${r.cc}）`)
}

// 2) 匿名連續讀兩條路徑各 3 次：都必須沒有清單內容，且不是從邊緣快取拿到的
for (const path of [`/restricted/wishlists/${CANARY_SLUG}`, `/public/wishlists/${CANARY_SLUG}`]) {
  for (let i = 1; i <= 3; i++) {
    const r = await get(path)
    check(!leaked(r) && [401, 403, 404].includes(r.status), `匿名 ${path} #${i} 未外洩（${r.status}）`)
    check(r.cache !== 'HIT', `匿名 ${path} #${i} 非快取命中（cf-cache-status: ${r.cache ?? '無'}）`)
  }
}

// 3) 對照組：public 清單應該照常可被快取（確認不是把整個 CDN 關掉才「通過」）
if (CANARY_PUBLIC_SLUG) {
  const r = await get(`/public/wishlists/${CANARY_PUBLIC_SLUG}`)
  check(r.status === 200 && (r.cc ?? '').startsWith('public'), `公開清單仍可快取（${r.status}，cache-control: ${r.cc}）`)
}

if (fails.length) { console.error(`\nCDN 金絲雀失敗 ${fails.length} 項：受限清單可能被快取外洩，請立即檢查 Cloudflare 快取規則。`); process.exit(1) }
console.log('\nCDN 金絲雀通過')
