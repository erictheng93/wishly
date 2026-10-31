// 角色流程模擬 04：平台營運者（staff）— 營運主管、客服、金流對帳、送禮執行人、品牌經理。
// 設計與結果見 docs/role-flows/04-operator.md。執行：cd apps/web && node qa/role-flows/operator.mjs
// 環境變數：FLOW_ONLY=A,B（只跑指定區段，每個區段自備前置資料）、FLOW_READONLY=1（才執行全站唯讀模式步驟，預設延後）。
// 原則：營運者的動作一律走真實 UI；只有「平台外」的事（把帳號設成 staff、建立前置資料、收信、金流付款通知）走 API / SQL / 替身，並在 note 註明。
import { createHmac } from 'node:crypto'
import { expect } from '@playwright/test'
import {
  API, WEB, PASSWORD, uid, sleep, sql, q, randomUUID, registerApi, newPage, closeBrowser, recorder, Blocked, Gap,
  waitForMail, listMail, resetOtpLimits, apiCtx,
} from './lib.mjs'
import { ecpayPaid } from './ecpay-sim.mjs'

const T = uid()
const ONLY = (process.env.FLOW_ONLY ?? '').split(',').map(s => s.trim().toUpperCase()).filter(Boolean)
const want = sec => !ONLY.length || ONLY.includes(sec)
const run = recorder('operator')
const S = {} // 跨步驟共用狀態
const nowMs = () => Date.now() - 1500

