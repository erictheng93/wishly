#!/usr/bin/env node
// Spike S3 量測：對目標鏈路（Cloudflare → Tunnel → VPS）持續連 SSE，記錄首位元組、心跳間隔、斷線與重連、事件延遲。
// 事件延遲 = 「捐助 POST 回應完成」到「SSE 收到對應 item.updated」。只在測試清單上用 TRIGGER，會建立真實捐助。
//
//   API_BASE=https://api.wishly.tw SLUG=AbCdEf1234 DURATION_MIN=120 \
//   ITEM_ID=<uuid> TRIGGER_EVERY_S=60 OUT=spike-s3.jsonl node scripts/sse-probe.mjs
//
// 沒設 ITEM_ID 就只量連線（TTFB、心跳、斷線）。OUT 的每行是一筆原始事件，可直接附在證據裡。
// 不需安裝套件（Node 18+）。伺服器每條連線上限 30 分鐘（dashboard.rs），所以測試至少要跑 > 30 分鐘才看得到重連。
import { appendFileSync, writeFileSync } from 'node:fs'
import { randomUUID } from 'node:crypto'

const { API_BASE, SLUG, ITEM_ID, ACCESS, OUT = 'spike-s3.jsonl' } = process.env
const DURATION_MIN = Number(process.env.DURATION_MIN ?? 120)
const TRIGGER_EVERY_S = Number(process.env.TRIGGER_EVERY_S ?? 60)
if (!API_BASE || !SLUG) { console.error('需要 API_BASE 與 SLUG'); process.exit(2) }
const base = API_BASE.replace(/\/$/, '') + '/api/v1'
const url = `${base}/public/wishlists/${SLUG}/events${ACCESS ? `?access=${encodeURIComponent(ACCESS)}` : ''}`
const t0 = Date.now(), endAt = t0 + DURATION_MIN * 60_000
writeFileSync(OUT, '')
const log = (o) => { const r = { t: new Date().toISOString(), ...o }; appendFileSync(OUT, JSON.stringify(r) + '\n'); if (o.k !== 'hb') console.log(JSON.stringify(r)) }

const st = { connects: 0, unexpectedDrops: 0, capCloses: 0, ttfb: [], hbGaps: [], lat: [], triggers: 0, missed: 0 }
const pending = [] // { start, done, ev, settled }
let lastByte = 0

async function once() {
  const started = Date.now(); st.connects++
  let res
  try { res = await fetch(url, { headers: { accept: 'text/event-stream' }, signal: AbortSignal.timeout(endAt - Date.now() + 1000) }) }
  catch (e) { log({ k: 'connect_error', err: String(e) }); st.unexpectedDrops++; return }
  const ttfb = Date.now() - started
  if (res.status !== 200) { log({ k: 'bad_status', status: res.status, cf: res.headers.get('cf-cache-status') }); st.unexpectedDrops++; return }
  st.ttfb.push(ttfb)
  log({ k: 'open', ttfb_ms: ttfb, ctype: res.headers.get('content-type'), cf_ray: res.headers.get('cf-ray') })
  lastByte = Date.now()
  const dec = new TextDecoder(); let buf = ''
  try {
    for await (const chunk of res.body) {
      const now = Date.now()
      st.hbGaps.push(now - lastByte)
      lastByte = now
      buf += dec.decode(chunk, { stream: true })
      let i
      while ((i = buf.indexOf('\n\n')) >= 0) {
        const block = buf.slice(0, i); buf = buf.slice(i + 2)
        if (block.startsWith(':')) { log({ k: 'hb' }); continue }
        const ev = Object.fromEntries(block.split('\n').map(l => { const j = l.indexOf(':'); return [l.slice(0, j), l.slice(j + 1).trim()] }))
        log({ k: 'event', event: ev.event ?? 'message', id: ev.id, retry: ev.retry })
        if (ev.event === 'item.updated') {
          const p = pending.find(x => x.ev === null)
          if (p) { p.ev = Date.now(); settle(p) }
        }
      }
    }
    const dur = Date.now() - started
    // 伺服器 30 分鐘上限屬預期關閉；明顯更早就是被鏈路切斷
    if (dur >= 29 * 60_000) { st.capCloses++; log({ k: 'closed_by_cap', dur_s: Math.round(dur / 1000) }) }
    else { st.unexpectedDrops++; log({ k: 'unexpected_close', dur_s: Math.round(dur / 1000) }) }
  } catch (e) { if (Date.now() < endAt) { st.unexpectedDrops++; log({ k: 'stream_error', err: String(e), after_s: Math.round((Date.now() - started) / 1000) }) } }
}

// 事件可能早於 POST 回應抵達（commit 後即廣播），所以先登記、兩邊都到才結算；延遲 = max(0, 事件 − 回應完成)
function settle(p) {
  if (p.done === null || p.ev === null || p.settled) return
  p.settled = true
  const d = Math.max(0, p.ev - p.done); st.lat.push(d)
  log({ k: 'latency', ms: d, from_post_start_ms: p.ev - p.start })
  pending.splice(pending.indexOf(p), 1)
}
async function trigger() {
  const p = { start: Date.now(), done: null, ev: null, settled: false }
  pending.push(p); st.triggers++
  const r = await fetch(`${base}/items/${ITEM_ID}/claims`, {
    method: 'POST', headers: { 'content-type': 'application/json', 'idempotency-key': randomUUID() },
    body: JSON.stringify({ qty: 1, display_name: 'spike-s3' }),
  }).catch(e => ({ status: 0, err: String(e) }))
  p.done = Date.now()
  log({ k: 'trigger', status: r.status, post_ms: p.done - p.start })
  if (r.status !== 201) { st.missed++; p.settled = true; pending.splice(pending.indexOf(p), 1); return }
  settle(p)
  setTimeout(() => { if (!p.settled) { p.settled = true; const i = pending.indexOf(p); if (i >= 0) pending.splice(i, 1); st.missed++; log({ k: 'event_missed_10s' }) } }, 10_000)
}

const pct = (a, q) => a.length ? [...a].sort((x, y) => x - y)[Math.min(a.length - 1, Math.floor(a.length * q))] : null
let trig
if (ITEM_ID) trig = setInterval(trigger, TRIGGER_EVERY_S * 1000)
while (Date.now() < endAt) { await once(); if (Date.now() < endAt) await new Promise(r => setTimeout(r, 5000)) } // 與伺服器 retry: 5s 一致
clearInterval(trig)
const summary = {
  duration_min: DURATION_MIN, connects: st.connects, unexpected_drops: st.unexpectedDrops, closed_by_server_cap: st.capCloses,
  ttfb_ms: { p50: pct(st.ttfb, .5), p95: pct(st.ttfb, .95), max: pct(st.ttfb, 1) },
  byte_gap_ms: { p50: pct(st.hbGaps, .5), max: pct(st.hbGaps, 1), over_30s: st.hbGaps.filter(g => g > 30_000).length },
  triggers: st.triggers, events_missed: st.missed,
  event_latency_ms: { n: st.lat.length, p50: pct(st.lat, .5), p95: pct(st.lat, .95), max: pct(st.lat, 1) },
}
log({ k: 'summary', ...summary })
console.log('\n' + JSON.stringify(summary, null, 2))
