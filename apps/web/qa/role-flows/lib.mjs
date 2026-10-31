// 角色流程模擬（docs/role-flows/）共用工具：以真實瀏覽器點擊 Web（:3000）驅動 API（:8080）。
// 環境：資料庫 wishly_flow、Mailpit :8025、S3 相容服務 :9000；啟動方式見 docs/role-flows/README.md。
// 每個步驟以 step() 包起來：記錄結果（pass / fail / blocked / gap）、截圖、console 與 HTTP 錯誤，最後寫 out/<role>.json。
import { chromium, request as pwRequest } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { randomBytes, randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { shortenInboxStream } from '../inbox-stream.mjs'

export const HERE = dirname(fileURLToPath(import.meta.url))
export const ROOT = resolve(HERE, '../../../..')
export const OUT = resolve(HERE, '../out/role-flows')
export const API = process.env.FLOW_API ?? 'http://localhost:8080'
export const WEB = process.env.FLOW_WEB ?? 'http://localhost:3000'
export const MAILPIT = process.env.FLOW_MAILPIT ?? 'http://localhost:8025'
export const DB = process.env.FLOW_DB ?? 'wishly_flow'
// 本機拋棄式測試帳號的密碼：每次執行隨機產生，不在原始碼留固定值
export const randomSecret = () => ['Fw1', randomBytes(9).toString('base64url')].join('-')
export const PASSWORD = randomSecret()
export const uid = () => randomBytes(4).toString('hex')
export { chromium, pwRequest, randomUUID }
export const sleep = ms => new Promise(r => setTimeout(r, ms))

/** 直接查 DB（只用於讀取驗證或模擬「平台外」動作，如把帳號設成 staff；每次使用都要在步驟裡註明） */
export function sql(stmt) {
  return execFileSync('docker', ['compose', 'exec', '-T', 'db', 'psql', '-U', 'wishly', '-d', DB, '-v', 'ON_ERROR_STOP=1', '-tA', '-c', stmt], { cwd: ROOT, encoding: 'utf8' }).trim()
}
export const q = s => String(s).replace(/'/g, "''")
/** 所有流程從同一 IP 打 API，每 IP 的 OTP 限流會累積；建帳號前清掉（真實使用者不會遇到） */
export const resetOtpLimits = () => sql(`DELETE FROM rate_limits WHERE key LIKE 'otp\\_%'`)

// ---------- Mailpit ----------
export async function waitForMail(to, { subject, after = 0, timeout = 20000 } = {}) {
  const api = await pwRequest.newContext()
  const t0 = Date.now()
  try {
    while (Date.now() - t0 < timeout) {
      const r = await api.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:"${to}"` } })
      const list = (await r.json()).messages ?? []
      const f = list.find(m => (!subject || subject.test(m.Subject)) && Date.parse(m.Created) >= after)
      if (f) { const m = await (await api.get(`${MAILPIT}/api/v1/message/${f.ID}`)).json(); return { id: f.ID, subject: f.Subject, text: m.Text, html: m.HTML } }
      await sleep(400)
    }
    throw new Error(`等不到寄給 ${to} 的信${subject ? ` (${subject})` : ''}`)
  } finally { await api.dispose() }
}
export async function listMail(to) {
  const api = await pwRequest.newContext()
  try { return ((await (await api.get(`${MAILPIT}/api/v1/search`, { params: { query: `to:"${to}"` } })).json()).messages ?? []).map(m => ({ subject: m.Subject, created: m.Created })) }
  finally { await api.dispose() }
}
export const otpFrom = text => text.match(/(\d{6})/)?.[1]

// ---------- API ----------
export const apiCtx = (storageState) => pwRequest.newContext({ baseURL: API + '/api/v1/', extraHTTPHeaders: { Origin: WEB }, storageState })

/** 以 API 快速建立已驗證帳號（UI 註冊流程本身由使用者流程覆蓋；其他角色用這個省時間） */
export async function registerApi({ name, tag = 'u', staff = false } = {}) {
  const email = `flow-${tag}-${uid()}@example.com`, display = name ?? `用戶${uid().slice(0, 4)}`
  resetOtpLimits()
  const api = await apiCtx()
  const t0 = Date.now() - 1000
  let r = await api.post('auth/register', { data: { email, password: PASSWORD, display_name: display } })
  if (!r.ok()) throw new Error('register ' + r.status() + await r.text())
  const { challenge_id } = await r.json()
  const m = await waitForMail(email, { after: t0 })
  r = await api.post('auth/register/verify', { data: { email, code: otpFrom(m.text), challenge_id } })
  if (!r.ok()) throw new Error('verify ' + r.status() + await r.text())
  if (staff) sql(`UPDATE users SET is_staff = true WHERE email = '${q(email)}'`)
  return { email, password: PASSWORD, name: display, api }
}

// ---------- 瀏覽器 ----------
let browser
export async function getBrowser() { // 專案 pin 的 Playwright 版本可能找不到內建瀏覽器；FLOW_CHROMIUM 或沙盒預裝路徑優先
  const executablePath = process.env.FLOW_CHROMIUM ?? (existsSync('/opt/pw-browsers/chromium') ? '/opt/pw-browsers/chromium' : undefined)
  return browser ??= await chromium.launch({ executablePath }) }
export async function closeBrowser() { await browser?.close(); browser = undefined }

/** 新瀏覽器情境（一個「人」一個 context）；mobile=true 用 390 寬 */
export async function newPage({ mobile = false, user } = {}) {
  const b = await getBrowser()
  const ctx = await b.newContext({ baseURL: WEB, locale: 'zh-TW', timezoneId: 'Asia/Taipei', viewport: mobile ? { width: 390, height: 844 } : { width: 1280, height: 860 }, isMobile: mobile, hasTouch: mobile })
  await shortenInboxStream(ctx)
  if (user) await ctx.addCookies((await user.api.storageState()).cookies.map(c => ({ ...c, domain: 'localhost' })))
  const page = await ctx.newPage()
  page.problems = []
  page.on('pageerror', e => page.problems.push(`pageerror: ${e.message}`.slice(0, 300)))
  page.on('console', m => { if (m.type() === 'error') page.problems.push(`console: ${m.text()}`.slice(0, 300)) })
  page.on('response', r => { if (r.status() >= 500) page.problems.push(`http${r.status()} ${r.request().method()} ${r.url()}`) })
  return page
}

/** UI 登入（密碼）。登入頁的實際欄位以 app/pages/login.vue 為準 */
export async function uiLogin(page, email, password = PASSWORD) {
  await page.goto('/login')
  await page.getByLabel(/email|電子郵件/i).first().fill(email)
  await page.getByLabel(/密碼/).first().fill(password)
  await page.getByRole('button', { name: /^登入$/ }).click()
  await page.waitForURL(u => !u.pathname.startsWith('/login'), { timeout: 15000 })
}

// ---------- 步驟紀錄 ----------
/**
 * const run = recorder('user')
 * await run.step('U-01', '註冊帳號', async ({ page, note }) => { ...; note('觀察') }, { page })
 * 回呼丟出 Blocked / Gap 例外時分別記為 blocked / gap；其他例外記 fail。
 */
export class Blocked extends Error {}
export class Gap extends Error {}
export function recorder(role) {
  const dir = resolve(OUT, role)
  mkdirSync(dir, { recursive: true })
  const results = []
  return {
    results,
    async step(id, title, fn, { page } = {}) {
      const notes = []
      const t0 = Date.now()
      const before = page?.problems?.length ?? 0
      let status = 'pass', error
      try { await fn({ note: s => notes.push(s) }) }
      catch (e) { status = e instanceof Blocked ? 'blocked' : e instanceof Gap ? 'gap' : 'fail'; error = String(e?.message ?? e).slice(0, 800) }
      let shot
      if (page && !page.isClosed()) {
        shot = `${id}.png`
        await page.screenshot({ path: resolve(dir, shot), fullPage: true }).catch(() => { shot = undefined })
      }
      const problems = page?.problems?.slice(before) ?? []
      const r = { id, title, status, ms: Date.now() - t0, notes, error, url: page && !page.isClosed() ? page.url() : undefined, shot, problems }
      results.push(r)
      console.log(`[${status.toUpperCase().padEnd(7)}] ${id} ${title}${error ? ' — ' + error.split('\n')[0] : ''}`)
      return status === 'pass'
    },
    save() {
      writeFileSync(resolve(OUT, `${role}.json`), JSON.stringify(results, null, 1))
      const c = results.reduce((a, r) => (a[r.status] = (a[r.status] ?? 0) + 1, a), {})
      console.log(`\n${role}: ${JSON.stringify(c)} → ${resolve(OUT, role + '.json')}`)
    },
  }
}