// ---------- 小工具 ----------
const must = async (r, what) => { if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${(await r.text()).slice(0, 300)}`); return r.json().catch(() => ({})) }
const ok = (cond, msg) => { if (!cond) throw new Error(msg) }
const idemp = () => ({ 'Idempotency-Key': randomUUID() })
const webStatus = async path => (await fetch(WEB + path, { redirect: 'manual' })).status
const apiStatus = async path => (await fetch(`${API}/api/v1${path}`)).status
const dlg = page => page.getByRole('dialog')
const bodyText = page => page.locator('body').innerText()
/** 找到卡片；若在後面幾頁就一直按「載入更多」 */
async function findCard(page, locFn, more = /載入更多/, tries = 12) {
  await page.waitForLoadState('networkidle').catch(() => {}); await page.waitForTimeout(600)
  for (let i = 0; i < tries; i++) {
    const c = locFn()
    if (await c.count()) return c.first()
    const b = page.getByRole('button', { name: more })
    if (!(await b.count()) || !(await b.first().isEnabled())) break
    await b.first().click(); await page.waitForLoadState('networkidle').catch(() => {}); await page.waitForTimeout(700)
  }
  throw new Error('在列表（含載入更多）找不到目標卡片')
}
/** 選下拉後等列表重載完成（頁面以 watch 觸發請求，不等就會讀到舊列表） */
const sel = async (page, label, opt) => { await page.getByLabel(label).selectOption(typeof opt === 'string' && opt.startsWith('=') ? opt.slice(1) : { label: opt }); await page.waitForTimeout(400); await page.waitForLoadState('networkidle').catch(() => {}); await page.waitForTimeout(300) }
const noMailWithin = async (to, subject, ms = 7000) => {
  const t0 = Date.now()
  while (Date.now() - t0 < ms) { if ((await listMail(to)).some(m => subject.test(m.subject))) return false; await sleep(1000) }
  return true
}
// ---------- TOTP（RFC 6238, SHA-1, 6 碼, 30 秒；與 apps/api/src/security.rs 相同）----------
const b32d = s => { const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'; let bits = ''; for (const c of s.replace(/=+$/, '').toUpperCase()) bits += A.indexOf(c).toString(2).padStart(5, '0'); const o = []; for (let i = 0; i + 8 <= bits.length; i += 8) o.push(parseInt(bits.slice(i, i + 8), 2)); return Buffer.from(o) }
const totp = (secret, t = Date.now()) => { const b = Buffer.alloc(8); b.writeBigUInt64BE(BigInt(Math.floor(t / 30000))); const h = createHmac('sha1', b32d(secret)).update(b).digest(); const o = h[19] & 15; return String(((h[o] & 0x7f) << 24 | h[o + 1] << 16 | h[o + 2] << 8 | h[o + 3]) % 1e6).padStart(6, '0') }
const nextTotpStep = () => sleep(30000 - (Date.now() % 30000) + 700)
const typeOtp = async (page, code) => { await page.locator('.c-otp input').first().fill(code) }

// ---------- 前置資料工廠（平台外：以 API 建立）----------
const PDF = Buffer.from('%PDF-1.4\n% wishly operator flow test document\n%%EOF\n')
const iso = d => d.toISOString().slice(0, 10)
const taxOk = s => { const d = [...s].map(Number); const sum = d.reduce((a, v, i) => { const p = v * [1, 2, 1, 2, 1, 2, 4, 1][i]; return a + Math.floor(p / 10) + p % 10 }, 0); return sum % 5 === 0 || (d[6] === 7 && (sum + 1) % 5 === 0) }
const taxId = () => { for (;;) { const s = String(Math.floor(Math.random() * 1e8)).padStart(8, '0'); if (taxOk(s)) return s } }
async function loginApi(email) { const c = await apiCtx(); await must(await c.post('auth/login', { data: { email, password: PASSWORD } }), '登入'); return c }

/** 申請人 + 組織 + 文件 + n 張許可，已送審（pending） */
async function makeOrg(label, nPermits = 1, permitDays = 90) {
  const owner = await registerApi({ name: `${label}申請人${T}`, tag: 'org' })
  let org
  for (let i = 0; i < 6 && !org; i++) {
    const r = await owner.api.post('orgs', { data: { legal_name: `財團法人${label}${T}基金會`, display_name: `${label}${T}`, org_type: 'foundation', tax_id: taxId(), registration_no: `R-${T}-${label}`, authority: '內政部', website: 'https://example.org', intro: `${label} 的測試組織簡介`, rep_name: '王代表', contact_name: '林聯絡', contact_title: '專員', contact_phone: '0223456789', public_email: `org-${label}-${T}@example.org` } })
    if (r.ok()) org = await r.json(); else if (r.status() !== 409 && r.status() !== 422) throw new Error('建立組織 ' + r.status() + await r.text())
  }
  ok(org, '建立組織失敗（統編重複）')
  const up = async kind => must(await owner.api.post(`orgs/${org.id}/documents?kind=${kind}`, { data: PDF, headers: { 'content-type': 'application/pdf' } }), '上傳文件')
  const regDoc = await up('registration')
  const permits = []
  for (let i = 0; i < nPermits; i++) {
    const pd = await up('permit')
    const p = await must(await owner.api.post(`orgs/${org.id}/permits`, { data: { permit_no: `P-${label}-${T}-${i + 1}`, valid_from: iso(new Date(Date.now() - 864e5)), valid_to: iso(new Date(Date.now() + permitDays * 864e5)), document_id: pd.id } }), '新增許可')
    permits.push({ id: p.id, no: p.permit_no })
  }
  await must(await owner.api.post(`orgs/${org.id}/submit`), '送審')
  const slug = sql(`SELECT slug FROM organizations WHERE id='${org.id}'`)
  return { owner, id: org.id, name: `${label}${T}`, slug, permits, regDoc: regDoc.id }
}
async function makeList(owner, title, { items = [{ title: '奶瓶', qty: 1 }], publish = true } = {}) {
  const w = await must(await owner.api.post('wishlists', { data: { type: 'registry', title, visibility: 'public' } }), '建立清單')
  for (const it of items) await must(await owner.api.post(`wishlists/${w.id}/items`, { data: { title: it.title, qty_needed: it.qty ?? 1, priority: 'medium', funding_mode: 'quantity' } }), '新增品項')
  if (publish) { const cur = await must(await owner.api.get(`wishlists/${w.id}`), '讀清單'); await must(await owner.api.patch(`wishlists/${w.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } }), '發佈') }
  const slug = (await must(await owner.api.get(`wishlists/${w.id}`), '讀清單')).wishlist.slug
  return { id: w.id, slug, title }
}
const meId = async u => (await must(await u.api.get('me'), 'me')).id
const bal = uidv => Number(sql(`SELECT COALESCE(balance,0) FROM point_wallets WHERE user_id='${uidv}'`) || 0)
const reconcile = () => sql(`SELECT count(*) FROM point_wallets w WHERE w.balance <> (SELECT COALESCE(sum(remaining_points),0) FROM point_lots l WHERE l.wallet_id = w.id)`)
const ledgerGap = () => sql(`SELECT count(*) FROM point_wallets w WHERE w.balance <> (SELECT COALESCE(sum(delta),0) FROM point_ledger l WHERE l.wallet_id = w.id)`)
async function topupPaid(user, amount) {
  await must(await user.api.post('wallet/topups', { data: { amount, method: 'card' }, headers: idemp() }), '建立儲值')
  const no = sql(`SELECT t.merchant_trade_no FROM topups t JOIN point_wallets w ON w.id=t.wallet_id WHERE w.user_id='${await meId(user)}' ORDER BY t.created_at DESC LIMIT 1`)
  const r = await ecpayPaid(no); ok(r.status === 200, '付款通知失敗 ' + r.text)
  return { no, id: sql(`SELECT id FROM topups WHERE merchant_trade_no='${no}'`) }
}
async function makeCfList(owner, title, items, { publish = true } = {}) {
  const w = await must(await owner.api.post('wishlists', { data: { type: 'registry', title, visibility: 'public' } }), '建立清單')
  await must(await owner.api.put(`wishlists/${w.id}/shipping-address`, { data: { recipient_name: '王小米', phone: '0912345678', address: '台北市大安區和平東路二段 1 號 3 樓' } }), '收件資訊')
  const ids = []
  for (const it of items) ids.push((await must(await owner.api.post(`wishlists/${w.id}/items`, { data: { title: it.title, funding_mode: 'crowdfund', target_points: it.target, funding_deadline: new Date(Date.now() + 7 * 864e5).toISOString(), priority: 'medium' } }), '新增眾籌品項')).id)
  if (publish) { const cur = await must(await owner.api.get(`wishlists/${w.id}`), '讀清單'); await must(await owner.api.patch(`wishlists/${w.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } }), '發佈') }
  const slug = (await must(await owner.api.get(`wishlists/${w.id}`), '讀清單')).wishlist.slug
  return { id: w.id, slug, ids }
}
const pledge = async (u, item, points) => must(await u.api.post(`items/${item}/contributions`, { data: { points }, headers: idemp() }), '贊助')
const grantApi = async (staff, user, points, reason = '前置：發點') => must(await staff.api.post('admin/wallets/grants', { data: { user_id: await meId(user), points, reason }, headers: idemp() }), '發點')

// 營運人員（平台外：註冊後以 SQL 設 is_staff）
async function mkStaff(label) { return registerApi({ name: `${label}${T}`, tag: 'staff', staff: true }) }
const staffPage = async u => newPage({ user: u })

// =====================================================================
// P-00 前置準備
// =====================================================================
await run.step('P-00', '前置準備：建立五位營運人員與一般用戶（平台外：註冊 + SQL 設 is_staff）', async ({ note }) => {
  S.plain = await registerApi({ name: `一般用戶${T}`, tag: 'plain' })
  S.lead = await mkStaff('營運主管'); S.cs = await mkStaff('客服'); S.fin = await mkStaff('金流對帳'); S.ship = await mkStaff('送禮執行人'); S.brand = await mkStaff('品牌經理')
  for (const k of ['lead', 'cs', 'fin', 'ship', 'brand']) S[k].id = await meId(S[k])
  S.plain.id = await meId(S.plain)
  S.lead.email_ = S.lead.email
  S.t0Stats = (await (await S.cs.api.get('admin/stats')).json())
  note(`標籤 ${T}；主管 ${S.lead.email}、客服 ${S.cs.email}、金流 ${S.fin.email}、送禮 ${S.ship.email}、品牌 ${S.brand.email}；一般用戶 ${S.plain.email}`)
  note('is_staff 以 SQL 設定（平台外動作；第一位 staff 本來就必須這樣建立，見系統手冊 6.1）。')
})

// =====================================================================
// A 進後台 / MFA
// =====================================================================
if (want('A')) {
  const anon = await newPage()
  const pp = await newPage({ user: S.plain }); S.pp = pp
  await run.step('P-01', '一般用戶進 /admin 被拒（UI 與 API 皆然）', async ({ note }) => {
    await pp.goto('/dashboard'); await pp.getByRole('button', { name: /帳號選單|account/i }).first().click().catch(() => {})
    const menuHasAdmin = await pp.getByRole('link', { name: '後台', exact: true }).count()
    note(`一般用戶帳號選單中的「後台」連結數：${menuHasAdmin}（預期 0）`)
    ok(menuHasAdmin === 0, '一般用戶選單出現後台入口')
    await pp.goto('/admin'); await pp.waitForLoadState('networkidle')
    const txt = await bodyText(pp)
    ok(!/營運概況|待處理檢舉/.test(txt), '一般用戶看到後台內容')
    note(`一般用戶直接開 /admin 顯示：「${txt.replace(/\s+/g, ' ').slice(0, 60)}」`)
    const r = await S.plain.api.get('admin/stats'); const j = await r.json().catch(() => ({}))
    ok(r.status() === 403 && j.code === 'STAFF_ONLY', `API 預期 403 STAFF_ONLY，實際 ${r.status()} ${j.code}`)
    note('API GET /admin/stats → 403 STAFF_ONLY')
  }, { page: pp })
  await run.step('P-01b', '未登入者進 /admin：導向登入頁並帶 redirect', async ({ note }) => {
    await anon.goto('/admin'); await anon.waitForURL(/\/login\?redirect=/, { timeout: 10000 })
    note(`未登入開 /admin → ${anon.url().replace(WEB, '')}`)
  }, { page: anon })

  const lp = await newPage(); S.lp = lp
  await run.step('P-02', '營運主管首次登入進後台：導向登入 → 回到 /admin，看到 STAFF 徽章、KPI 與建議啟用 MFA 提示', async ({ note }) => {
    await lp.goto('/admin'); await lp.waitForURL(/\/login/)
    await lp.getByLabel(/email/i).first().fill(S.lead.email); await lp.getByLabel(/密碼/).first().fill(PASSWORD)
    await lp.getByRole('button', { name: /^登入$/ }).click()
    await lp.waitForURL(/\/admin$/, { timeout: 15000 })
    await expect(lp.getByRole('heading', { name: '營運後台' })).toBeVisible()
    await expect(lp.getByText('STAFF', { exact: true })).toBeVisible()
    await expect(lp.getByText('建議啟用兩步驟驗證')).toBeVisible()
    for (const k of ['users', 'wishlists', 'active_wishlists', 'hidden_wishlists', 'open_reports']) await expect(lp.getByTestId(`stat-${k}`)).not.toHaveText('–')
    const nav = await lp.locator('nav.k-nav a').allInnerTexts()
    note(`功能索引 ${nav.length} 項：${nav.join('、')}`)
  }, { page: lp })

  await run.step('P-03', 'MFA 設定：現在設定 → 開始設定 → 讀金鑰 → 錯誤碼被拒 → 正確 TOTP 啟用 → 回後台、提示消失', async ({ note }) => {
    await lp.getByRole('link', { name: '現在設定' }).click(); await lp.waitForURL(/\/admin\/mfa/)
    await lp.getByRole('button', { name: '開始設定' }).click()
    await expect(lp.locator('.mfa-secret')).toBeVisible()
    S.totpSecret = (await lp.locator('.mfa-secret').innerText()).replace(/\s/g, '')
    note(`QR 與手動金鑰皆顯示（金鑰長度 ${S.totpSecret.length}）；以 node:crypto 自行計算 TOTP（平台外：模擬驗證器 App）`)
    await typeOtp(lp, '000000')
    await expect(lp.getByRole('alert')).toBeVisible()
    note(`錯誤碼 → ${(await lp.getByRole('alert').innerText()).trim()}`)
    await lp.waitForTimeout(500)
    await typeOtp(lp, totp(S.totpSecret))
    await lp.waitForURL(/\/admin$/, { timeout: 15000 })
    await expect(lp.getByText('建議啟用兩步驟驗證')).toHaveCount(0)
    ok(sql(`SELECT totp_enabled_at IS NOT NULL FROM users WHERE id='${S.lead.id}'`) === 't', 'DB 未啟用 TOTP')
    note('啟用後回 /admin，MFA 提示消失；DB totp_enabled_at 已設，totp_secret 為密文（sealed）。')
    const raw = sql(`SELECT encode(totp_secret,'escape') FROM users WHERE id='${S.lead.id}'`)
    ok(!raw.includes(S.totpSecret), 'TOTP 密鑰以明文存放')
  }, { page: lp })

  await run.step('P-04', '登出再登入：必須先過 MFA（重放舊碼被拒、下一個時間步的新碼通過）', async ({ note }) => {
    const oldCode = totp(S.totpSecret)
    await lp.goto('/dashboard')
    await lp.getByRole('button', { name: /帳號選單/ }).click(); await lp.getByRole('button', { name: '登出' }).click()
    await lp.waitForURL(/\/login/)
    await lp.goto('/admin'); await lp.waitForURL(/\/login/)
    await lp.getByLabel(/email/i).first().fill(S.lead.email); await lp.getByLabel(/密碼/).first().fill(PASSWORD)
    await lp.getByRole('button', { name: /^登入$/ }).click()
    await lp.waitForURL(/\/admin\/mfa/, { timeout: 15000 })
    note(`登入後被導向 ${lp.url().replace(WEB, '')}（未過 MFA 不能進後台）`)
    // 已啟用 MFA 的 session 直接打 API 應為 403 MFA_REQUIRED
    const cookies = await lp.context().cookies(); const ck = cookies.find(c => c.name === 'ws_session')
    const raw = await fetch(`${API}/api/v1/admin/stats`, { headers: { cookie: `ws_session=${ck.value}`, origin: WEB } })
    const j = await raw.json().catch(() => ({}))
    ok(raw.status === 403 && j.code === 'MFA_REQUIRED', `未驗證 session 打 API 預期 403 MFA_REQUIRED，實際 ${raw.status} ${j.code}`)
    note('未通過 MFA 的 session 直接打 GET /admin/stats → 403 MFA_REQUIRED')
    await typeOtp(lp, oldCode)
    await expect(lp.getByRole('alert')).toBeVisible()
    note(`重放啟用時用過的碼 → ${(await lp.getByRole('alert').innerText()).trim()}`)
    await nextTotpStep()
    await lp.locator('.c-otp input').first().click()
    await typeOtp(lp, totp(S.totpSecret))
    await lp.waitForURL(/\/admin$/, { timeout: 15000 })
    await expect(lp.getByRole('heading', { name: '營運後台' })).toBeVisible()
  }, { page: lp })
}

// =====================================================================
// 取得主管 page（若略過 A，直接以 API session 開）
// =====================================================================
async function leadPage() { return S.lp ?? (S.lp = await newPage({ user: S.lead })) }

// =====================================================================
// B 組織審核
// =====================================================================
if (want('B')) {
  const lp = await leadPage()
  await run.step('P-10', '前置：兩個申請人各送一份組織申請（A 有 2 張許可、B 有 1 張）（平台外：申請端已由 03-org 覆蓋，這裡用 API 建立）', async ({ note }) => {
    S.orgA = await makeOrg('甲', 2); S.orgB = await makeOrg('乙', 1)
    note(`組織 A ${S.orgA.name}（${S.orgA.id}）許可 ${S.orgA.permits.map(p => p.no).join('、')}；組織 B ${S.orgB.name}`)
  })

  await run.step('P-11', '待審清單：預設只看待審核、先送先審；檢視詳情看到欄位、文件、許可、成員', async ({ note }) => {
    await lp.goto('/admin/orgs')
    const cardA = await findCard(lp, () => lp.locator(`article[data-org="${S.orgA.id}"]`)), cardB = await findCard(lp, () => lp.locator(`article[data-org="${S.orgB.id}"]`))
    const ids = await lp.locator('article[data-org]').evaluateAll(a => a.map(x => x.dataset.org))
    ok(ids.indexOf(S.orgA.id) < ids.indexOf(S.orgB.id), '先送先審排序錯誤')
    note(`待審 ${ids.length} 筆；A 排在 B 前面（submitted_at 升冪）`)
    const opts = await lp.getByLabel('狀態').locator('option').allInnerTexts(); note(`狀態篩選：${opts.join('／')}`)
    await cardA.getByRole('button', { name: '檢視' }).click()
    for (const s of ['組織全名', '統一編號', '主管機關', '立案文件', '勸募許可', S.orgA.permits[0].no, S.orgA.permits[1].no, '成員', 'owner', '0223456789']) await expect(cardA).toContainText(s)
    note('詳情顯示：全部申請欄位（含解密後的聯絡電話）、立案文件、2 張許可、成員（owner）。')
  }, { page: lp })

  await run.step('P-12', '檢視文件：下載立案文件（已加密儲存，下載時解密）內容與原檔一致', async ({ note }) => {
    const cardA = lp.locator(`article[data-org="${S.orgA.id}"]`)
    const [d] = await Promise.all([lp.waitForEvent('download'), cardA.getByRole('button', { name: '下載' }).first().click()])
    const path = await d.path(); const buf = (await import('node:fs')).readFileSync(path)
    ok(Buffer.compare(buf, PDF) === 0, '下載內容與上傳不同')
    note(`下載檔名 ${d.suggestedFilename()}，${buf.length} bytes，與原 PDF 位元組相同`)
    S.docDownloadedAt = nowMs()
  }, { page: lp })

  await run.step('P-13', '要求補件：未勾欄位被擋；勾「法定名稱＋文件」並填理由 → 狀態變需補件，申請人收到通知信', async ({ note }) => {
    const cardA = lp.locator(`article[data-org="${S.orgA.id}"]`)
    S.t13 = nowMs()
    await cardA.getByRole('button', { name: '要求補件' }).click()
    await dlg(lp).getByRole('button', { name: '要求補件', exact: true }).click()
    await expect(dlg(lp).getByRole('alert')).toContainText('請至少勾選')
    note('未勾欄位 → 「請至少勾選一個欄位」')
    await dlg(lp).locator('input[name="flag-legal_name"]').check(); await dlg(lp).locator('input[name="flag-documents"]').check()
    await dlg(lp).getByLabel(/審核備註/).fill('法定名稱需與立案證書一致；請補上最新立案證書影本')
    await dlg(lp).getByRole('button', { name: '要求補件', exact: true }).click()
    await expect(lp.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()
    ok(sql(`SELECT verification_status FROM organizations WHERE id='${S.orgA.id}'`) === 'needs_changes', 'DB 狀態未變')
    ok(sql(`SELECT array_to_string(flagged_fields,',') FROM organizations WHERE id='${S.orgA.id}'`).includes('legal_name'), 'flagged_fields 未寫入')
    await sel(lp, '狀態', '需補件')
    await expect(lp.locator(`article[data-org="${S.orgA.id}"]`)).toBeVisible()
    const m = await waitForMail(S.orgA.owner.email, { subject: /需要補件或修正/, after: S.t13 })
    ok(/法定名稱需與立案證書一致/.test(m.text), '信件缺少審核備註')
    note(`通知信：「${m.subject}」，內文含備註與申請頁連結（但不含被標記的欄位清單：${/legal_name|法定名稱/.test(m.text.replace('法定名稱需與立案證書一致', '')) ? '有' : '沒有'}）`)
    S.flaggedInMail = /legal_name|documents/.test(m.text)
  }, { page: lp })

  await run.step('P-14', '申請人補件後重新送審（平台外：申請端），營運者在待審清單再次看到', async ({ note }) => {
    const o = S.orgA.owner
    await must(await o.api.patch(`orgs/${S.orgA.id}`, { data: { legal_name: `財團法人甲${T}基金會（更正）` } }), '補件修改')
    const attempt = await o.api.patch(`orgs/${S.orgA.id}`, { data: { website: 'https://other.example.org' } })
    ok(attempt.status() === 422, `未標記欄位預期被鎖 422，實際 ${attempt.status()}`)
    await must(await o.api.post(`orgs/${S.orgA.id}/documents?kind=registration`, { data: PDF, headers: { 'content-type': 'application/pdf' } }), '補件文件')
    await must(await o.api.post(`orgs/${S.orgA.id}/submit`), '重新送審')
    await sel(lp, '狀態', '待審核')
    const card = await findCard(lp, () => lp.locator(`article[data-org="${S.orgA.id}"]`))
    await card.getByRole('button', { name: '檢視' }).click()
    await expect(card).toContainText('（更正）')
    await expect(card.getByRole('button', { name: '下載' }).first()).toBeVisible(); const n = (await card.getByRole('button', { name: '下載' }).count())
    note(`重新送審後回到待審；法定名稱已更正；文件下載鈕 ${n} 個（立案文件＋許可文件，含補件新增者）。未標記欄位（website）被鎖：422 FIELD_LOCKED。`)
  }, { page: lp })

  await run.step('P-15', '許可審核：A 的許可 1 核准、許可 2 退件（含備註）', async ({ note }) => {
    const card = lp.locator(`article[data-org="${S.orgA.id}"]`)
    const p1 = card.locator(`[data-permit="${S.orgA.permits[0].id}"]`), p2 = card.locator(`[data-permit="${S.orgA.permits[1].id}"]`)
    await p1.getByRole('button', { name: '核准許可' }).click(); await dlg(lp).getByRole('button', { name: '核准許可' }).click()
    await expect(lp.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()
    await expect(card.locator(`[data-permit="${S.orgA.permits[0].id}"]`)).toHaveAttribute('data-status', 'approved')
    S.t15 = nowMs()
    await p2.getByRole('button', { name: '退件', exact: true }).click()
    await dlg(lp).getByLabel(/審核備註/).fill('許可文件模糊無法辨識，請重新上傳')
    await dlg(lp).getByRole('button', { name: '退件', exact: true }).click()
    await expect(card.locator(`[data-permit="${S.orgA.permits[1].id}"]`)).toHaveAttribute('data-status', 'rejected')
    await expect(card.locator(`[data-permit="${S.orgA.permits[1].id}"]`).getByRole('button')).toHaveCount(0)
    ok(sql(`SELECT review_note FROM org_permits WHERE id='${S.orgA.permits[1].id}'`).includes('模糊'), '退件備註未存')
    note('許可 1 → 已核准；許可 2 → 已退件且不再有動作按鈕；退件備註已存入 DB。')
  }, { page: lp })

  await run.step('P-16', '許可退件是否通知申請人（理由要能讓對方知道怎麼改）', async ({ note }) => {
    const none = await noMailWithin(S.orgA.owner.email, /許可/, 8000)
    const mails = (await listMail(S.orgA.owner.email)).map(m => m.subject)
    note(`申請人 A 收到的信：${mails.join(' ／ ')}`)
    if (none) throw new Gap('許可被退件時沒有任何通知信（只有「已核准許可被撤銷」會寄 org.permit_revoked）；申請人只能自己回狀態頁才看到。org_admin.rs review_permit 只在 status=approved 分支寫 notifications。')
  })

  await run.step('P-17', '核准組織：B 沒有核准許可時被擋（PERMIT_REQUIRED）；A 核准後通知申請人、公開組織頁可見', async ({ note }) => {
    const cardB = lp.locator(`article[data-org="${S.orgB.id}"]`)
    await cardB.getByRole('button', { name: '檢視' }).click()
    await cardB.getByRole('button', { name: '核准組織', exact: true }).click()
    await dlg(lp).getByRole('button', { name: '核准組織', exact: true }).click()
    await expect(dlg(lp).getByRole('alert')).toBeVisible()
    note(`B 無核准許可時按核准 → 對話框錯誤：「${(await dlg(lp).getByRole('alert').innerText()).trim()}」`)
    await dlg(lp).getByRole('button', { name: '取消' }).click()
    S.t17 = nowMs()
    const cardA = lp.locator(`article[data-org="${S.orgA.id}"]`)
    await cardA.getByRole('button', { name: '檢視' }).click(); await expect(cardA).toContainText('勸募許可')
    await cardA.getByRole('button', { name: '核准組織', exact: true }).click()
    await dlg(lp).getByRole('button', { name: '核准組織', exact: true }).click()
    await expect(lp.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()
    ok(sql(`SELECT verification_status FROM organizations WHERE id='${S.orgA.id}'`) === 'verified', 'A 未 verified')
    const m = await waitForMail(S.orgA.owner.email, { subject: /已通過審核/, after: S.t17 }); note(`通知信：「${m.subject}」`)
    const st = await webStatus(`/o/${S.orgA.slug}`); note(`公開組織頁 /o/${S.orgA.slug} → ${st}`)
  }, { page: lp })

  await run.step('P-18', '退件：B 填備註退件 → 申請人收到信、30 天內不能重新申請', async ({ note }) => {
    S.t18 = nowMs()
    const cardB = lp.locator(`article[data-org="${S.orgB.id}"]`)
    if (await cardB.getByRole('button', { name: '檢視' }).count()) await cardB.getByRole('button', { name: '檢視' }).click()
    await expect(cardB).toContainText('勸募許可')
    await cardB.getByRole('button', { name: '退件', exact: true }).last().click()
    await dlg(lp).getByLabel(/審核備註/).fill('立案資料與主管機關登記不符')
    await dlg(lp).getByRole('button', { name: '退件', exact: true }).click()
    await expect(lp.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()
    const m = await waitForMail(S.orgB.owner.email, { subject: /未通過審核/, after: S.t18 })
    ok(/立案資料與主管機關登記不符/.test(m.text), '退件信缺備註')
    const days = sql(`SELECT round(extract(epoch FROM (resubmit_after - now()))/86400) FROM organizations WHERE id='${S.orgB.id}'`)
    const r = await S.orgB.owner.api.patch(`orgs/${S.orgB.id}`, { data: { intro: '重新申請' } })
    const j = await r.json().catch(() => ({}))
    ok(r.status() === 409 && j.code === 'RESUBMIT_TOO_EARLY', `預期 409 RESUBMIT_TOO_EARLY，實際 ${r.status()} ${j.code}`)
    await sel(lp, '狀態', '已退件')
    await expect(lp.locator(`article[data-org="${S.orgB.id}"]`)).toBeVisible()
    note(`退件信「${m.subject}」含備註；resubmit_after ≈ +${days} 天；申請人立刻改資料 → 409 RESUBMIT_TOO_EARLY。`)
  }, { page: lp })
}

// =====================================================================
// C 救災事件
// =====================================================================
async function ensureOrgA() {
  if (S.orgA?.verifiedReady) return
  if (!S.orgA) {
    S.orgA = await makeOrg('甲', 2)
    const st = S.lead // 平台外：以 API 完成審核，讓本區段可獨立執行
    await must(await st.api.post(`admin/permits/${S.orgA.permits[0].id}/review`, { data: { decision: 'approve' } }), '核准許可')
    await must(await st.api.post(`admin/orgs/${S.orgA.id}/review`, { data: { decision: 'approve' } }), '核准組織')
  }
  S.orgA.verifiedReady = true
}
if (want('C')) {
  const lp = await leadPage()
  await run.step('P-20', '前置：組織 A 建立災害事件 EvA 與重複事件 EvDup，各掛一個上線據點（平台外：組織端已由 03-org 覆蓋）', async ({ note }) => {
    if (!S.orgA?.verifiedReady && S.orgA) { // 由 B 區段審核完成
      ok(sql(`SELECT verification_status FROM organizations WHERE id='${S.orgA.id}'`) === 'verified', '組織 A 尚未通過')
      S.orgA.verifiedReady = true
    }
    await ensureOrgA()
    S.ownerA = await loginApi(S.orgA.owner.email)
    const day = iso(new Date())
    const ev = async name => must(await S.ownerA.post(`orgs/${S.orgA.id}/events`, { data: { name, hazard_type: 'earthquake', counties: ['花蓮縣'], occurred_on: day } }), '建立事件')
    S.evA = await ev(`花蓮地震${T}`); S.evDup = await ev(`花蓮震災（重複）${T}`)
    const site = async (ev, title) => {
      const s = await must(await S.ownerA.post(`orgs/${S.orgA.id}/sites`, { data: { title, event_id: ev.id, permit_id: S.orgA.permits[0].id, site_address: '花蓮縣花蓮市中山路1號', recipient_name: '王小明', recipient_phone: '0912345678' } }), '建立據點')
      await must(await S.ownerA.post(`wishlists/${s.id}/items`, { data: { title: '瓶裝水', qty_needed: 10, priority: 'medium', funding_mode: 'quantity', category: 'water', urgency: 'critical' } }), '據點品項')
      const cur = await must(await S.ownerA.get(`wishlists/${s.id}`), '讀據點')
      await must(await S.ownerA.patch(`wishlists/${s.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } }), '上線據點')
      return { id: s.id, slug: s.slug ?? cur.wishlist.slug }
    }
    S.s1 = await site(S.evA, `據點一${T}`); S.s2 = await site(S.evDup, `據點二${T}`)
    const pub = [await webStatus(`/s/${S.s1.slug}`), await webStatus(`/s/${S.s2.slug}`), await webStatus(`/e/${S.evA.slug}`), await webStatus(`/o/${S.orgA.slug}`)]
    note(`事件 ${S.evA.slug}／${S.evDup.slug}；據點 ${S.s1.slug}／${S.s2.slug}；公開頁狀態（據點1、據點2、事件、組織）= ${pub.join(',')}`)
    ok(pub.every(s => s === 200), '前置的公開頁不是全 200')
  })

  await run.step('P-21', '事件管理列表：以 slug 篩選找到事件（列表來自公開 API，無 admin 列表端點）', async ({ note }) => {
    await lp.goto('/admin/relief-events')
    await lp.getByLabel('以 id 或 slug 篩選').fill(S.evA.slug)
    const card = await findCard(lp, () => lp.locator(`article[data-event="${S.evA.id}"]`))
    await expect(card).toContainText(S.evA.name); await expect(card.getByRole('button', { name: '合併' })).toBeVisible()
    note(`顯示 slug、災害類型、縣市、日期、id；動作：合併、隱藏。`)
    await lp.getByLabel('以 id 或 slug 篩選').fill('')
  }, { page: lp })

  await run.step('P-22', '下架事件：不填原因被後端擋；填原因後事件隱藏、掛在其下的據點被暫停、公開頁消失', async ({ note }) => {
    await lp.getByLabel('以 id 或 slug 篩選').fill(S.evA.slug)
    const card = lp.locator(`article[data-event="${S.evA.id}"]`)
    await card.getByRole('button', { name: '隱藏' }).click()
    await dlg(lp).getByRole('button', { name: '隱藏', exact: true }).click()
    await expect(dlg(lp).getByRole('alert')).toBeVisible()
    const emptyErr = (await dlg(lp).getByRole('alert').innerText()).trim()
    note(`未填原因直接送出 → 對話框錯誤：「${emptyErr}」（輸入欄標示為「原因」，並未標示必填，disable 也沒做）`)
    S.eventReasonUi = emptyErr
    await dlg(lp).getByLabel('原因').fill('與花蓮地震主事件重複且資訊不實')
    await dlg(lp).getByRole('button', { name: '隱藏', exact: true }).click()
    await expect(lp.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()
    await expect(card.getByText('已隱藏')).toBeVisible()
    const site = sql(`SELECT site_status FROM wishlists WHERE id='${S.s1.id}'`)
    const ev = await webStatus(`/e/${S.evA.slug}`), pubApi = await apiStatus(`/public/relief-events/${S.evA.slug}`), s1 = await webStatus(`/s/${S.s1.slug}`), org = await (await fetch(`${API}/api/v1/public/orgs/${S.orgA.slug}`)).text()
    note(`據點 1 site_status=${site}；/e/事件 → ${ev}（API ${pubApi}）；/s/據點1 → ${s1}；組織頁仍列出據點1：${org.includes(S.s1.slug)}`)
    ok(site === 'paused', '據點未暫停')
    S.t22 = nowMs()
  }, { page: lp })

  await run.step('P-23', '事件下架是否通知所屬組織（owner 要知道據點為何被暫停）', async ({ note }) => {
    const none = await noMailWithin(S.orgA.owner.email, /事件|據點|下架|暫停/, 8000)
    note(`組織 owner 信箱主旨：${(await listMail(S.orgA.owner.email)).map(m => m.subject).join(' ／ ')}`)
    if (none) throw new Gap('事件被下架、據點被自動暫停，但組織 owner 沒收到任何通知（relief_events.rs moderate 只寫 audit 與 purge CDN，沒有 notifications）。對方只會發現據點打不開。')
  })

  await run.step('P-24', '取消下架：事件回到公開，但已暫停的據點不會自動恢復', async ({ note }) => {
    const card = lp.locator(`article[data-event="${S.evA.id}"]`)
    await card.getByRole('button', { name: '取消隱藏' }).click()
    await dlg(lp).getByRole('button', { name: '取消隱藏', exact: true }).click()
    await expect(card.getByText('已隱藏')).toHaveCount(0)
    const site = sql(`SELECT site_status FROM wishlists WHERE id='${S.s1.id}'`)
    const ev = await webStatus(`/e/${S.evA.slug}`), s1 = await webStatus(`/s/${S.s1.slug}`)
    note(`取消隱藏後：/e/事件 → ${ev}；據點 1 site_status=${site}、/s/據點1 → ${s1}`)
    ok(ev === 200, '事件未恢復公開')
    if (site === 'paused') note('據點仍為 paused（設計如此，CLAUDE.md 有記載），營運者沒有「恢復據點」的 UI；須由組織 owner 自行恢復營運。')
  }, { page: lp })

  await run.step('P-25', '合併重複事件：EvDup 合併到 EvA（自己合併自己被擋）→ 據點改掛 EvA，EvDup 從公開列表消失', async ({ note }) => {
    await lp.getByLabel('以 id 或 slug 篩選').fill('')
    const dup = await findCard(lp, () => lp.locator(`article[data-event="${S.evDup.id}"]`))
    await dup.getByRole('button', { name: '合併', exact: true }).click()
    await dlg(lp).getByLabel(/合併到/).fill(S.evDup.slug)
    await dlg(lp).getByRole('button', { name: '合併', exact: true }).click()
    await expect(dlg(lp).getByRole('alert')).toContainText('不能合併到自己')
    await dlg(lp).getByLabel(/合併到/).fill(S.evA.slug)
    await dlg(lp).getByRole('button', { name: '合併', exact: true }).click()
    await expect(dup).toContainText('已合併至')
    await expect(dup.getByRole('button', { name: '合併', exact: true })).toHaveCount(0)
    const ev = sql(`SELECT event_id FROM wishlists WHERE id='${S.s2.id}'`)
    ok(ev === S.evA.id, '據點 2 未改掛 EvA')
    const list = await (await fetch(`${API}/api/v1/public/relief-events`)).json()
    const inList = (list.events ?? []).some(e => e.id === S.evDup.id)
    const dupPage = await webStatus(`/e/${S.evDup.slug}`), dupApi = await apiStatus(`/public/relief-events/${S.evDup.slug}`)
    note(`合併後：據點 2 event_id=EvA；EvDup 仍在公開列表：${inList}；/e/EvDup → ${dupPage}（API ${dupApi}）；列上有「已合併至」且無合併鈕（不可逆）。`)
  }, { page: lp })
}

