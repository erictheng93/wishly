// 許可到期背景工作（relief_events::permit_expiry_tick）驗證：org.mjs 的 O-55 為專屬測試組織建立上線據點、並把其許可快轉到過期但「不動」該據點，
// 本腳本等到 API 的下一次每小時 tick 後，檢查該據點是否被自動暫停、是否寄出 org.permit_expired 通知與信。
// 無法在不重啟 API 的前提下手動觸發 tick，只能等；逾時（預設 70 分鐘，TICK_WAIT_MIN 可調）就記為 blocked。
// 環境變數：TICK_STATE（org.mjs 寫的狀態檔，預設 org-state.json）、TICK_OUT（輸出檔，預設 org-tick.json）。
import { writeFileSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { OUT, listMail, sleep, sql } from './lib.mjs'

const S = JSON.parse(readFileSync(resolve(OUT, process.env.TICK_STATE ?? 'org-state.json'), 'utf8'))
if (!S.tickSiteId) { console.error('狀態檔沒有 tickSiteId（O-55 未跑到）'); process.exit(2) }
const deadline = Date.now() + (Number(process.env.TICK_WAIT_MIN ?? 70)) * 60_000
const q = () => sql(`SELECT w.site_status||'|'||w.status||'|'||(now() AT TIME ZONE 'Asia/Taipei')::date||'|'||pm.valid_to FROM wishlists w JOIN org_permits pm ON pm.id=w.permit_id WHERE w.id='${S.tickSiteId}'`)
const t0 = new Date().toISOString(), s0 = q()
console.log('開始', t0, s0)
let out = { start: t0, before: s0 }
while (Date.now() < deadline) {
  const s = q()
  if (s.startsWith('paused')) {
    const notif = sql(`SELECT kind||':'||status FROM notifications WHERE kind='org.permit_expired' AND payload->>'org_id'='${S.tickOrgId}'`)
    const mails = (await listMail(S.tickOwnerEmail)).map(m => `${m.subject} @ ${m.created}`)
    out = { ...out, detectedAt: new Date().toISOString(), after: s, notification: notif, ownerMails: mails, status: 'pass' }
    break
  }
  await sleep(30_000)
}
out.status ??= 'blocked'
out.finalState = q()
writeFileSync(resolve(OUT, process.env.TICK_OUT ?? 'org-tick.json'), JSON.stringify(out, null, 1))
console.log(JSON.stringify(out, null, 1))