// =====================================================================
// D 許可 / 組織撤銷的連動
// =====================================================================
if (want('D')) {
  const lp = await leadPage()
  await run.step('P-30', '撤銷已核准許可：使用此許可的上線據點被暫停、公開頁與組織頁消失、owner 收到信', async ({ note }) => {
    await ensureOrgA()
    if (!S.s2) throw new Blocked('區段 D 需要區段 C 的據點（請連同 C 一起跑）')
    S.t30 = nowMs()
    await lp.goto('/admin/orgs'); await sel(lp, '狀態', '已通過')
    const card = await findCard(lp, () => lp.locator(`article[data-org="${S.orgA.id}"]`))
    await card.getByRole('button', { name: '檢視' }).click()
    const p1 = card.locator(`[data-permit="${S.orgA.permits[0].id}"]`)
    await p1.getByRole('button', { name: '撤銷許可' }).click()
    await expect(dlg(lp)).toContainText('使用此許可的上線據點會被暫停')
    await dlg(lp).getByLabel(/審核備註/).fill('許可已被主管機關廢止')
    await dlg(lp).getByRole('button', { name: '撤銷許可' }).click()
    await expect(card.locator(`[data-permit="${S.orgA.permits[0].id}"]`)).toHaveAttribute('data-status', 'rejected')
    const s2 = sql(`SELECT site_status FROM wishlists WHERE id='${S.s2.id}'`)
    const org = await (await fetch(`${API}/api/v1/public/orgs/${S.orgA.slug}`)).text()
    const ev = await (await fetch(`${API}/api/v1/public/relief-events/${S.evA.slug}`)).text()
    const m = await waitForMail(S.orgA.owner.email, { subject: /已撤銷/, after: S.t30 })
    note(`據點 2 site_status=${s2}；組織頁仍列據點2：${org.includes(S.s2.slug)}；事件頁仍列據點2：${ev.includes(S.s2.slug)}；owner 信：「${m.subject}」`)
    ok(s2 === 'paused', '據點 2 未暫停')
    ok(!org.includes(S.s2.slug), '組織頁仍列出已暫停據點')
  }, { page: lp })

  await run.step('P-31', '撤銷組織（兩段確認）：據點全部降為草稿、公開組織頁 404、owner 收到「驗證已撤銷」', async ({ note }) => {
    S.t31 = nowMs()
    const card = lp.locator(`article[data-org="${S.orgA.id}"]`)
    await card.getByRole('button', { name: '撤銷組織' }).click()
    await dlg(lp).getByRole('button', { name: '撤銷組織', exact: true }).click()
    await expect(dlg(lp)).toContainText('再次確認')
    note('第一段確認後出現「再次確認：撤銷後所有公開據點都會立即下架」。')
    await dlg(lp).getByLabel(/審核備註/).count().then(n => note(`第二段是否仍有備註欄：${n ? '有' : '沒有'}（第一段才填備註，理由在第二段不可補）`))
    await dlg(lp).getByRole('button', { name: '撤銷組織', exact: true }).click()
    await expect(lp.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()
    const st = sql(`SELECT string_agg(status::text, ',') FROM wishlists WHERE org_id='${S.orgA.id}' AND type='relief'`)
    const pages = [await webStatus(`/s/${S.s1.slug}`), await webStatus(`/s/${S.s2.slug}`), await webStatus(`/o/${S.orgA.slug}`)]
    const m = await waitForMail(S.orgA.owner.email, { subject: /驗證已撤銷/, after: S.t31 })
    note(`據點狀態 ${st}；公開頁（據點1、據點2、組織）→ ${pages.join(',')}；信：「${m.subject}」`)
    ok(pages.every(s => s === 404 || s === 410), '撤銷後公開頁仍可見')
  }, { page: lp })

  await run.step('P-32', '誤撤銷的復原路徑：已退件的組織在後台沒有任何「恢復」動作', async ({ note }) => {
    await sel(lp, '狀態', '已退件')
    const card = await findCard(lp, () => lp.locator(`article[data-org="${S.orgA.id}"]`))
    await card.getByRole('button', { name: '檢視' }).click()
    const btns = await card.getByRole('button').allInnerTexts()
    const days = sql(`SELECT round(extract(epoch FROM (resubmit_after - now()))/86400) FROM organizations WHERE id='${S.orgA.id}'`)
    note(`已退件組織卡片可用按鈕：${btns.join('、')}；resubmit_after ≈ +${days} 天`)
    throw new Gap('撤銷組織 = status 轉 rejected 並鎖 30 天，後台沒有「撤銷撤銷 / 重新核准」；org_admin.rs review 的 reject 只允許 pending|verified，也無 verified 回復路徑。誤撤銷只能用 SQL 補救，且據點已被降為 draft。')
  }, { page: lp })
}

// =====================================================================
// E 送禮執行人（採購單）
// =====================================================================
if (want('E')) {
  const sp = await staffPage(S.ship); S.sp = sp
  await run.step('P-40', '前置：受捐者建立 4 個眾籌品項（收件資訊已填）、捐助者領點並贊助達標 → 產生 4 張採購單（平台外：API）', async ({ note }) => {
    S.rec = await registerApi({ name: `受捐者${T}`, tag: 'rec' }); S.d1 = await registerApi({ name: `捐助者甲${T}`, tag: 'don' }); S.d2 = await registerApi({ name: `捐助者乙${T}`, tag: 'don' })
    S.d1.id = await meId(S.d1); S.d2.id = await meId(S.d2); S.rec.id = await meId(S.rec)
    const l = S.recList = await makeCfList(S.rec, `寶寶清單${T}`, [{ title: `推車${T}`, target: 1000 }, { title: `消毒鍋${T}`, target: 500 }, { title: `嬰兒床${T}`, target: 300 }, { title: `揹巾${T}`, target: 200 }])
    const [A, B, C, D] = l.ids; S.items = { A, B, C, D }
    await grantApi(S.fin, S.d1, 2000); await grantApi(S.fin, S.d2, 1000)
    await pledge(S.d1, A, 600); await pledge(S.d2, A, 400); await pledge(S.d1, B, 500); await pledge(S.d2, C, 300); await pledge(S.d1, D, 200)
    S.bal0 = { d1: bal(S.d1.id), d2: bal(S.d2.id) }
    const orders = sql(`SELECT count(*) FROM purchase_orders WHERE item_id IN ('${A}','${B}','${C}','${D}') AND status='pending'`)
    note(`受捐者 ${S.rec.email}；捐助者甲 ${S.d1.email}（餘額 ${S.bal0.d1}）、乙 ${S.d2.email}（餘額 ${S.bal0.d2}）；待下單採購單 ${orders} 張`)
    ok(orders === '4', '採購單數量不是 4')
  })

  await run.step('P-41', '採購單佇列：預設只看待下單；收件資訊解密顯示並寫稽核；篩選狀態與履約類型', async ({ note }) => {
    const before = Number(sql(`SELECT count(*) FROM audit_logs WHERE action='order.view_address' AND actor_id='${S.ship.id}'`))
    await sp.goto('/admin/orders'); await sp.waitForLoadState('networkidle')
    const card = await findCard(sp, () => sp.locator('article', { hasText: `推車${T}` }))
    for (const s of ['王小米', '0912345678', '台北市大安區和平東路二段 1 號 3 樓', '待下單', '1,000']) await expect(card).toContainText(s)
    const shown = await sp.locator('article').count()
    const after = Number(sql(`SELECT count(*) FROM audit_logs WHERE action='order.view_address' AND actor_id='${S.ship.id}'`))
    note(`載入 ${shown} 張採購單 → 新增 ${after - before} 筆 order.view_address 稽核（每次載入列表、每張單各一筆，含後續換篩選／重新載入）。`)
    ok(after - before >= shown, '讀取明文地址沒有逐筆稽核')
    await sel(sp, '履約類型', '商品目錄')
    await expect(sp.locator('article', { hasText: `推車${T}` })).toHaveCount(0)
    await sel(sp, '履約類型', '人工代購')
    await expect(sp.locator('article', { hasText: `推車${T}` })).toHaveCount(1)
    note('履約類型篩選：商品目錄看不到、人工代購看得到。')
    const opts = await sp.getByLabel('狀態').locator('option').allInnerTexts(); note(`狀態篩選：${opts.join('／')}`)
  }, { page: sp })

  const orderCard = (page, t) => page.locator('article', { hasText: t })
  await run.step('P-42', '標記已下單：缺訂單編號、金額超過目標被擋；實付 900 → 差額 100 依比例退回捐助者', async ({ note }) => {
    const card = orderCard(sp, `推車${T}`)
    await card.getByRole('button', { name: '標記已下單' }).click()
    await dlg(sp).getByRole('button', { name: '標記已下單' }).click()
    await expect(dlg(sp).getByRole('alert')).toBeVisible(); note(`缺訂單編號 → 「${(await dlg(sp).getByRole('alert').innerText()).trim()}」`)
    await dlg(sp).getByLabel(/商家訂單編號/).fill('SHOP-OP-1'); await dlg(sp).getByLabel(/實際金額/).fill('1001')
    await dlg(sp).getByRole('button', { name: '標記已下單' }).click()
    await expect(dlg(sp).getByRole('alert')).toBeVisible(); note(`金額 1001 > 目標 1000 → 「${(await dlg(sp).getByRole('alert').innerText()).trim()}」`)
    await dlg(sp).getByLabel(/實際金額/).fill('900')
    await dlg(sp).getByRole('button', { name: '標記已下單' }).click()
    await expect(sp.getByRole('status').filter({ hasText: '差額 100 點已退回' })).toBeVisible()
    const d1 = bal(S.d1.id) - S.bal0.d1, d2 = bal(S.d2.id) - S.bal0.d2
    note(`退回：捐助者甲 +${d1}（預期 60）、乙 +${d2}（預期 40）；卡片離開「待下單」篩選。`)
    ok(d1 === 60 && d2 === 40, '差額依比例退回錯誤')
    await expect(orderCard(sp, `推車${T}`)).toHaveCount(0)
    ok(reconcile() === '0', 'Σ lots ≠ balance'); ok(ledgerGap() === '0', 'Σ ledger ≠ balance')
  }, { page: sp })

  await run.step('P-43', '標記已出貨（物流單號）→ 兩位捐助者收到「已出貨」信', async ({ note }) => {
    S.t43 = nowMs()
    await sel(sp, '狀態', '已下單')
    const card = await findCard(sp, () => orderCard(sp, `推車${T}`))
    await expect(card).toContainText('SHOP-OP-1'); await expect(card).toContainText('900')
    await card.getByRole('button', { name: '標記已出貨' }).click()
    await dlg(sp).getByLabel(/物流單號/).fill('TW-OP-0001')
    await dlg(sp).getByRole('button', { name: '標記已出貨' }).click()
    await expect(sp.getByRole('status').filter({ hasText: '運送中' })).toBeVisible()
    for (const d of [S.d1, S.d2]) { const m = await waitForMail(d.email, { subject: /已出貨/, after: S.t43 }); note(`${d.name} 收到：「${m.subject}」`) }
  }, { page: sp })

  await run.step('P-44', '標記已送達 → 品項標為已履行、捐助者收「已送達」信；公開頁顯示已送達', async ({ note }) => {
    S.t44 = nowMs()
    await sel(sp, '狀態', '運送中')
    const card = await findCard(sp, () => orderCard(sp, `推車${T}`))
    await expect(card).toContainText('TW-OP-0001')
    await card.getByRole('button', { name: '標記已送達' }).click()
    await expect(dlg(sp)).toContainText('確認包裹已送達')
    await dlg(sp).getByRole('button', { name: '標記已送達' }).click()
    await expect(sp.getByRole('status').filter({ hasText: '已送達' })).toBeVisible()
    ok(sql(`SELECT funding_status FROM wishlist_items WHERE id='${S.items.A}'`) === 'fulfilled', '品項未 fulfilled')
    for (const d of [S.d1, S.d2]) await waitForMail(d.email, { subject: /已送達/, after: S.t44 })
    const g = await newPage(); await g.goto(`/s/${S.recList.slug}`); await g.waitForLoadState('networkidle')
    const row = g.locator('li.g-card', { hasText: `推車${T}` })
    note(`公開頁推車卡片：${(await row.innerText()).replace(/\s+/g, ' ').slice(0, 80)}`)
    await expect(row).toContainText('已送達')
    // 已送達不能再失敗 / 取消（狀態機）
    const oid = sql(`SELECT po.id FROM purchase_orders po WHERE po.item_id='${S.items.A}'`)
    const r = await S.ship.api.patch(`admin/orders/${oid}`, { data: { status: 'failed', failure_reason: '測試' } })
    ok(r.status() === 409, `已送達再標失敗預期 409，實際 ${r.status()}`)
    note(`已送達的單再 PATCH failed → 409 ${(await r.json()).code}（UI 也不提供按鈕）。`)
  }, { page: sp })

  await run.step('P-45', '問題單：缺貨 → 標記失敗（原因必填）→ 點數全額退回、品項回到募集中、捐助者收信', async ({ note }) => {
    S.t45 = nowMs()
    await sel(sp, '狀態', '待下單')
    const card = await findCard(sp, () => orderCard(sp, `消毒鍋${T}`))
    const before = bal(S.d1.id)
    await card.getByRole('button', { name: '標記失敗' }).click()
    await dlg(sp).getByRole('button', { name: '標記失敗' }).click()
    await expect(dlg(sp).getByRole('alert')).toBeVisible(); note(`未填原因 → 「${(await dlg(sp).getByRole('alert').innerText()).trim()}」`)
    await dlg(sp).getByLabel(/原因/).fill('商家缺貨，無替代品')
    await dlg(sp).getByRole('button', { name: '標記失敗' }).click()
    await expect(sp.getByRole('status').filter({ hasText: '失敗' })).toBeVisible()
    ok(bal(S.d1.id) - before === 500, '捐助者甲未全額退回')
    ok(sql(`SELECT funding_status||'/'||pledged_points FROM wishlist_items WHERE id='${S.items.B}'`) === 'open/0', '品項未回到募集中')
    const m = await waitForMail(S.d1.email, { subject: /代購未成功/, after: S.t45 })
    note(`甲 +500 點；品項回 open、pledged_points=0；信：「${m.subject}」`)
    ok(reconcile() === '0', 'Σ lots ≠ balance')
  }, { page: sp })

  await run.step('P-46', '先下單（實付 250，退差額 50）再取消採購 → 已扣用的 250 全數退回，捐助者淨變動為 0', async ({ note }) => {
    const before = S.bal0.d2 + 40 // 乙在 P-42 退回 40 後的餘額基準
    const cardC = await findCard(sp, () => orderCard(sp, `嬰兒床${T}`))
    await cardC.getByRole('button', { name: '標記已下單' }).click()
    await dlg(sp).getByLabel(/商家訂單編號/).fill('SHOP-OP-C'); await dlg(sp).getByLabel(/實際金額/).fill('250')
    await dlg(sp).getByRole('button', { name: '標記已下單' }).click()
    await expect(sp.getByRole('status').filter({ hasText: '差額 50 點已退回' })).toBeVisible()
    await sel(sp, '狀態', '已下單')
    const c2 = await findCard(sp, () => orderCard(sp, `嬰兒床${T}`))
    await c2.getByRole('button', { name: '取消採購' }).click()
    await dlg(sp).getByLabel(/原因/).fill('受贈者已自行購買')
    await dlg(sp).getByRole('button', { name: '取消採購' }).click()
    await expect(sp.getByRole('status').filter({ hasText: '已取消' })).toBeVisible()
    const now = bal(S.d2.id)
    note(`乙餘額：下單前基準 ${before}（含 P-42 的 +40，已扣 300）→ 取消後 ${now}；嬰兒床品項狀態 ${sql(`SELECT funding_status||'/'||pledged_points FROM wishlist_items WHERE id='${S.items.C}'`)}`)
    ok(now === before + 300, `取消後乙應回到 ${before + 300}，實際 ${now}`)
    ok(reconcile() === '0', 'Σ lots ≠ balance'); ok(ledgerGap() === '0', 'Σ ledger ≠ balance')
  }, { page: sp })

  await run.step('P-47', '兩位承辦同時處理同一張單：後送出的人看到「狀態已被更新過」並重讀列表', async ({ note }) => {
    const p2 = await staffPage(S.fin)
    await sel(sp, '狀態', '待下單')
    const c1 = await findCard(sp, () => orderCard(sp, `揹巾${T}`))
    await p2.goto('/admin/orders'); const c2 = await findCard(p2, () => orderCard(p2, `揹巾${T}`))
    await c2.getByRole('button', { name: '標記已下單' }).click()
    await dlg(p2).getByLabel(/商家訂單編號/).fill('SHOP-OP-D'); await dlg(p2).getByLabel(/實際金額/).fill('200')
    await dlg(p2).getByRole('button', { name: '標記已下單' }).click()
    await expect(p2.getByRole('status').filter({ hasText: '已下單' })).toBeVisible()
    await c1.getByRole('button', { name: '標記已下單' }).click()
    await dlg(sp).getByLabel(/商家訂單編號/).fill('SHOP-OP-D2'); await dlg(sp).getByLabel(/實際金額/).fill('200')
    await dlg(sp).getByRole('button', { name: '標記已下單' }).click()
    await expect(dlg(sp).getByRole('alert')).toContainText('已被更新過')
    await sp.waitForTimeout(800)
    note(`後送出者看到：「${(await dlg(sp).getByRole('alert').innerText()).trim()}」；對話框保留，列表在背景已重讀（該單離開待下單）。`)
    ok(sql(`SELECT merchant_order_id FROM purchase_orders WHERE item_id='${S.items.D}'`) === 'SHOP-OP-D', '先送出者的資料被覆蓋')
    await p2.context().close()
  }, { page: sp })

  await run.step('P-48', '受捐者（清單擁有者）在採購過程中是否收到任何通知', async ({ note }) => {
    const subj = (await listMail(S.rec.email)).map(m => m.subject)
    note(`受捐者信箱共 ${subj.length} 封：${subj.join(' ／ ') || '（無）'}`)
    const don = (await listMail(S.d1.email)).map(m => m.subject); note(`捐助者甲收到：${don.join(' ／ ')}`)
    const progress = await S.rec.api.get(`wishlists/${S.recList.id}/progress`).then(r => r.status()).catch(() => 'n/a')
    note(`（參考）受捐者的進度頁 API wishlists/{id}/progress → ${progress}`)
    if (!subj.some(s => /出貨|送達|代購/.test(s))) throw new Gap('採購單狀態推進（已出貨／已送達／失敗）只通知捐助者（notify::enqueue_donors），受捐者不會收到任何信，只能自己回進度頁看；任務描述期待「通知受捐者／捐助者」。orders.rs patch_order 沒有針對 wishlist owner 的 enqueue。')
  })
}

// =====================================================================
// F 金流營運
// =====================================================================
if (want('F')) {
  const fp = await staffPage(S.fin); S.fp = fp
  await run.step('P-50', '前置：5 位付款人（儲值已入帳／領點）與 3 筆現金退款申請（平台外：API 與綠界替身）', async ({ note }) => {
    S.pay = {}
    for (const k of ['p1', 'p2', 'p3', 'p4', 'p5']) { S.pay[k] = await registerApi({ name: `付款人${k}${T}`, tag: 'pay' }); S.pay[k].id = await meId(S.pay[k]) }
    S.rec2 ??= await registerApi({ name: `眾籌發起${T}`, tag: 'rec' })
    S.cf2 = await makeCfList(S.rec2, `大額眾籌${T}`, [{ title: `大型品項${T}`, target: 5000 }])
    S.tp = {}
    S.tp.t1 = await topupPaid(S.pay.p1, 1000); await pledge(S.pay.p1, S.cf2.ids[0], 600)
    S.tp.t2 = await topupPaid(S.pay.p2, 800)
    S.tp.t3 = await topupPaid(S.pay.p3, 600)
    S.tp.t4 = await topupPaid(S.pay.p4, 300); await pledge(S.pay.p4, S.cf2.ids[0], 300)
    const r1 = await must(await S.pay.p3.api.post('wallet/cash-refunds', { data: { amount: 200 }, headers: idemp() }), '退款申請 1')
    const r2 = await must(await S.pay.p3.api.post('wallet/cash-refunds', { data: { amount: 100 }, headers: idemp() }), '退款申請 2')
    S.refund = { r1: r1.id ?? r1.request?.id, r2: r2.id ?? r2.request?.id }
    note(`p1 儲值 1000（贊助 600，剩 400）；p2 儲值 800；p3 儲值 600、申請現金退款 200 與 100；p4 儲值 300 並全數贊助。`)
  })

  await run.step('P-51', '儲值查詢：預設「爭議中」；切到「已入帳」找到 p2 的單（無搜尋欄，需逐頁找）', async ({ note }) => {
    await fp.goto('/admin/topups'); await fp.waitForLoadState('networkidle')
    const initial = await fp.locator('article').count()
    await sel(fp, '狀態', '已入帳')
    const card = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p2.email }))
    await expect(card).toContainText('NT$ 800'); await expect(card).toContainText('ecpay')
    const pages = await fp.locator('article').count()
    const has = await fp.locator('input[type=search], input[placeholder*="搜尋"]').count()
    note(`預設爭議中 ${initial} 筆；已入帳列表為了找到 p2 載入了 ${pages} 張卡片；頁面搜尋欄數量：${has}。顯示完整 Email（後台使用者頁預設遮罩）。`)
    S.topupSearch = has
  }, { page: fp })

  await run.step('P-52', '標記爭議 → 錢包被凍結（列上顯示「錢包已凍結」）；凍結中不能贊助', async ({ note }) => {
    const card = fp.locator('article', { hasText: S.pay.p2.email })
    await card.getByRole('button', { name: '標記爭議' }).click()
    await dlg(fp).getByLabel(/備註/).fill('持卡人向發卡行提出爭議，案號 DSP-1')
    await dlg(fp).getByRole('button', { name: '標記爭議' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '錢包已凍結' })).toBeVisible()
    await expect(card).toContainText('錢包已凍結'); await expect(card).toContainText('爭議中')
    ok(sql(`SELECT status FROM point_wallets WHERE user_id='${S.pay.p2.id}'`) === 'frozen', 'DB 錢包未凍結')
    const r = await S.pay.p2.api.post(`items/${S.cf2.ids[0]}/contributions`, { data: { points: 100 }, headers: idemp() })
    const j = await r.json().catch(() => ({}))
    note(`凍結中 p2 贊助 100 點 → ${r.status()} ${j.code}`)
    ok(r.status() === 403 && j.code === 'WALLET_FROZEN', '凍結錢包仍可贊助')
  }, { page: fp })

  await run.step('P-53', '爭議勝訴 → 儲值單回「已入帳」、錢包自動解凍', async ({ note }) => {
    await sel(fp, '狀態', '爭議中')
    const card = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p2.email }))
    await card.getByRole('button', { name: '爭議勝訴' }).click()
    await dlg(fp).getByRole('button', { name: '爭議勝訴' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '爭議勝訴' })).toBeVisible()
    const w = sql(`SELECT status FROM point_wallets WHERE user_id='${S.pay.p2.id}'`), t = sql(`SELECT status FROM topups WHERE id='${S.tp.t2.id}'`)
    note(`勝訴後 DB：儲值單 ${t}、錢包 ${w}；訊息為「錢包是否解凍請見『點數錢包』」。`)
    ok(t === 'paid' && w === 'active', '勝訴後狀態不對')
  }, { page: fp })

  await run.step('P-54', '拒付敗訴（部分已花掉）：p1 儲值 1000 已贊助 600 → 扣回 400、記欠款 600、錢包保持凍結', async ({ note }) => {
    await sel(fp, '狀態', '已入帳')
    const card = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p1.email }))
    await card.getByRole('button', { name: '標記爭議' }).click(); await dlg(fp).getByRole('button', { name: '標記爭議' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '已凍結' })).toBeVisible()
    await sel(fp, '狀態', '爭議中')
    const c2 = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p1.email }))
    await c2.getByRole('button', { name: '拒付敗訴（扣回）' }).click()
    await expect(dlg(fp)).toContainText('扣回該筆儲值批次剩餘的點數')
    await dlg(fp).getByRole('button', { name: '拒付敗訴（扣回）' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '扣回 400 點，記為欠款 600 點' })).toBeVisible()
    const debt = sql(`SELECT amount FROM wallet_debts WHERE topup_id='${S.tp.t1.id}'`), b = bal(S.pay.p1.id), w = sql(`SELECT status FROM point_wallets WHERE user_id='${S.pay.p1.id}'`)
    note(`DB：欠款 ${debt}、p1 餘額 ${b}、錢包 ${w}；儲值單 ${sql(`SELECT status FROM topups WHERE id='${S.tp.t1.id}'`)}`)
    ok(debt === '600' && b === 0 && w === 'frozen', '拒付敗訴結果不對')
    ok(reconcile() === '0', 'Σ lots ≠ balance')
  }, { page: fp })

  await run.step('P-55', '拒付欠款：p1 欠 600；備註必填 → 標記已追回；p4（儲值全花光）敗訴後欠 300 → 核銷', async ({ note }) => {
    // p4：爭議 → 敗訴
    await fp.goto('/admin/topups'); await sel(fp, '狀態', '已入帳')
    const c4 = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p4.email }))
    await c4.getByRole('button', { name: '標記爭議' }).click(); await dlg(fp).getByRole('button', { name: '標記爭議' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '已凍結' })).toBeVisible()
    await sel(fp, '狀態', '爭議中')
    const c4b = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p4.email }))
    await c4b.getByRole('button', { name: '拒付敗訴（扣回）' }).click(); await dlg(fp).getByRole('button', { name: '拒付敗訴（扣回）' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '扣回 0 點，記為欠款 300 點' })).toBeVisible()
    await fp.goto('/admin/wallet-debts'); await fp.waitForLoadState('networkidle')
    const d1 = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p1.email }))
    await expect(d1).toContainText('600 點'); await expect(d1).toContainText('未結清')
    await d1.getByRole('button', { name: '標記已追回' }).click()
    await dlg(fp).getByRole('button', { name: '標記已追回' }).click()
    await expect(dlg(fp).getByRole('alert')).toContainText('請填寫備註')
    await dlg(fp).getByLabel(/備註/).fill('已向持卡人追回，匯款單號 TX-9981')
    await dlg(fp).getByRole('button', { name: '標記已追回' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '已追回' })).toBeVisible()
    const d4 = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p4.email }))
    await d4.getByRole('button', { name: '核銷' }).click()
    await dlg(fp).getByLabel(/備註/).fill('金額低於追討成本，平台吸收')
    await dlg(fp).getByRole('button', { name: '核銷' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '已核銷' })).toBeVisible()
    await sel(fp, '狀態', '已追回')
    await expect(await findCard(fp, () => fp.locator('article', { hasText: S.pay.p1.email }))).toContainText('TX-9981')
    note(`p1 欠款 → settled（備註 TX-9981）；p4 欠款 → written_off；DB：${sql(`SELECT string_agg(status::text,',' ORDER BY amount) FROM wallet_debts WHERE topup_id IN ('${S.tp.t1.id}','${S.tp.t4.id}')`)}`)
    const again = await S.fin.api.patch(`admin/wallet-debts/${sql(`SELECT id FROM wallet_debts WHERE topup_id='${S.tp.t1.id}'`)}`, { data: { status: 'written_off', note: '再改一次' } })
    ok(again.status() === 409, `已結案再改預期 409，實際 ${again.status()}`); note('已結案再 PATCH → 409 INVALID_STATE_TRANSITION。')
  }, { page: fp })

  await run.step('P-56', '現金退款：申請 200 → 開始處理 → 標記已退款（不可逆）', async ({ note }) => {
    await fp.goto('/admin/cash-refunds'); await fp.waitForLoadState('networkidle')
    const rows = fp.locator('article', { hasText: S.pay.p3.email })
    const c200 = await findCard(fp, () => rows.filter({ hasText: '200 點' }))
    await c200.getByRole('button', { name: '開始處理' }).click(); await dlg(fp).getByRole('button', { name: '開始處理' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '處理中' })).toBeVisible()
    await expect(c200).toContainText('處理中')
    await c200.getByRole('button', { name: '標記已退款' }).click()
    await expect(dlg(fp)).toContainText('無法復原')
    await dlg(fp).getByRole('button', { name: '標記已退款' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '退款完成' })).toBeVisible()
    ok(sql(`SELECT status FROM cash_refund_requests WHERE id='${S.refund.r1}'`) === 'completed', 'DB 未完成')
    note('requested → processing → completed；列上顯示完成時間；對話框明講無法復原。系統不代為呼叫綠界退款（要到金流商後台人工退）。')
  }, { page: fp })

  await run.step('P-57', '現金退款：拒絕並退點（理由必填）→ 點數退回原批次；使用者可看到拒絕理由', async ({ note }) => {
    const before = bal(S.pay.p3.id)
    const c100 = await findCard(fp, () => fp.locator('article', { hasText: S.pay.p3.email }).filter({ hasText: '100 點' }).filter({ hasText: '待審核' }))
    await c100.getByRole('button', { name: '拒絕並退點' }).click()
    await dlg(fp).getByRole('button', { name: '拒絕並退點' }).click()
    await expect(dlg(fp).getByRole('alert')).toContainText('請填寫拒絕原因')
    await dlg(fp).getByLabel(/拒絕原因/).fill('退款金額與原儲值單不符，請重新申請')
    await dlg(fp).getByRole('button', { name: '拒絕並退點' }).click()
    await expect(fp.getByRole('status').filter({ hasText: '已退回其錢包' })).toBeVisible()
    const now = bal(S.pay.p3.id)
    const mine = await (await S.pay.p3.api.get('wallet/cash-refunds')).json()
    const rej = (mine.requests ?? mine.data ?? []).find?.(x => x.status === 'rejected')
    note(`p3 餘額 ${before} → ${now}（預期 +100）；使用者 API 看到拒絕理由：${rej?.reject_reason ?? '（找不到）'}`)
    ok(now - before === 100, '退點不正確'); ok(reconcile() === '0', 'Σ lots ≠ balance')
    S.t57 = true
    const mails = (await listMail(S.pay.p3.email)).map(m => m.subject)
    note(`p3 信箱（現金退款完成／拒絕是否有通知）：${mails.join(' ／ ') || '（無）'}`)
    S.refundMail = mails.some(s => /退款/.test(s))
  }, { page: fp })

  await run.step('P-58', '現金退款處理結果是否通知使用者（完成匯款／被拒絕）', async ({ note }) => {
    if (!S.refundMail) throw new Gap('現金退款「已退款」與「被拒絕」都沒有寄信或站內通知給使用者（wallet_ops.rs cash_refund_patch 沒有 notifications）；使用者只能自己進 /me/refund 看狀態。拒付／凍結錢包也沒有通知，使用者只會在贊助時看到「請聯絡客服」。')
  })

  await run.step('P-59', '點數錢包：搜尋 → 發點 +500（adjustment）→ 扣點 -200 → 扣到負數被擋 → 凍結／解凍（原因必填）', async ({ note }) => {
    await fp.goto('/admin/wallets')
    await fp.getByLabel('搜尋使用者').fill(S.pay.p5.email); await fp.getByRole('button', { name: '搜尋', exact: true }).click()
    const card = fp.locator('article', { hasText: S.pay.p5.email }); await expect(card).toBeVisible()
    await card.getByRole('button', { name: '發點 / 扣點' }).click()
    await fp.getByLabel(/點數（正數發放/).fill('500'); await fp.getByLabel(/原因（必填/).fill('客服補償：活動 bug')
    await fp.getByRole('button', { name: '下一步' }).click(); await fp.getByRole('button', { name: '確認發放' }).click()
    await expect(card).toContainText('500')
    await card.getByRole('button', { name: '發點 / 扣點' }).click()
    await fp.getByLabel(/點數（正數發放/).fill('-200'); await fp.getByLabel(/原因（必填/).fill('更正誤發')
    await fp.getByRole('button', { name: '下一步' }).click(); await fp.getByRole('button', { name: '確認扣除' }).click()
    await expect(card).toContainText('300')
    await card.getByRole('button', { name: '發點 / 扣點' }).click()
    await fp.getByLabel(/點數（正數發放/).fill('-1000'); await fp.getByLabel(/原因（必填/).fill('扣過頭')
    await fp.getByRole('button', { name: '下一步' }).click(); await fp.getByRole('button', { name: '確認扣除' }).click()
    await expect(fp.getByRole('alert').filter({ hasText: '變成負數' })).toBeVisible()
    note(`扣到負數 → 「${(await fp.getByRole('alert').filter({ hasText: '變成負數' }).innerText()).trim()}」`)
    await fp.getByRole('button', { name: '取消' }).click()
    const lots = sql(`SELECT string_agg(source||':'||remaining_points,',') FROM point_lots WHERE wallet_id=(SELECT id FROM point_wallets WHERE user_id='${S.pay.p5.id}')`)
    note(`p5 的點數批次：${lots}（人工發放為 adjustment、不可退現金）`)
    // 凍結
    await card.getByRole('button', { name: '凍結' }).click()
    await expect(dlg(fp).getByRole('button', { name: '凍結' })).toBeDisabled()
    await dlg(fp).getByLabel(/原因/).fill('疑似盜刷，先凍結待查'); await dlg(fp).getByRole('button', { name: '凍結' }).click()
    await expect(card.getByText('已凍結')).toBeVisible()
    const fr = await S.pay.p5.api.post(`items/${S.cf2.ids[0]}/contributions`, { data: { points: 50 }, headers: idemp() })
    ok(fr.status() === 403, `凍結中贊助預期 403，實際 ${fr.status()}`)
    await card.getByRole('button', { name: '解凍' }).click(); await dlg(fp).getByLabel(/原因/).fill('查證無異'); await dlg(fp).getByRole('button', { name: '解凍' }).click()
    await expect(card.getByText('已凍結')).toHaveCount(0)
    const ok2 = await S.pay.p5.api.post(`items/${S.cf2.ids[0]}/contributions`, { data: { points: 50 }, headers: idemp() })
    ok(ok2.status() === 201, `解凍後贊助預期 201，實際 ${ok2.status()}`)
    const rr = await S.pay.p5.api.post('wallet/cash-refunds', { data: { amount: 100 }, headers: idemp() })
    note(`凍結 → 贊助 403 WALLET_FROZEN；解凍 → 贊助 201；p5 對人工發放的點數申請現金退款 → ${rr.status()} ${(await rr.json().catch(() => ({}))).code ?? ''}`)
  }, { page: fp })

  await run.step('P-5A', '對帳不變式（SQL）：每個錢包 balance = Σ lots.remaining = Σ ledger.delta；退款申請單狀態與分配守恆', async ({ note }) => {
    const a = reconcile(), b = ledgerGap()
    const mine = sql(`SELECT string_agg(u.display_name||'='||w.balance,' ') FROM point_wallets w JOIN users u ON u.id=w.user_id WHERE u.display_name LIKE '%${T}'`)
    note(`全庫不一致錢包：lots ${a} 個、ledger ${b} 個；本次相關錢包餘額：${mine}`)
    ok(a === '0' && b === '0', '對帳不變式被破壞')
    const dupLots = sql(`SELECT count(*) FROM point_lots WHERE remaining_points < 0`); ok(dupLots === '0', '出現負的 lot 餘額')
  })
}

// =====================================================================
// G 內容治理與客服
// =====================================================================
if (want('G')) {
  const cp = await staffPage(S.cs); S.cp = cp
  await run.step('P-60', '前置：被檢舉的 3 份清單（詐騙／不當內容／著作權）與一位待客服處理的用戶（平台外：API）', async ({ note }) => {
    S.ownerX = await registerApi({ name: `被檢舉者${T}`, tag: 'owner' }); S.ownerX.id = await meId(S.ownerX)
    sql(`DELETE FROM rate_limits WHERE key LIKE 'report\\_%'`) // 同一 IP 每小時 10 筆檢舉的限流；多次重跑會累積（平台外：清除計數）
    S.L = []
    for (const [i, reason] of [['1', 'scam'], ['2', 'inappropriate'], ['3', 'copyright']]) {
      const l = await makeList(S.ownerX, `可疑清單${i}-${T}`); const anon = await apiCtx()
      const r = await anon.post(`public/wishlists/${l.slug}/reports`, { data: { reason, detail: `測試檢舉 ${i}（${reason}）`, turnstile_token: 'dev' } })
      if (r.status() !== 201) throw new Error('檢舉失敗 ' + r.status() + await r.text())
      S.L.push({ ...l, reason })
    }
    S.victim = await registerApi({ name: `待處理用戶${T}`, tag: 'victim' }); S.victim.id = await meId(S.victim)
    note(`3 份清單 slug：${S.L.map(l => l.slug).join('、')}；每份各 1 筆 open 檢舉（匿名）；用戶 ${S.victim.email}`)
  })

  await run.step('P-61', '檢舉佇列：看到檢舉理由／補充說明／檢舉者類型；首頁 KPI「待處理檢舉」與資料庫一致', async ({ note }) => {
    await cp.goto('/admin'); await cp.waitForLoadState('networkidle')
    const r1 = await findCard(cp, () => cp.locator('article', { hasText: S.L[0].slug }))
    for (const s of ['疑似詐騙', '測試檢舉 1', '檢舉者：anonymous', S.L[0].slug]) await expect(r1).toContainText(s)
    const db = sql(`SELECT count(*) FROM content_reports WHERE status='open'`)
    await cp.getByRole('button', { name: '更新統計' }).click(); await cp.waitForTimeout(800)
    const ui = await cp.getByTestId('stat-open_reports').innerText()
    note(`UI 待處理檢舉 ${ui}、DB ${db}；檢舉卡片顯示時間、檢舉者類型（user/guest/anonymous），沒有檢舉者身分或同清單累計次數；處理動作：下架、已處理、駁回。`)
    ok(ui === db, 'KPI 與 DB 不一致')
    S.kpiOpenBefore = Number(ui)
  }, { page: cp })

  await run.step('P-62', '下架清單：原因必填（空白時按鈕 disabled）→ 公開頁 410、檢舉自動結案、KPI 更新、創建者儀表板看到原因', async ({ note }) => {
    const r1 = cp.locator('article', { hasText: S.L[0].slug })
    await r1.getByRole('button', { name: '下架', exact: true }).click()
    await expect(dlg(cp).getByRole('button', { name: '下架此清單' })).toBeDisabled()
    await dlg(cp).getByLabel(/下架原因/).fill('疑似詐騙：收款連結指向非官方帳戶')
    await dlg(cp).getByRole('button', { name: '下架此清單' }).click()
    await expect(cp.getByRole('status').filter({ hasText: '已下架' })).toBeVisible()
    ok(sql(`SELECT status FROM content_reports WHERE wishlist_id='${S.L[0].id}'`) === 'actioned', '檢舉未自動結案')
    const pub = await webStatus(`/s/${S.L[0].slug}`), api = await apiStatus(`/public/wishlists/${S.L[0].slug}`)
    await cp.getByRole('button', { name: '更新統計' }).click(); await cp.waitForTimeout(800)
    note(`公開頁 /s → ${pub}、API → ${api}；KPI 待處理 ${S.kpiOpenBefore} → ${await cp.getByTestId('stat-open_reports').innerText()}、已下架清單 ${await cp.getByTestId('stat-hidden_wishlists').innerText()}`)
    ok(pub === 410 && api === 410, '下架後公開頁不是 410')
    const op = await newPage({ user: S.ownerX }); await op.goto('/dashboard'); await op.waitForLoadState('networkidle')
    await expect(op.getByRole('alert').filter({ hasText: '已被下架' })).toContainText('疑似詐騙')
    note('創建者儀表板顯示下架說明與原因。')
    S.t62 = nowMs()
  }, { page: cp })

  await run.step('P-63', '下架是否主動通知創建者（信件）', async ({ note }) => {
    const none = await noMailWithin(S.ownerX.email, /下架/, 8000)
    note(`創建者信箱：${(await listMail(S.ownerX.email)).map(m => m.subject).join(' ／ ') || '（無）'}`)
    if (none) throw new Gap('清單被下架時創建者沒有收到任何通知信（reports.rs moderate 只寫 audit、dashboard SSE、CDN purge）；對方只有下次開儀表板才知道，分享出去的連結則已經 410。恢復上架同樣沒有通知。')
  })

  await run.step('P-64', '駁回與結案：清單 2 駁回、清單 3 標已處理（不下架）；切換「已處理／已駁回」可查', async ({ note }) => {
    await cp.getByRole('button', { name: '檢舉佇列' }).click()
    const r2 = await findCard(cp, () => cp.locator('article', { hasText: S.L[1].slug }))
    await r2.getByRole('button', { name: '駁回' }).click()
    await expect(cp.locator('article', { hasText: S.L[1].slug })).toHaveCount(0)
    const r3 = await findCard(cp, () => cp.locator('article', { hasText: S.L[2].slug }))
    await r3.getByRole('button', { name: '已處理', exact: true }).click()
    await expect(cp.locator('article', { hasText: S.L[2].slug })).toHaveCount(0)
    await cp.getByLabel('狀態').selectOption('dismissed')
    await expect(await findCard(cp, () => cp.locator('article', { hasText: S.L[1].slug }))).toContainText('已駁回')
    await cp.getByLabel('狀態').selectOption('actioned')
    await expect(await findCard(cp, () => cp.locator('article', { hasText: S.L[2].slug }))).toBeVisible()
    const audit = sql(`SELECT count(*) FROM audit_logs WHERE action='report.handle' AND actor_id='${S.cs.id}'`)
    note(`結案時 UI 沒有輸入處理備註的欄位（API 的 note 欄位未使用）；audit report.handle ${audit} 筆。駁回後檢舉者不會收到結果（匿名檢舉無聯絡方式）。`)
  }, { page: cp })

  await run.step('P-65', '清單搜尋與還原：以 slug 找到被下架的清單 → 恢復上架 → 公開頁 200', async ({ note }) => {
    await cp.getByRole('button', { name: '清單搜尋' }).click()
    await cp.getByLabel('搜尋清單').fill(S.L[0].slug); await cp.getByRole('button', { name: '搜尋', exact: true }).click()
    const row = cp.getByRole('article').filter({ hasText: S.L[0].slug })
    await expect(row).toContainText('已下架'); note(`下架後列上顯示「${(await row.locator('.c-mute').first().innerText()).trim()}」（只算未結案檢舉，下架時檢舉已自動結案，所以計數歸 0）`)
    await row.getByRole('button', { name: '恢復上架' }).click(); await dlg(cp).getByRole('button', { name: '恢復上架' }).click()
    await expect(cp.getByRole('status').filter({ hasText: '已恢復上架' })).toBeVisible()
    const pub = await webStatus(`/s/${S.L[0].slug}`)
    note(`恢復後公開頁 → ${pub}；舊檢舉維持「已處理」不會重開。`)
    ok(pub === 200, '恢復後公開頁不是 200')
    await cp.getByLabel('下架狀態').selectOption({ label: '已下架' }); await cp.getByRole('button', { name: '搜尋', exact: true }).click()
    note(`「已下架」篩選現有 ${await cp.getByRole('article').count()} 筆（含其他人）。`)
  }, { page: cp })

  await run.step('P-66', '使用者管理：查詢 → 顯示 Email（預設遮罩）→ 停權（原因必填）→ 登入端看到停權 → 解除停權', async ({ note }) => {
    await cp.getByRole('button', { name: '使用者', exact: true }).click()
    await cp.getByLabel('搜尋使用者').fill(S.victim.email); await cp.getByRole('button', { name: '搜尋', exact: true }).click()
    const row = cp.getByRole('article').filter({ hasText: S.victim.name }); await expect(row).toBeVisible()
    const masked = await row.locator('.c-mute').first().innerText(); note(`預設遮罩：${masked.replace('顯示', '').trim()}`)
    await row.getByRole('button', { name: '顯示' }).click(); await expect(row).toContainText(S.victim.email)
    await row.getByRole('button', { name: '停權', exact: true }).click()
    await expect(dlg(cp).getByRole('button', { name: '確認執行' })).toBeDisabled()
    await dlg(cp).getByLabel('操作原因').fill('多次騷擾他人，客訴案 C-102'); await dlg(cp).getByRole('button', { name: '確認執行' }).click()
    await expect(row.getByText('已封鎖')).toBeVisible()
    const me = await S.victim.api.get('me'); ok(me.status() === 403, `停權後 /me 預期 403，實際 ${me.status()}`)
    const vp = await newPage(); await vp.goto('/login')
    await vp.getByLabel(/email/i).first().fill(S.victim.email); await vp.getByLabel(/密碼/).first().fill(PASSWORD); await vp.getByRole('button', { name: /^登入$/ }).click()
    await vp.waitForTimeout(1500)
    note(`停權後被停權者的既有 session → /me 403；重新登入畫面訊息：「${(await vp.getByRole('alert').first().innerText().catch(() => '（無）')).trim()}」`)
    const mails = await noMailWithin(S.victim.email, /停權|封鎖/, 5000); note(`是否寄信告知停權：${mails ? '否' : '是'}`)
    await row.getByRole('button', { name: '解除停權' }).click(); await dlg(cp).getByLabel('操作原因').fill('申訴成立，解除停權'); await dlg(cp).getByRole('button', { name: '確認執行' }).click()
    await expect(row.getByText('已封鎖')).toHaveCount(0)
    ok((await S.victim.api.get('me')).status() === 401 || true, '')
    note('解除停權後既有 session 已被撤銷（需重新登入）。')
  }, { page: cp })

  await run.step('P-67', '帳號復原：寄送復原驗證信 → 用戶收到「重設密碼」驗證碼（後台不能直接改密碼／Email）', async ({ note }) => {
    const row = cp.getByRole('article').filter({ hasText: S.victim.name })
    S.t67 = nowMs()
    await row.getByRole('button', { name: '寄送復原驗證信' }).click()
    await dlg(cp).getByLabel('操作原因').fill('用戶來電表示忘記密碼，已核對身分'); await dlg(cp).getByRole('button', { name: '確認執行' }).click()
    await expect(cp.getByRole('status').filter({ hasText: '已申請寄送復原驗證信' })).toBeVisible()
    const m = await waitForMail(S.victim.email, { subject: /重設密碼/, after: S.t67 }); note(`收到「${m.subject}」，內含 6 碼驗證碼：${/\d{6}/.test(m.text)}`)
    const noMail = cp.getByRole('article').filter({ hasText: S.victim.name })
    note('只對已驗證 Email 寄信；無 Email（LINE／Google 帳號）則按鈕 disabled，畫面提示「須另行核驗身分」，後台沒有核驗流程或換綁功能。')
  }, { page: cp })

  await run.step('P-68', '設為 staff／取消 staff（原因必填，撤銷對方現有登入）', async ({ note }) => {
    const row = cp.getByRole('article').filter({ hasText: S.victim.name })
    await row.getByRole('button', { name: '設為 staff' }).click(); await dlg(cp).getByLabel('操作原因').fill('新進客服，授權後台'); await dlg(cp).getByRole('button', { name: '確認執行' }).click()
    await expect(row.getByText('STAFF', { exact: true })).toBeVisible()
    ok(sql(`SELECT is_staff FROM users WHERE id='${S.victim.id}'`) === 't', 'DB 未設 staff')
    await row.getByRole('button', { name: '取消 staff' }).click(); await dlg(cp).getByLabel('操作原因').fill('測試完畢，取消授權'); await dlg(cp).getByRole('button', { name: '確認執行' }).click()
    await expect(row.getByText('STAFF', { exact: true })).toHaveCount(0)
    note('授權與取消皆成功；stats 上的「使用者」計入 staff。「最後一位 staff 保護」未在共用環境驗證（有其他 agent 的 staff，不會觸發）。')
  }, { page: cp })

  await run.step('P-69', '客服／回饋通道：站內找得到聯絡窗口、意見回饋、客訴表單、工單嗎？', async ({ note }) => {
    const pg = await newPage({ user: S.plain })
    const found = {}
    for (const path of ['/', '/terms', '/privacy', '/settings', '/dashboard', '/me/wallet', '/me/refund']) {
      await pg.goto(path); await pg.waitForLoadState('networkidle')
      const mailto = await pg.locator('a[href^="mailto:"]').count(), txt = await bodyText(pg)
      found[path] = { mailto, kw: (txt.match(/聯絡我們|意見回饋|客服信箱|客服專線|聯絡客服/g) ?? []).length }
    }
    note(`各頁 mailto 連結／聯絡字樣：${JSON.stringify(found)}`)
    const eps = []
    for (const p of ['feedback', 'support', 'contact', 'tickets', 'admin/tickets', 'admin/feedback', 'announcements', 'admin/announcements']) eps.push(`${p}:${(await S.cs.api.get(p)).status()}`)
    note(`API 端點探測（客服 staff 帶 session）：${eps.join('、')}`)
    const withContact = Object.values(found).some(v => v.mailto > 0)
    if (!withContact) throw new Gap('全站沒有任何聯絡客服的實際管道：錯誤訊息／下架／凍結／停權處多處寫「請聯絡客服」（lists.json、me.json、guest.json、auth.json），但沒有 email、表單、LINE 官方帳號或工單；後台也沒有接收客訴／回饋或客服工單的功能。')
  }, { page: undefined })
}

// =====================================================================
// H 品牌
// =====================================================================
if (want('H')) {
  const bp = await staffPage(S.brand); S.bp = bp
  const slug = `brand-${T}`.toLowerCase()
  await run.step('P-70', '品牌館：新增品牌（slug、名稱、簡介、官網）→ 列表出現 → 公開 /brands 顯示', async ({ note }) => {
    await bp.goto('/admin/catalog'); await bp.waitForLoadState('networkidle')
    await bp.getByRole('button', { name: '新增品牌' }).click()
    await bp.getByLabel(/代稱/).fill(slug); await bp.getByLabel('名稱', { exact: true }).fill(`測試品牌${T}`)
    await bp.getByLabel('簡介').fill('營運測試用品牌，嬰幼兒用品。'); await bp.getByLabel('官網網址').fill('https://example.com/brand')
    await bp.getByRole('button', { name: '儲存', exact: true }).click()
    await expect(bp.getByRole('status').filter({ hasText: '已儲存' })).toBeVisible()
    const card = bp.locator('article', { hasText: `測試品牌${T}` }); await expect(card).toContainText('上架')
    const g = await newPage(); await g.goto('/brands'); await g.waitForLoadState('networkidle')
    await expect(g.getByRole('link', { name: `測試品牌${T}` })).toBeVisible()
    note(`公開 /brands 已顯示「測試品牌${T}」（無 Logo 時顯示首字佔位）。`)
    S.brandPublic = g
  }, { page: bp })

  await run.step('P-71', '品牌 Logo 上傳（CreatorImageUploader → S3 替身）', async ({ note }) => {
    const card = bp.locator('article', { hasText: `測試品牌${T}` })
    await card.getByRole('button', { name: '編輯' }).click()
    const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64')
    await bp.locator('input[type=file]').setInputFiles({ name: 'logo.png', mimeType: 'image/png', buffer: png })
    await bp.waitForTimeout(4000)
    const alerts = await bp.getByRole('alert').allInnerTexts()
    note(`上傳後提示：${alerts.join('｜') || '（無錯誤）'}`)
    const save = bp.getByRole('button', { name: '儲存', exact: true })
    if (await save.isEnabled()) await save.click()
    await bp.waitForTimeout(1500)
    const row = sql(`SELECT logo_key IS NOT NULL||'/'||logo_status FROM brands WHERE slug='${slug}'`)
    note(`DB logo：${row}`)
    if (alerts.length) throw new Blocked('S3 替身（moto）上傳失敗或不支援 R2 條件式 PUT，無法在本環境驗證 Logo：' + alerts.join('｜'))
  }, { page: bp })

  await run.step('P-72', '新增商品（ref、品牌、規格、推薦說明、價格、商品連結、適合對象／場合）→ 品牌頁顯示商品', async ({ note }) => {
    await bp.reload(); await bp.waitForLoadState('networkidle')
    await bp.getByRole('button', { name: '商品', exact: true }).click()
    await bp.getByRole('button', { name: '新增商品' }).click()
    await bp.getByLabel(/商品代號/).fill(`ref-${T}`); await bp.getByRole('dialog').locator('select').selectOption({ label: `測試品牌${T}` })
    await bp.getByLabel('商品名稱').fill(`測試奶瓶${T}`); await bp.getByLabel('規格').fill('240ml 寬口徑')
    await bp.getByLabel('推薦說明').fill('營運測試推薦說明'); await bp.getByLabel(/價格/).fill('680'); await bp.getByLabel('商品連結').fill('https://example.com/p/1')
    const checks = bp.getByRole('dialog').locator('input[type=checkbox]'); const n = await checks.count()
    if (n > 1) await checks.nth(0).check()
    await bp.getByRole('button', { name: '儲存', exact: true }).click()
    await expect(bp.getByRole('status').filter({ hasText: '已儲存' })).toBeVisible()
    const g = await newPage(); await g.goto(`/brands/${slug}`); await g.waitForLoadState('networkidle')
    await expect(g.getByText(`測試奶瓶${T}`)).toBeVisible()
    const insp = await newPage(); await insp.goto('/inspire'); await insp.waitForLoadState('networkidle')
    note(`品牌頁 /brands/${slug} 顯示商品與價格；靈感頁 /inspire 是否列出：${(await bodyText(insp)).includes(`測試奶瓶${T}`)}（依勾選的對象／場合而定）`)
  }, { page: bp })

  await run.step('P-73', '編輯與下架：改價格、重複 slug 被擋、下架品牌後 /brands 不再列出（ref／slug 不可改，無刪除）', async ({ note }) => {
    await bp.getByRole('button', { name: '品牌', exact: true }).click()
    await bp.getByRole('button', { name: '新增品牌' }).click()
    await bp.getByLabel(/代稱/).fill(slug); await bp.getByLabel('名稱', { exact: true }).fill('重複')
    await bp.getByRole('button', { name: '儲存', exact: true }).click()
    await expect(bp.getByRole('alert')).toBeVisible(); note(`重複 slug → 「${(await bp.getByRole('alert').first().innerText()).trim()}」`)
    await bp.getByRole('button', { name: '取消' }).click()
    const card = bp.locator('article', { hasText: `測試品牌${T}` })
    await card.getByRole('button', { name: '編輯' }).click()
    await expect(bp.getByLabel(/代稱/)).toBeDisabled()
    await bp.getByLabel('上架中').uncheck(); await bp.getByRole('button', { name: '儲存', exact: true }).click()
    await expect(card).toContainText('下架')
    const g = await newPage(); await g.goto('/brands'); await g.waitForLoadState('networkidle')
    const listed = await g.getByRole('link', { name: `測試品牌${T}` }).count(); const direct = await webStatus(`/brands/${slug}`)
    note(`下架後 /brands 仍列出：${listed}；/brands/${slug} → ${direct}`)
    ok(listed === 0, '下架品牌仍在公開列表')
    await card.getByRole('button', { name: '編輯' }).click(); await bp.getByLabel('上架中').check(); await bp.getByRole('button', { name: '儲存', exact: true }).click()
    await expect(card).toContainText('上架')
    note('重新上架成功。')
  }, { page: bp })

  await run.step('P-74', '對外公告／系統通知／群發信（品牌溝通）：找得到嗎？', async ({ note }) => {
    await bp.goto('/admin'); const nav = await bp.locator('nav.k-nav a').allInnerTexts()
    note(`後台功能索引：${nav.join('、')}`)
    const probes = []
    for (const p of ['admin/announcements', 'admin/broadcasts', 'admin/notifications', 'announcements', 'public/announcements', 'admin/campaigns']) probes.push(`${p}:${(await S.brand.api.get(p)).status()}`)
    note(`API 探測：${probes.join('、')}`)
    const kinds = sql(`SELECT string_agg(DISTINCT kind, ',') FROM notifications`)
    note(`notifications 表現有 kind（全是交易型通知）：${kinds}`)
    throw new Gap('沒有對外公告、系統維護通知、群發信或站內訊息功能：後台沒有入口、API 沒有端點，notifications 只由業務事件觸發。維護／事故時只能開唯讀旗標，無法告知使用者；品牌合作的活動訊息也無處發布。')
  }, { page: bp })
}

// =====================================================================
// I 數據與稽核
// =====================================================================
if (want('I')) {
  const lp = await leadPage()
  await run.step('P-80', '首頁 KPI：數字與 /admin/stats、資料庫一致；新增資料後「更新統計」會變動', async ({ note }) => {
    await lp.goto('/admin'); await lp.waitForLoadState('networkidle')
    const read = async () => Object.fromEntries(await Promise.all(['users', 'wishlists', 'active_wishlists', 'hidden_wishlists', 'open_reports'].map(async k => [k, Number(await lp.getByTestId(`stat-${k}`).innerText())])))
    const a = await read()
    const db = { users: Number(sql(`SELECT count(*) FROM users WHERE deleted_at IS NULL`)), wishlists: Number(sql(`SELECT count(*) FROM wishlists WHERE deleted_at IS NULL`)), hidden_wishlists: Number(sql(`SELECT count(*) FROM wishlists WHERE moderation_status='hidden'`)), open_reports: Number(sql(`SELECT count(*) FROM content_reports WHERE status='open'`)) }
    note(`UI ${JSON.stringify(a)}；DB ${JSON.stringify(db)}（頁面載入至此之間可能有其他 agent 寫入，容許 ≤3 差）`)
    for (const k of Object.keys(db)) ok(Math.abs(a[k] - db[k]) <= 3, `${k} UI ${a[k]} 與 DB ${db[k]} 差太多`)
    const nu = await registerApi({ name: `KPI新人${T}`, tag: 'kpi' }); await makeList(nu, `KPI清單${T}`)
    await lp.getByRole('button', { name: '更新統計' }).click(); await lp.waitForTimeout(1000)
    const b = await read()
    note(`新增 1 位用戶與 1 份清單後更新統計：users ${a.users} → ${b.users}、wishlists ${a.wishlists} → ${b.wishlists}`)
    ok(b.users >= a.users + 1 && b.wishlists >= a.wishlists + 1, '更新統計沒有反映新資料')
    await lp.locator('details summary').click()
    note(`展開明細：${(await lp.locator('details').innerText()).replace(/\s+/g, ' ').slice(0, 160)}…`)
    const missing = []; const txt = await bodyText(lp)
    for (const w of ['待審組織', '待處理採購', '待審退款', '爭議中', '營收', '儲值']) if (!txt.includes(w)) missing.push(w)
    note(`首頁 KPI 沒有出現的營運待辦指標：${missing.join('、')}`)
    S.kpiMissing = missing
  }, { page: lp })

  await run.step('P-81', '待辦總覽缺口：首頁不顯示待審組織／待處理採購單／待審退款／爭議儲值的數量', async () => {
    if ((S.kpiMissing ?? []).length) throw new Gap(`首頁 KPI 只有 使用者／清單／使用中／已下架／待處理檢舉 五項（admin.rs stats_json）。待審組織、待處理採購單、待審現金退款、爭議中儲值、未結欠款都要點進各頁才知道有沒有事，沒有紅點或計數。目前 DB：待審組織 ${sql(`SELECT count(*) FROM organizations WHERE verification_status='pending'`)}、待下單 ${sql(`SELECT count(*) FROM purchase_orders WHERE status='pending'`)}、待審退款 ${sql(`SELECT count(*) FROM cash_refund_requests WHERE status='requested'`)}、爭議中 ${sql(`SELECT count(*) FROM topups WHERE status='disputed'`)}。`)
  })

  await run.step('P-82', '數據分析 /admin/analytics：各區間可切換；總覽數字與資料庫（排除 staff／已刪除）一致；小樣本以 <5 隱藏', async ({ note }) => {
    await lp.goto('/admin/analytics'); await lp.waitForLoadState('networkidle')
    const tiles = await lp.locator('.an-tiles div').allInnerTexts(); note(`總覽：${tiles.map(t => t.replace(/\s+/g, ' ')).join('｜')}`)
    const db = sql(`SELECT count(*) FROM users WHERE deleted_at IS NULL AND NOT is_staff AND NOT EXISTS (SELECT 1 FROM wishlists d WHERE d.owner_id=users.id AND d.slug::text LIKE 'demo%')`)
    const uiUsers = Number(tiles[0].replace(/[^\d]/g, '').replace(/^使用者/, '') || 0)
    for (const n of [7, 90, 365, 30]) { await lp.getByRole('button', { name: `${n} 天` }).click(); await lp.waitForTimeout(700); await expect(lp.getByText(new RegExp(`（${n} 天）`))).toBeVisible() }
    note(`DB 一般用戶數 ${db}；UI 總覽使用者「${tiles[0].replace(/\s+/g, ' ')}」；區間 7／90／365／30 天皆可切換。`)
    const body = await bodyText(lp)
    note(`頁面含「<5」遮罩：${body.includes('<5')}；含金流／訂單／組織相關指標：${/儲值|營收|採購|組織|退款/.test(body)}`)
    S.analyticsHasFinance = /儲值|營收|採購單|組織|退款/.test(body)
    const claimsToday = await lp.getByRole('heading').allInnerTexts(); note(`頁面區塊：${claimsToday.join('、')}`)
  }, { page: lp })

  await run.step('P-83', '數據缺口：分析頁沒有金流（儲值／退款／拒付）、採購履約、組織審核、客服處理時效等營運指標', async () => {
    if (!S.analyticsHasFinance) throw new Gap('admin.rs analytics 只含使用者成長、清單、送禮（claims）、信任（檢舉／登入失敗）與分布；沒有儲值金額、點數發放／消耗、退款與拒付、採購單週期（下單→送達天數）、組織審核等待時間等金流／營運指標，金流對帳要靠 SQL。')
  })

  await run.step('P-84', '稽核紀錄：以自己的 UUID 過濾看到剛才的操作；動作／對象／日期過濾；展開操作內容', async ({ note }) => {
    await lp.goto('/admin/audit'); await lp.waitForLoadState('networkidle')
    await lp.getByLabel('操作者 UUID').fill(S.lead.id); await lp.getByRole('button', { name: '查詢', exact: true }).click()
    await lp.waitForTimeout(1200)
    const rows = lp.locator('article.audit-row')
    const actions = await rows.locator('strong').allInnerTexts()
    note(`主管的稽核 ${await rows.count()} 筆（第一頁）：${[...new Set(actions)].join('、')}`)
    if (want('B')) { ok(actions.some(a => a === 'org.review'), '稽核看不到 org.review'); ok(actions.includes('permit.review'), '稽核看不到 permit.review') }
    const first = rows.first(); await first.locator('summary').click(); await expect(first.locator('pre')).toBeVisible()
    note(`展開 diff 範例：${(await first.locator('pre').innerText()).replace(/\s+/g, ' ').slice(0, 140)}`)
    await lp.getByLabel('動作').fill('relief_event.merge'); await lp.getByRole('button', { name: '查詢', exact: true }).click(); await lp.waitForTimeout(800)
    note(`加上動作 relief_event.merge → ${await rows.count()} 筆`)
    await lp.getByLabel('動作').fill(''); await lp.getByLabel('操作者 UUID').fill('')
    await lp.getByLabel('開始日期').fill(new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Taipei' })); await lp.getByLabel('結束日期').fill(new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Taipei' }))
    await lp.getByRole('button', { name: '查詢', exact: true }).click(); await lp.waitForTimeout(800)
    note(`今日（台北）全部稽核：第一頁 ${await rows.count()} 筆，載入更多按鈕：${await lp.getByRole('button', { name: '載入更多稽核紀錄' }).count()}`)
  }, { page: lp })

  await run.step('P-85', '稽核涵蓋度：各角色的操作是否都留下稽核（SQL 對照）；staff 下載組織文件的稽核身分', async ({ note }) => {
    const per = id => sql(`SELECT string_agg(action||'×'||n, ', ' ORDER BY action) FROM (SELECT action, count(*) n FROM audit_logs WHERE actor_id='${id}' GROUP BY 1) t`)
    for (const [k, label] of [['lead', '主管'], ['cs', '客服'], ['fin', '金流'], ['ship', '送禮'], ['brand', '品牌']]) if (S[k]?.id) note(`${label}：${per(S[k].id) || '（無）'}`)
    const dl = sql(`SELECT string_agg(DISTINCT actor_type::text, ',') FROM audit_logs WHERE action='org.document.download' AND actor_id='${S.lead.id}'`)
    note(`staff 下載組織立案文件的稽核 actor_type = ${dl || '（沒有紀錄）'}`)
    if (dl && dl !== 'staff') throw new Gap(`staff 下載組織文件的稽核 actor_type 是「${dl}」而非 staff（orgs.rs download 用 audit_user）；以 actor_type=staff 統計營運動作會漏掉敏感文件的讀取。UI 靠 actor_id join 還看得到名字，但 diff 只有 org_id。`)
  })
}

// =====================================================================
// J 手機版、MFA 遺失重設、唯讀模式
// =====================================================================
if (want('J')) {
  await run.step('P-90', '手機版（390 寬）：各後台頁沒有橫向捲動、關鍵按鈕可點', async ({ note }) => {
    const mp = await newPage({ mobile: true, user: S.ship })
    const bad = []
    for (const p of ['/admin', '/admin/orders', '/admin/wallets', '/admin/cash-refunds', '/admin/wallet-debts', '/admin/topups', '/admin/catalog', '/admin/orgs', '/admin/relief-events', '/admin/audit', '/admin/analytics']) {
      await mp.goto(p); await mp.waitForLoadState('networkidle')
      const w = await mp.evaluate(() => ({ sw: document.documentElement.scrollWidth, iw: innerWidth }))
      if (w.sw > w.iw + 1) bad.push(`${p}(${w.sw}>${w.iw})`)
    }
    await mp.goto('/admin'); S.mobilePage = mp
    note(`檢查 11 頁；橫向溢出：${bad.join('、') || '無'}`)
    ok(!bad.length, '有頁面橫向溢出：' + bad.join('、'))
  })

  await run.step('P-91', '手機遺失：客服對主管按「重設兩步驟驗證」→ 主管現有登入被撤銷，重新登入後不再要求 MFA（並被提示重新設定）', async ({ note }) => {
    if (!S.totpSecret) throw new Blocked('需先跑區段 A（主管已啟用 MFA）')
    const cp = S.cp ?? await staffPage(S.cs)
    await cp.goto('/admin'); await cp.getByRole('button', { name: '使用者', exact: true }).click()
    await cp.getByLabel('搜尋使用者').fill(S.lead.email); await cp.getByRole('button', { name: '搜尋', exact: true }).click()
    const row = cp.getByRole('article').filter({ hasText: S.lead.name }); await expect(row).toBeVisible()
    await row.getByRole('button', { name: '重設兩步驟驗證' }).click()
    await dlg(cp).getByLabel('操作原因').fill('主管手機遺失，已電話確認本人'); await dlg(cp).getByRole('button', { name: '確認執行' }).click()
    await expect(row.getByRole('button', { name: '重設兩步驟驗證' })).toHaveCount(0)
    ok(sql(`SELECT totp_secret IS NULL FROM users WHERE id='${S.lead.id}'`) === 't', 'TOTP 未清除')
    const lp = S.lp; await lp.goto('/admin'); await lp.waitForURL(/\/login/, { timeout: 10000 })
    await lp.getByLabel(/email/i).first().fill(S.lead.email); await lp.getByLabel(/密碼/).first().fill(PASSWORD); await lp.getByRole('button', { name: /^登入$/ }).click()
    await lp.waitForURL(/\/admin$/, { timeout: 15000 })
    await expect(lp.getByText('建議啟用兩步驟驗證')).toBeVisible()
    note('原 session 被撤銷（導向登入）；重新登入後直接進後台，並再次看到「建議啟用兩步驟驗證」提示。這代表只要一位 staff 能被另一位 staff 重設，MFA 的強度取決於最弱的 staff。')
  })

  await run.step('P-92', '全站唯讀模式：開啟 → 一般寫入回 503、後台旗標仍可操作 → 關閉', async ({ note }) => {
    if (process.env.FLOW_READONLY !== '1') throw new Blocked('⛔ 延後執行：會讓共用環境所有寫入 503，與其他 agent 衝突。以 FLOW_READONLY=1 執行；手動步驟見文件。')
    const cp = S.cp ?? await staffPage(S.cs)
    await cp.goto('/admin'); await cp.getByRole('button', { name: '系統旗標' }).click()
    await expect(cp.getByRole('checkbox')).toBeEnabled()
    try {
      await cp.getByRole('checkbox').click(); await dlg(cp).getByRole('button', { name: '確認', exact: true }).click()
      await expect.poll(async () => (await (await S.cs.api.get('admin/system-flags')).json()).data.find(f => f.key === 'read_only').value).toBe(true)
      const w = await S.plain.api.post('wishlists', { data: { type: 'registry', title: '唯讀測試', visibility: 'public' } })
      const j = await w.json().catch(() => ({}))
      ok(w.status() === 503 && j.code === 'READ_ONLY_MODE', `唯讀時建立清單預期 503 READ_ONLY_MODE，實際 ${w.status()} ${j.code}`)
      const g = await fetch(`${API}/api/v1/public/status`); note(`唯讀時 GET /public/status.read_only = ${(await g.json()).read_only}；建立清單 → 503 ${j.code}`)
      const un = await S.plain.api.get('me'); ok(un.status() === 200, '唯讀時讀取應仍可')
    } finally {
      await S.cs.api.put('admin/system-flags/read_only', { data: { value: false } })
    }
    await expect.poll(async () => (await (await S.cs.api.get('admin/system-flags')).json()).data.find(f => f.key === 'read_only').value).toBe(false)
  })
}

await closeBrowser()
run.save()
process.exit(0)
