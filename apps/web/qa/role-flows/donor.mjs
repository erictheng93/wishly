// 角色流程模擬：用戶・捐助者（docs/role-flows/02-user-donor.md）
// 執行：cd apps/web && node qa/role-flows/donor.mjs   （環境見 docs/role-flows/README.md）
// 原則：能用 UI 就用 UI；只有「平台外」的事（建受捐者資料、取 OTP、發點、時間快轉、金流付款、種子資料）才走 API / SQL / 替身，並在步驟 note 註明。
// 環境變數：DONOR_SKIP_WAIT=1 略過最後等待背景逾期釋放（最長約 5.5 分鐘）。
import { createHmac } from 'node:crypto'
import { expect } from '@playwright/test'
import { API, Blocked, closeBrowser, Gap, MAILPIT, PASSWORD, getBrowser, listMail, otpFrom, pwRequest, randomUUID, recorder, registerApi, resetOtpLimits, sleep, sql, q, uid, waitForMail, WEB } from './lib.mjs'
import { ecpayPaid } from './ecpay-sim.mjs'

expect.configure({ timeout: 10000 })
const run = recorder('donor')
const T = uid() // 本次執行的唯一標記（標題、探索頁過濾用）
const ECPAY = 'https://payment-stage.ecpay.com.tw/**'
const LINE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari Line/14.5.0'
const S = {} // 跨步驟共用狀態

// ---------- 工具 ----------
const hyd = p => p.waitForFunction(() => window.useNuxtApp?.().isHydrating === false, null, { timeout: 15000 }).catch(() => {})
const card = (p, t) => p.locator('li.g-card', { hasText: t })
const done = p => p.getByRole('heading', { name: '✓ 這份交給你了！' })
const taipei = d => new Date(Date.now() + d * 864e5).toISOString()
const must = async (r, what) => { if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${await r.text()}`); return r }
const uidOf = email => sql(`SELECT id FROM users WHERE email='${q(email)}'`)

/** 自訂 context（LINE UA、關閉 JS、手機）；回傳 page（含 problems 收集，同 lib.newPage） */
async function ctxPage({ mobile = false, ua, js = true, user } = {}) {
  const b = await getBrowser()
  const ctx = await b.newContext({ baseURL: WEB, locale: 'zh-TW', timezoneId: 'Asia/Taipei', viewport: mobile ? { width: 390, height: 844 } : { width: 1280, height: 860 }, isMobile: mobile, hasTouch: mobile, userAgent: ua, javaScriptEnabled: js })
  if (user) await ctx.addCookies((await user.api.storageState()).cookies.map(c => ({ ...c, domain: 'localhost' })))
  const page = await ctx.newPage()
  page.problems = []
  page.on('pageerror', e => page.problems.push(`pageerror: ${e.message}`.slice(0, 300)))
  page.on('console', m => { if (m.type() === 'error') page.problems.push(`console: ${m.text()}`.slice(0, 300)) })
  page.on('response', r => { if (r.status() >= 500) page.problems.push(`http${r.status()} ${r.request().method()} ${r.url()}`) })
  return page
}
/** 受捐者資料（前置準備，走 API）。items: {title, qty} 或眾籌 {title, target, deadlineDays} */
async function mkList(owner, { title, visibility = 'public', surprise = false, showNames = false, items = [], publish = true, ttl, address = false }) {
  const body = { type: 'registry', title, visibility, surprise_mode: surprise, show_claimer_names: showNames }
  if (surprise) body.event_date = new Date(Date.now() + 30 * 864e5 + 8 * 36e5).toISOString().slice(0, 10)
  if (ttl) body.claim_ttl_hours = ttl
  const w = await (await must(await owner.api.post('wishlists', { data: body }), '建立清單')).json()
  if (address) await must(await owner.api.put(`wishlists/${w.id}/shipping-address`, { data: { recipient_name: '王小米', phone: '0912345678', address: '台北市大安區和平東路二段 1 號 3 樓' } }), '收件資訊')
  const out = []
  for (const it of items) {
    const data = it.target
      ? { title: it.title, funding_mode: 'crowdfund', target_points: it.target, funding_deadline: taipei(it.days ?? 7), priority: 'medium' }
      : { title: it.title, qty_needed: it.qty ?? 1, priority: 'medium', funding_mode: 'quantity' }
    const r = await (await must(await owner.api.post(`wishlists/${w.id}/items`, { data }), '新增品項 ' + it.title)).json()
    out.push({ id: r.id, title: it.title })
  }
  let slug = w.slug
  if (publish) {
    const cur = await (await must(await owner.api.get(`wishlists/${w.id}`), '讀取清單')).json()
    const p = await (await must(await owner.api.patch(`wishlists/${w.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } }), '發佈')).json()
    slug = p.slug ?? cur.wishlist.slug
  }
  slug ??= (await (await owner.api.get(`wishlists/${w.id}`)).json()).wishlist.slug
  return { id: w.id, slug, title, items: out, byTitle: Object.fromEntries(out.map(i => [i.title, i.id])) }
}
const setStatus = async (owner, list, status) => {
  const cur = await (await must(await owner.api.get(`wishlists/${list.id}`), '讀取')).json()
  await must(await owner.api.patch(`wishlists/${list.id}`, { data: { status, expected_updated_at: cur.wishlist.updated_at } }), '改狀態 ' + status)
}
const claimsOf = (itemId) => sql(`SELECT count(*) FILTER (WHERE status IN ('reserved','purchased','delivered')), coalesce(sum(qty) FILTER (WHERE status IN ('reserved','purchased','delivered')),0) FROM claims WHERE item_id='${itemId}'`)
const qtyClaimed = itemId => sql(`SELECT qty_claimed FROM wishlist_items WHERE id='${itemId}'`)
/** 以 API 當會員認捐（前置準備或併發探測用） */
const apiClaim = (ctx, itemId, qty = 1, key = randomUUID(), extra = {}) => ctx.post(`${API}/api/v1/items/${itemId}/claims`, { data: { qty, ...extra }, headers: { 'Idempotency-Key': key, Origin: WEB } })
const loginForm = async (p, email) => {
  await p.getByLabel('Email').fill(email)
  await p.getByLabel('密碼', { exact: true }).fill(PASSWORD)
  await p.getByRole('button', { name: '登入', exact: true }).click()
}
const noHScroll = async p => expect(await p.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(1)

// =====================================================================
// 前置準備：受捐者、清單、營運帳號（走 API；不是捐助者的操作）
// =====================================================================
let owner, staff, L = {}
await run.step('D-00', '前置準備：用 API 建受捐者「小愛」與各種狀態的清單、營運帳號', async ({ note }) => {
  owner = await registerApi({ name: `小愛${T.slice(0, 3)}`, tag: 'don-owner' })
  staff = await registerApi({ name: `營運${T.slice(0, 3)}`, tag: 'don-staff', staff: true })
  L.main = await mkList(owner, { title: `${T} 小愛的待產清單`, showNames: true, items: [{ title: '奶瓶組', qty: 3 }, { title: '嬰兒帽', qty: 1 }, { title: '揹巾', qty: 2 }, { title: '圍兜', qty: 2 }, { title: '尿布', qty: 3 }] })
  L.surprise = await mkList(owner, { title: `${T} 驚喜生日清單`, surprise: true, showNames: true, items: [{ title: '生日蛋糕', qty: 2 }] })
  L.friends = await mkList(owner, { title: `${T} 好友限定清單`, visibility: 'friends', items: [{ title: '安撫奶嘴', qty: 1 }] })
  L.private = await mkList(owner, { title: `${T} 私人清單`, visibility: 'private', items: [{ title: '秘密禮物', qty: 1 }] })
  L.closed = await mkList(owner, { title: `${T} 已結束清單`, items: [{ title: '積木', qty: 2 }] })
  L.gone = await mkList(owner, { title: `${T} 將被下架清單`, items: [{ title: '違規品', qty: 1 }] })
  L.draft = await mkList(owner, { title: `${T} 草稿清單`, publish: false, items: [{ title: '草稿品項', qty: 1 }] })
  L.empty = await mkList(owner, { title: `${T} 空清單`, items: [{ title: '將被刪除的品項', qty: 1 }] })
  await must(await owner.api.delete(`items/${L.empty.items[0].id}`), '刪除品項（讓清單上線但沒有品項）')
  L.race = await mkList(owner, { title: `${T} 搶購測試清單`, items: [{ title: 'API 搶購品', qty: 1 }, { title: '同人雙開', qty: 3 }] })
  L.cf = await mkList(owner, { title: `${T} 眾籌清單`, address: true, showNames: true, items: [{ title: '嬰兒推車', target: 300 }, { title: '搖籃', target: 200 }, { title: '玩具車', target: 150 }, { title: '奶粉', qty: 3 }] })
  // 下架（營運動作）：跳過 CDN 之外的真實流程，直接用營運 API
  await must(await staff.api.patch(`admin/wishlists/${L.gone.id}/moderation`, { data: { moderation_status: 'hidden', reason: 'QA 捐助者流程：測 410' } }), '下架')
  note(`標記 T=${T}；清單 slug：main=${L.main.slug} surprise=${L.surprise.slug} friends=${L.friends.slug} private=${L.private.slug} closed=${L.closed.slug} gone=${L.gone.slug} race=${L.race.slug} cf=${L.cf.slug}`)
  note('受捐者與營運動作皆為前置（API）；gone 清單由營運 API 下架以測 410')
})

// =====================================================================
// A. 未登入訪客瀏覽（桌面 / 手機 / LINE / 無 JS）
// =====================================================================
S.obs = await ctxPage() // 「訪客 B」：全程不登入，開著分享頁當即時更新觀察者
await run.step('D-01', '桌面・未登入開分享頁：標題、進度、品項卡、登入引導、SSR OG', async ({ note }) => {
  const p = S.obs
  const res = await p.goto(`/s/${L.main.slug}`); await hyd(p)
  expect(res.status()).toBe(200)
  await expect(p.getByRole('heading', { level: 1, name: L.main.title })).toBeVisible()
  await expect(p.locator('li.g-card')).toHaveCount(5)
  await expect(p.getByRole('progressbar').first()).toBeVisible()
  await expect(p.getByText('完成度 0%')).toBeVisible()
  await expect(p.getByText('送禮或贊助需要登入會員')).toBeVisible()
  await expect(p.getByRole('link', { name: '登入後來送' })).toHaveCount(5)
  await expect(p.getByRole('button', { name: '我來送' })).toHaveCount(0)
  await expect(p.getByRole('dialog')).toHaveCount(0)
  await expect(p.getByLabel(/暱稱|聯絡方式/)).toHaveCount(0)
  // SSR 原始 HTML（爬蟲視角，不跑 JS）
  const html = await (await p.request.get(`/s/${L.main.slug}`)).text()
  const meta = k => html.match(new RegExp(`<meta[^>]+(?:property|name)="${k}"[^>]+content="([^"]*)"`))?.[1]
  note(`og:title=${meta('og:title')}｜og:description=${meta('og:description')}｜og:image=${meta('og:image')}`)
  expect(meta('og:title')).toContain(L.main.title)
  expect(html).toContain(`<h1>${L.main.title}</h1>`)
  await expect(p.getByText('即時更新中')).toBeVisible()
  note('footer 顯示「即時更新中」（SSE 已連上）；這個頁面保持開啟當觀察者')
}, { page: S.obs })

S.aunt = await ctxPage({ mobile: true, ua: LINE_UA })
await run.step('D-02', '手機 390 寬・LINE 內建瀏覽器 UA：版面、LINE 引導橫幅、點擊目標', async ({ note }) => {
  const p = S.aunt
  await p.goto(`/s/${L.main.slug}`); await hyd(p)
  await expect(p.getByRole('heading', { level: 1, name: L.main.title })).toBeVisible()
  await noHScroll(p)
  const banner = p.locator('.g-banner', { hasText: '用預設瀏覽器開啟' })
  await expect(banner).toBeVisible()
  const h = await card(p, '奶瓶組').getByRole('link', { name: '登入後來送' }).evaluate(e => Math.round(e.getBoundingClientRect().height))
  const w = await card(p, '奶瓶組').getByRole('link', { name: '登入後來送' }).evaluate(e => Math.round(e.getBoundingClientRect().width))
  note(`「登入後來送」按鈕尺寸 ${w}x${h}px（建議 ≥44px）`)
  await banner.getByRole('button', { name: '知道了' }).click()
  await expect(banner).toBeHidden()
  await p.reload(); await hyd(p)
  await expect(banner).toBeHidden()
  note('LINE UA 為模擬字串（isLineBrowser 只比對 /Line\\//）；真機 LINE 內建瀏覽器行為見 D-90')
  if (h < 44) throw new Gap(`按鈕高度僅 ${h}px，小於 44px 觸控建議`)
}, { page: S.aunt })

await run.step('D-03', '驚喜清單：訪客看到的提示與捐助者姓名顯示（對照一般清單）', async ({ note }) => {
  const p = await ctxPage()
  await p.goto(`/s/${L.surprise.slug}`); await hyd(p)
  await expect(p.getByText('這是驚喜清單：活動日前建立者看不到誰送了什麼')).toBeVisible()
  await expect(card(p, '生日蛋糕')).toContainText('0 / 2')
  await expect(p.getByRole('link', { name: '登入後來送' })).toBeVisible()
  // 一般清單（D-01）無此提示
  await expect(S.obs.getByText('這是驚喜清單')).toHaveCount(0)
  note('驚喜清單對捐助者仍可正常認捐；提示文字提醒不要對壽星洩漏。建立者本人看到的遮蔽屬受捐者流程')
  S.surpriseDonor = p
}, { page: undefined })

await run.step('D-04', '品項深連結 ?item=<id>：捲到該卡並加亮', async () => {
  const p = await ctxPage()
  await p.goto(`/s/${L.main.slug}?item=${L.main.byTitle['尿布']}`); await hyd(p)
  await expect(p.locator(`#item-${L.main.byTitle['尿布']}.g-hl`)).toBeVisible()
  await p.context().close()
}, {})

await run.step('D-05', '關閉 JavaScript（舊瀏覽器／內嵌）：SSR 內容仍可讀，無送禮按鈕', async ({ note }) => {
  const p = await ctxPage({ js: false })
  await p.goto(`/s/${L.main.slug}`)
  await expect(p.getByRole('heading', { level: 1, name: L.main.title })).toBeVisible()
  await expect(p.locator('li.g-card')).toHaveCount(5)
  await expect(p.getByText('需要 JavaScript 才能送禮')).toBeVisible()
  note('登入連結仍是一般連結，可點')
  await p.context().close()
}, {})

// =====================================================================
// B. 登入引導與註冊（阿姨：LINE 點開 → 沒帳號 → 註冊 → 回到原清單）
// =====================================================================
S.auntEmail = `flow-aunt-${uid()}@example.com`; S.auntName = `阿姨${T.slice(0, 3)}`
await run.step('D-06', '按「登入後來送」→ 登入頁帶 redirect → 改點「建立帳號」，redirect 仍保留', async ({ note }) => {
  const p = S.aunt
  await card(p, '奶瓶組').getByRole('link', { name: '登入後來送' }).click()
  await expect(p).toHaveURL(new RegExp(`/login\\?redirect=(%2F|/)s(%2F|/)${L.main.slug}$`))
  await expect(p.getByText('要送禮，請先登入會員')).toBeVisible()
  await p.getByRole('link', { name: '建立帳號' }).click()
  await expect(p).toHaveURL(new RegExp(`/register\\?redirect=(%2F|/)s(%2F|/)${L.main.slug}$`))
  note('登入頁另有 Google / LINE 登入鈕（第三方登入無法在本環境驗證，見 D-91）')
}, { page: S.aunt })

await run.step('D-07', '註冊表單驗證：密碼太短、錯誤驗證碼、重寄倒數', async ({ note }) => {
  const p = S.aunt
  resetOtpLimits()
  await p.getByLabel('顯示名稱').fill(S.auntName)
  await p.getByLabel('Email').fill(S.auntEmail)
  await p.getByLabel('密碼', { exact: true }).fill('short')
  await p.getByRole('button', { name: '寄送驗證碼' }).click()
  await expect(p.getByRole('alert')).toContainText('密碼至少需 8 個字元')
  await p.getByLabel('密碼', { exact: true }).fill(PASSWORD)
  S.t0 = Date.now() - 1000
  await p.getByRole('button', { name: '寄送驗證碼' }).click()
  await expect(p.getByText(`驗證碼已寄到 ${S.auntEmail}`)).toBeVisible()
  await expect(p.getByText(/秒後可重新寄送/)).toBeVisible()
  await p.locator('.c-otp input').first().fill('000000')
  await expect(p.getByRole('alert')).toContainText('驗證碼不正確或已過期')
  const mail = await waitForMail(S.auntEmail, { after: S.t0 })
  S.otp = otpFrom(mail.text)
  note(`OTP 信主旨「${mail.subject}」；驗證碼取自 Mailpit（平台外動作）`)
}, { page: S.aunt })

await run.step('D-08', '輸入正確驗證碼完成註冊 → 自動回到原清單 /s/slug 並已登入', async () => {
  const p = S.aunt
  await p.locator('.c-otp input').first().fill(S.otp)
  const btn = p.getByRole('button', { name: '完成註冊' })
  if (/\/register/.test(p.url()) && await btn.isVisible().catch(() => false)) await btn.click().catch(() => {})
  await p.waitForURL(new RegExp(`/s/${L.main.slug}$`), { timeout: 15000 })
  await hyd(p)
  await expect(card(p, '奶瓶組').getByRole('button', { name: '我來送' })).toBeVisible()
  await expect(p.getByRole('link', { name: '登入後來送' })).toHaveCount(0)
  await expect(p.getByText('送禮或贊助需要登入會員')).toHaveCount(0)
}, { page: S.aunt })

await run.step('D-09', '登入後的分享頁／我的送禮頁：找得到帳號選單、登出、點數錢包的入口嗎？（導覽列）', async ({ note }) => {
  const p = S.aunt
  const nav = await p.locator('header.g-top').innerText()
  note(`/s 頁頂部導覽文字：${nav.replace(/\s+/g, ' ')}`)
  const count = async () => ({
    logout: await p.getByRole('button', { name: /登出/ }).count() + await p.getByRole('link', { name: /登出/ }).count(),
    wallet: await p.locator('a[href="/me/wallet"]').count(),
    account: await p.getByRole('button', { name: /帳號選單/ }).count(),
  })
  const a = await count()
  await p.goto('/me/claims'); await hyd(p)
  const b = await count()
  note(`分享頁 登出/錢包連結/帳號選單 = ${a.logout}/${a.wallet}/${a.account}；我的送禮頁 = ${b.logout}/${b.wallet}/${b.account}`)
  await p.goto(`/s/${L.main.slug}`); await hyd(p)
  if (!a.logout && !a.account && !a.wallet && !b.logout && !b.account && !b.wallet) throw new Gap('分享頁與我的送禮頁（GuestTop）沒有帳號選單、登出、錢包／儲值入口；捐助者只有在 /dashboard、/me/wallet 等創建者頁才看得到 UserMenu。從分享頁要到錢包只能靠眾籌 sheet 的「查看我的點數」，要登出只能手動輸入 /settings')
}, { page: S.aunt })

// =====================================================================
// C. 認捐數量制品項
// =====================================================================
S.reqs = []
S.aunt.on('request', r => { if (r.method() === 'POST' && /\/items\/[^/]+\/claims$/.test(r.url())) S.reqs.push({ url: r.url(), key: r.headers()['idempotency-key'], body: r.postData() }) })
await run.step('D-10', '開認捐 sheet：身分列、數量步進上下限、留言長度、可見度預設、匿名、Esc 放棄確認', async ({ note }) => {
  const p = S.aunt, dlg = p.getByRole('dialog')
  await card(p, '奶瓶組').getByRole('button', { name: '我來送' }).click()
  await expect(dlg.getByText('我來送「奶瓶組」')).toBeVisible()
  await expect(dlg.getByText(`以 ${S.auntName} 的身分送禮`)).toBeVisible()
  await expect(dlg.getByText('還缺 3 個')).toBeVisible()
  await expect(dlg.getByRole('button', { name: '減少' })).toBeDisabled()
  await dlg.getByRole('button', { name: '增加' }).click(); await dlg.getByRole('button', { name: '增加' }).click()
  await expect(dlg.locator('.g-step b')).toHaveText('3')
  await expect(dlg.getByRole('button', { name: '增加' })).toBeDisabled()
  await dlg.getByRole('button', { name: '減少' }).click()
  await expect(dlg.locator('.g-step b')).toHaveText('2')
  const vis = await dlg.getByLabel('誰看得到是你送的').inputValue()
  note(`可見度預設值：${vis}`)
  await dlg.getByLabel('留言（選填）').fill('字'.repeat(250))
  expect((await dlg.getByLabel('留言（選填）').inputValue()).length).toBe(200)
  note('留言欄 maxlength=200，貼 250 字只保留 200')
  // Esc：有輸入 → 要確認；先按「取消」(dismiss) 應仍開著
  let msg = ''
  p.once('dialog', d => { msg = d.message(); d.dismiss() })
  await p.keyboard.press('Escape')
  await expect(dlg).toBeVisible()
  note(`Esc 時跳出 confirm：「${msg}」，取消後 sheet 仍在，輸入保留`)
  await dlg.getByLabel('留言（選填）').fill('')
}, { page: S.aunt })

await run.step('D-11', '認捐 2 件：留言、匿名、可見度「僅好友」→ 成功畫面與期限提示', async ({ note }) => {
  const p = S.aunt, dlg = p.getByRole('dialog')
  await dlg.getByLabel('留言（選填）').fill('寶寶出生快樂，阿姨的心意')
  await dlg.getByLabel(/匿名認領/).check()
  await dlg.getByLabel('誰看得到是你送的').selectOption('friends')
  await dlg.getByRole('button', { name: '確定要送' }).click()
  await expect(done(p)).toBeVisible()
  await expect(p.getByText('你要送 奶瓶組 × 2')).toBeVisible()
  const dl = await p.locator('.js-done-deadline').innerText()
  note(`期限提示：${dl}`)
  await expect(p.getByRole('link', { name: '查看我的送禮' })).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '奶瓶組')).toContainText('2 / 3')
  await expect(card(p, '奶瓶組')).toContainText('你要送 2 件')
  await expect(card(p, '奶瓶組')).toContainText('匿名朋友')
  const row = sql(`SELECT qty, note, is_anonymous, visibility, to_char(expires_at,'MM-DD') FROM claims WHERE item_id='${L.main.byTitle['奶瓶組']}'`)
  note(`DB claims：${row}`)
}, { page: S.aunt })

await run.step('D-12', '另一個未登入訪客停在原頁不重新整理：進度自動更新、顯示「匿名朋友」（SSE 即時更新）', async ({ note }) => {
  const p = S.obs
  await expect(card(p, '奶瓶組')).toContainText('2 / 3', { timeout: 15000 })
  await expect(card(p, '奶瓶組')).toContainText('匿名朋友')
  await expect(card(p, '奶瓶組')).not.toContainText(S.auntName)
  await expect(p.getByText(/完成度 18%・0 \/ 5 項/)).toBeVisible()
  note('頁首完成度以「數量」加權：2 件 / 全部 11 件 = 18%，「0 / 5 項」是已全數認領的品項數；觀察者未重整即同步')
  note('匿名認捐在公開頁只顯示「匿名朋友」，不洩漏帳號名；連線標示：' + await p.locator('.g-foot').first().innerText())
}, { page: S.obs })

await run.step('D-13', '重新整理後仍是「修改我要送的」（身分靠 session cookie）', async () => {
  const p = S.aunt
  await p.reload(); await hyd(p)
  await expect(card(p, '奶瓶組').getByRole('button', { name: '修改我要送的' })).toBeVisible()
  await expect(card(p, '奶瓶組')).toContainText('你要送 2 件')
}, { page: S.aunt })

await run.step('D-14', '修改認捐：把 2 改成上限 3（可調到剩餘+自己的量），再改回 2；留言一併修改', async ({ note }) => {
  const p = S.aunt, dlg = p.getByRole('dialog')
  await card(p, '奶瓶組').getByRole('button', { name: '修改我要送的' }).click()
  await expect(dlg.getByText('修改我要送的「奶瓶組」')).toBeVisible()
  await expect(dlg.locator('.g-step b')).toHaveText('2')
  await dlg.getByRole('button', { name: '增加' }).click()
  await expect(dlg.getByRole('button', { name: '增加' })).toBeDisabled()
  await dlg.getByLabel('留言（選填）').fill('改成全包了')
  await dlg.getByRole('button', { name: '儲存修改' }).click()
  await expect(done(p)).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '奶瓶組')).toContainText('3 / 3')
  note('修改畫面沒有可見度／匿名選項（只能在 /me/claims 改可見度；匿名無法再改）')
  await card(p, '奶瓶組').getByRole('button', { name: '修改我要送的' }).click()
  await dlg.getByRole('button', { name: '減少' }).click()
  await dlg.getByRole('button', { name: '儲存修改' }).click()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '奶瓶組')).toContainText('2 / 3')
}, { page: S.aunt })

await run.step('D-15', '冪等（UI）：快速連點「確定要送」只建立一筆', async ({ note }) => {
  const p = S.aunt, dlg = p.getByRole('dialog')
  S.reqs.length = 0
  await card(p, '揹巾').getByRole('button', { name: '我來送' }).click()
  await dlg.getByRole('button', { name: '確定要送' }).dblclick()
  await expect(done(p)).toBeVisible()
  const n = S.reqs.filter(r => r.url.includes(L.main.byTitle['揹巾'])).length
  const c = await claimsOf(L.main.byTitle['揹巾'])
  note(`POST /claims 實際送出 ${n} 次；DB(筆數|總數量)=${c}`)
  expect(c).toBe('1|1')
  S.key = S.reqs.find(r => r.url.includes(L.main.byTitle['揹巾']))
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
}, { page: S.aunt })

await run.step('D-16', '冪等（API 重放）：同 key 同內容回放、同 key 不同內容衝突、新 key 重複認捐被擋', async ({ note }) => {
  const p = S.aunt, item = L.main.byTitle['揹巾']
  const body = JSON.parse(S.key.body)
  const r1 = await p.request.post(`${API}/api/v1/items/${item}/claims`, { data: body, headers: { 'Idempotency-Key': S.key.key, Origin: WEB } })
  note(`同 key 同 body：${r1.status()} replayed=${r1.headers()['idempotency-replayed']}`)
  expect(r1.headers()['idempotency-replayed']).toBeTruthy()
  const r2 = await p.request.post(`${API}/api/v1/items/${item}/claims`, { data: { ...body, qty: 2 }, headers: { 'Idempotency-Key': S.key.key, Origin: WEB } })
  const j2 = await r2.json().catch(() => ({}))
  note(`同 key 不同 body：${r2.status()} code=${j2.code}`)
  expect(r2.status()).toBeGreaterThanOrEqual(400)
  const r3 = await apiClaim(p.request, item, 1)
  const j3 = await r3.json().catch(() => ({}))
  note(`新 key 對已認捐品項再送：${r3.status()} code=${j3.code}`)
  expect(j3.code).toBe('CLAIM_ALREADY_EXISTS')
  const r4 = await p.request.post(`${API}/api/v1/items/${item}/claims`, { data: { qty: 1 }, headers: { Origin: WEB } })
  const j4 = await r4.json().catch(() => ({}))
  note(`缺 Idempotency-Key：${r4.status()} code=${j4.code}`)
  expect(await claimsOf(item)).toBe('1|1')
}, { page: S.aunt })

await run.step('D-17', '登入逾時：認捐送出時 session 已失效 → 導到登入頁（帶 redirect）→ 重新登入回原清單並認捐成功', async ({ note }) => {
  const p = S.aunt, dlg = p.getByRole('dialog')
  await card(p, '圍兜').getByRole('button', { name: '我來送' }).click()
  await expect(dlg).toBeVisible()
  sql(`DELETE FROM sessions WHERE user_id='${uidOf(S.auntEmail)}'`)
  note('平台外：直接刪除該帳號所有 sessions 模擬「登入逾時／在別處登出」')
  await dlg.getByRole('button', { name: '確定要送' }).click()
  await p.waitForURL(/\/login\?redirect=/, { timeout: 10000 })
  note('導向：' + p.url())
  await loginForm(p, S.auntEmail)
  await p.waitForURL(new RegExp(`/s/${L.main.slug}$`), { timeout: 15000 }); await hyd(p)
  await card(p, '圍兜').getByRole('button', { name: '我來送' }).click()
  await dlg.getByRole('button', { name: '確定要送' }).click()
  await expect(done(p)).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
}, { page: S.aunt })

// =====================================================================
// D. 搶最後一件（防超賣）
// =====================================================================
await run.step('D-18', '驚喜清單認捐：會員正常認捐；公開頁顯示進度但不顯示捐助者名字', async ({ note }) => {
  const u = await registerApi({ name: `驚喜朋友${T.slice(0, 3)}`, tag: 'don-sur' })
  const p = await ctxPage({ user: u })
  await p.goto(`/s/${L.surprise.slug}`); await hyd(p)
  await card(p, '生日蛋糕').getByRole('button', { name: '我來送' }).click()
  await p.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
  await expect(done(p)).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '生日蛋糕')).toContainText('1 / 2')
  note('捐助者自己看：' + (await card(p, '生日蛋糕').innerText()).replace(/\s+/g, ' '))
  const v = S.surpriseDonor
  await v.reload(); await hyd(v)
  const txt = (await card(v, '生日蛋糕').innerText()).replace(/\s+/g, ' ')
  note('未登入訪客看：' + txt)
  const api = await (await p.request.get(`${API}/api/v1/public/wishlists/${L.surprise.slug}`)).json()
  note(`公開 API：claimers_visible=${api.claimers_visible}、item.claimers=${JSON.stringify(api.items[0].claimers ?? null)}`)
  expect(txt).not.toContain(u.name)
  await p.context().close(); await u.api.dispose()
}, { page: undefined })

await run.step('D-19', '認捐 API 輸入驗證：數量 0／負數／小數／超大、留言過長、不合法可見度；私人清單的品項 id 被猜到', async ({ note }) => {
  const p = S.aunt, item = L.main.byTitle['揹巾']
  const out = []
  for (const [label, body] of [['qty=0', { qty: 0 }], ['qty=-1', { qty: -1 }], ['qty=1.5', { qty: 1.5 }], ['qty="a"', { qty: 'a' }], ['qty=10000', { qty: 10000 }], ['note 201 字', { qty: 1, note: 'x'.repeat(201) }], ['visibility=bogus', { qty: 1, visibility: 'bogus' }]]) {
    const r = await p.request.post(`${API}/api/v1/items/${item}/claims`, { data: body, headers: { 'Idempotency-Key': randomUUID(), Origin: WEB } })
    const j = await r.json().catch(() => ({}))
    out.push(`${label} → ${r.status()} ${j.code ?? ''}`)
    expect([400, 409, 422]).toContain(r.status())
  }
  note(out.join('；'))
  const r = await apiClaim(p.request, L.private.byTitle['秘密禮物'], 1)
  note(`對私人清單品項（猜到 id）認捐：${r.status()} ${(await r.json().catch(() => ({}))).code}`)
  expect([403, 404]).toContain(r.status())
  const r2 = await apiClaim(p.request, L.gone.byTitle['違規品'], 1)
  note(`對被下架清單品項認捐：${r2.status()} ${(await r2.json().catch(() => ({}))).code}`)
  expect(r2.status()).toBeGreaterThanOrEqual(400)
  const r3 = await apiClaim(p.request, randomUUID(), 1)
  note(`對不存在的品項認捐：${r3.status()}`)
}, { page: S.aunt })

await run.step('D-19b', '惡意顯示名稱／留言（HTML）：公開頁看到的是純文字不執行；同時對照「預設可見度＝私人」與「選公開」的顯示差異', async ({ note }) => {
  const evil = '<img src=x onerror="window.__xss=1">'
  const [uA, uB] = await Promise.all([registerApi({ name: evil, tag: 'don-xss' }), registerApi({ name: evil, tag: 'don-xss' })])
  const item = L.cf.byTitle['奶粉']
  const r1 = await apiClaim(uA.api, item, 1, randomUUID(), { note: '<script>window.__xss=2</script>' }) // 不帶 visibility：套帳號預設
  const r2 = await apiClaim(uB.api, item, 1, randomUUID(), { note: '<script>window.__xss=3</script>', visibility: 'public' })
  expect([r1.status(), r2.status()]).toEqual([201, 201])
  note('DB 兩筆認捐的可見度：' + sql(`SELECT string_agg(visibility::text, ',' ORDER BY created_at) FROM claims WHERE item_id='${item}'`))
  const v = await ctxPage()
  await v.goto(`/s/${L.cf.slug}`); await hyd(v)
  const txt = (await card(v, '奶粉').innerText()).replace(/\s+/g, ' ')
  note('公開頁（show_claimer_names=開）奶粉卡：' + txt)
  expect(await v.evaluate(() => window.__xss)).toBeUndefined()
  expect(txt).toContain(evil)           // 選「公開」的那位顯示為純文字
  expect(txt).toContain('匿名朋友')      // 沒選可見度（預設私人）的那位顯示為「匿名朋友」
  await v.context().close(); await Promise.all([uA.api.dispose(), uB.api.dispose()])
}, {})

S.chen = await registerApi({ name: `小陳${T.slice(0, 3)}`, tag: 'don-chen' })
S.lin = await registerApi({ name: `小林${T.slice(0, 3)}`, tag: 'don-lin' })
S.pc = await ctxPage()
S.pl = await ctxPage({ user: S.lin })
await run.step('D-20', '同事小陳（UI 登入）與小林（同時）搶「嬰兒帽」最後 1 件：只有一人成功', async ({ note }) => {
  const [a, b] = [S.pc, S.pl]
  // 小陳走 UI 登入（登入頁 → redirect 回清單）
  await a.goto(`/login?redirect=/s/${L.main.slug}`)
  await loginForm(a, S.chen.email)
  await a.waitForURL(new RegExp(`/s/${L.main.slug}$`), { timeout: 15000 }); await hyd(a)
  await b.goto(`/s/${L.main.slug}`); await hyd(b)
  for (const p of [a, b]) {
    await card(p, '嬰兒帽').getByRole('button', { name: '我來送' }).click()
    await expect(p.getByRole('dialog').getByText('還缺 1 個')).toBeVisible()
  }
  const outcome = p => Promise.race([done(p).waitFor({ timeout: 10000 }).then(() => 'ok'), p.locator('dialog .g-banner.err').waitFor({ timeout: 10000 }).then(async () => 'lose:' + await p.locator('dialog .g-banner.err').innerText().catch(() => '?'))])
  const res = await Promise.all([a, b].map(async p => { const o = outcome(p); await p.getByRole('dialog').getByRole('button', { name: '確定要送' }).click(); return o }))
  const wins = res.map(r => r === 'ok')
  S.raceWins = wins; S.raceMsg = res.find(r => r.startsWith('lose:'))
  note(`小陳成功=${wins[0]}、小林成功=${wins[1]}；DB(筆數|總數量)=${await claimsOf(L.main.byTitle['嬰兒帽'])}、qty_claimed=${qtyClaimed(L.main.byTitle['嬰兒帽'])}`)
  expect(wins.filter(Boolean).length).toBe(1)
  expect(await claimsOf(L.main.byTitle['嬰兒帽'])).toBe('1|1')
  expect(qtyClaimed(L.main.byTitle['嬰兒帽'])).toBe('1')
}, { page: S.pc })

await run.step('D-21', '搶輸的人：看到什麼訊息、多久後 sheet 自動關閉、關閉後頁面還留不留提示；贏家看到成功', async ({ note }) => {
  const [win, lose] = S.raceWins[0] ? [S.pc, S.pl] : [S.pl, S.pc]
  note(`輸家 sheet 內橫幅：${S.raceMsg}`)
  const t0 = Date.now()
  await expect(lose.getByRole('dialog')).toBeHidden({ timeout: 8000 })
  note(`輸家 sheet 已自動關閉（ClaimSheet.vue 以 1200ms 計時關閉；腳本偵測時已關）`)
  const left = await lose.getByText(/剛剛被別人認領完了|抱歉/).count()
  note(`sheet 關閉後頁面上殘留的提示文字數：${left}`)
  await expect(card(lose, '嬰兒帽')).toContainText('1 / 1')
  await expect(card(lose, '嬰兒帽').getByText('已全數認領')).toBeVisible()
  await expect(done(win)).toBeVisible()
  await win.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(win, '嬰兒帽')).toContainText('要送')
  S.raceLoser = lose
  if (!left) throw new Gap('搶輸時訊息只在 sheet 內閃現約 1.2 秒就隨 sheet 關閉，頁面沒有留下任何 toast 或說明，使用者可能只看到卡片突然變「已全數認領」')
}, { page: undefined })

await run.step('D-22', '部分搶購：「尿布」剩 3，兩人各要 2 → 輸家被告知「只剩 1 個」、上限調為 1，改送 1 成功', async ({ note }) => {
  const [a, b] = [S.pc, S.pl]
  for (const p of [a, b]) {
    await p.reload(); await hyd(p)
    await card(p, '尿布').getByRole('button', { name: '我來送' }).click()
    await p.getByRole('dialog').getByRole('button', { name: '增加' }).click()
    await expect(p.getByRole('dialog').locator('.g-step b')).toHaveText('2')
  }
  await Promise.all([a, b].map(p => p.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()))
  await sleep(2500)
  const wins = await Promise.all([a, b].map(p => done(p).isVisible()))
  expect(wins.filter(Boolean).length).toBe(1)
  const lose = wins[0] ? b : a
  const msg = await lose.getByRole('dialog').locator('.g-banner.err').innerText()
  note('輸家訊息：' + msg)
  expect(msg).toContain('只剩 1 個')
  await expect(lose.getByRole('dialog').locator('.g-step b')).toHaveText('1')
  await expect(lose.getByRole('dialog').getByRole('button', { name: '增加' })).toBeDisabled()
  await lose.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
  await expect(done(lose)).toBeVisible()
  expect(qtyClaimed(L.main.byTitle['尿布'])).toBe('3')
  await a.screenshot({ path: new URL('../out/role-flows/donor/D-22-a.png', import.meta.url).pathname }).catch(() => {})
}, { page: S.pl })

await run.step('D-23', 'API 層併發：6 個新會員同時搶 1 件 → 恰 1 個 201、其餘 409，qty_claimed=1', async ({ note }) => {
  const users = await Promise.all(Array.from({ length: 6 }, (_, i) => registerApi({ tag: `don-r${i}` }).catch(e => { throw e })))
  const item = L.race.byTitle['API 搶購品']
  const rs = await Promise.all(users.map(u => apiClaim(u.api, item, 1)))
  const codes = await Promise.all(rs.map(async r => r.status() + ((await r.json().catch(() => ({}))).code ?? '')))
  note(`回應：${codes.join('、')}；DB(筆數|總數量)=${await claimsOf(item)}、qty_claimed=${qtyClaimed(item)}`)
  expect(rs.filter(r => r.status() === 201).length).toBe(1)
  expect(qtyClaimed(item)).toBe('1')
  await Promise.all(users.map(u => u.api.dispose()))
}, {})

await run.step('D-23b', '同一人開兩個分頁同時送（不同 Idempotency-Key）：只成立一筆，另一筆被擋', async ({ note }) => {
  const item = L.race.byTitle['同人雙開']
  const rs = await Promise.all([apiClaim(S.chen.api, item, 1), apiClaim(S.chen.api, item, 1), apiClaim(S.chen.api, item, 1)])
  const codes = await Promise.all(rs.map(async r => r.status() + ((await r.json().catch(() => ({}))).code ?? '')))
  note(`回應：${codes.join('、')}；DB(筆數|總數量)=${await claimsOf(item)}、qty_claimed=${qtyClaimed(item)}`)
  expect(rs.filter(r => r.status() === 201).length).toBe(1)
  expect(await claimsOf(item)).toBe('1|1')
}, {})

// =====================================================================
// E. 我的送禮 /me/claims（阿姨，手機）
// =====================================================================
await run.step('D-24', '/me/claims：分組、狀態徽章、期限、可見度選單；改可見度後重新整理仍保留', async ({ note }) => {
  const p = S.aunt
  await p.getByRole('link', { name: '我的送禮' }).first().click()
  await expect(p).toHaveURL(/\/me\/claims$/)
  await expect(p.getByRole('link', { name: L.main.title })).toBeVisible()
  await expect(p.getByText('奶瓶組 × 2')).toBeVisible()
  await expect(p.getByText('揹巾 × 1')).toBeVisible()
  await expect(p.getByText('圍兜 × 1')).toBeVisible()
  await expect(p.getByText(/請在 .* 前標記已購買/).first()).toBeVisible()
  await noHScroll(p)
  const row = p.locator('.g-item', { hasText: '奶瓶組' })
  const before = await row.getByLabel('誰看得到').inputValue()
  await Promise.all([p.waitForResponse(r => r.request().method() === 'PATCH' && /\/claims\//.test(r.url())), row.getByLabel('誰看得到').selectOption('private')])
  await expect(row.getByLabel('誰看得到')).toHaveValue('private')
  await p.reload(); await hyd(p)
  await expect(p.locator('.g-item', { hasText: '奶瓶組' }).getByLabel('誰看得到')).toHaveValue('private')
  note(`可見度 ${before} → private，重整後保留；/me/claims 頂端身分卡：` + await p.locator('.g-card').first().innerText())
}, { page: S.aunt })

await run.step('D-25', '延長購買期限 7 天（一次為限）', async ({ note }) => {
  const p = S.aunt
  const row = p.locator('.g-item', { hasText: '奶瓶組' })
  const before = await row.locator('.js-deadline').innerText()
  await row.getByRole('button', { name: '延長 7 天' }).click()
  await expect(row.getByRole('button', { name: '延長 7 天' })).toHaveCount(0)
  const after = await row.locator('.js-deadline').innerText()
  note(`期限：${before} → ${after}`)
  expect(after).not.toBe(before)
}, { page: S.aunt })

await run.step('D-26', '標記已購買 → 已送達（揹巾）：徽章更新、期限與延長鈕消失、已送達後不能再取消', async ({ note }) => {
  const p = S.aunt
  const row = () => p.locator('.g-item', { hasText: '揹巾' })
  await row().getByRole('button', { name: '標記已購買' }).click()
  await expect(row().locator('.g-badge')).toHaveText('已購買')
  await expect(row().locator('.js-deadline')).toHaveCount(0)
  await row().getByRole('button', { name: '標記已送達' }).click()
  await expect(row().locator('.g-badge')).toHaveText('已送達')
  await expect(row().getByRole('button', { name: '取消', exact: true })).toHaveCount(0)
  note('已送達後沒有任何動作鈕')
}, { page: S.aunt })

await run.step('D-27', '取消認捐（confirm 對話框）→ 移到「過去的紀錄」，數量釋出回分享頁', async ({ note }) => {
  const p = S.aunt
  let msg = ''
  p.once('dialog', d => { msg = d.message(); d.accept() })
  await p.locator('.g-item', { hasText: '奶瓶組' }).getByRole('button', { name: '取消', exact: true }).click()
  await expect(p.getByText(/過去的紀錄（\d+）/)).toBeVisible()
  note(`confirm 文字：「${msg}」`)
  await p.goto(`/s/${L.main.slug}`); await hyd(p)
  await expect(card(p, '奶瓶組')).toContainText('0 / 3')
  await expect(card(p, '奶瓶組').getByRole('button', { name: '我來送' })).toBeVisible()
  await expect(S.obs.locator('li.g-card', { hasText: '奶瓶組' })).toContainText('0 / 3', { timeout: 15000 })
  note('觀察者（未重整）也同步看到 0 / 3')
}, { page: S.aunt })

await run.step('D-27b', '取消後再認捐同一品項（已取消的紀錄不擋重新認捐）', async ({ note }) => {
  const p = S.aunt
  await card(p, '奶瓶組').getByRole('button', { name: '我來送' }).click()
  await p.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
  await expect(done(p)).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '奶瓶組')).toContainText('1 / 3')
  note('DB 該使用者對奶瓶組的紀錄：' + sql(`SELECT string_agg(status::text||':'||qty, ',' ORDER BY created_at) FROM claims WHERE item_id='${L.main.byTitle['奶瓶組']}' AND user_id='${uidOf(S.auntEmail)}'`))
}, { page: S.aunt })

await run.step('D-28', '購買期限逾期：把「圍兜」認捐期限改成過去（時間快轉），等背景工作釋放；先確認逾期前狀態', async ({ note }) => {
  const id = L.main.byTitle['圍兜']
  S.expClaim = sql(`UPDATE claims SET expires_at = now() - interval '1 minute' WHERE item_id='${id}' AND user_id='${uidOf(S.auntEmail)}' AND status='reserved' RETURNING id`).split('\n')[0]
  expect(S.expClaim).toBeTruthy()
  S.expAt = Date.now()
  note(`平台外：UPDATE claims SET expires_at = now()-1min（claim ${S.expClaim}）；背景 expire_due 每 5 分鐘跑一次，結果於 D-29 檢查（本腳本尾端）`)
  note(`改時間後、tick 前 qty_claimed=${qtyClaimed(id)}（仍佔用）`)
}, {})

// =====================================================================
// F. 清單狀態與存取限制
// =====================================================================
await run.step('D-30', '已結束清單：先有認捐 → 受捐者結束清單 → 分享頁橫幅、無送禮鈕；我的送禮仍可標記已購買但不可延長', async ({ note }) => {
  const p = S.aunt
  const r = await apiClaim(p.request, L.closed.byTitle['積木'], 1)
  expect(r.status()).toBe(201)
  await setStatus(owner, L.closed, 'closed')
  note('平台外：先以 API 為阿姨建一筆認捐，再由受捐者(API)結束清單')
  await p.goto(`/s/${L.closed.slug}`); await hyd(p)
  await expect(p.getByText('這份清單已結束').first()).toBeVisible()
  await expect(p.getByRole('button', { name: /我來送|修改我要送的/ })).toHaveCount(0)
  await expect(p.getByRole('link', { name: '登入後來送' })).toHaveCount(0)
  const r2 = await apiClaim(p.request, L.closed.byTitle['積木'], 1)
  const j2 = await r2.json().catch(() => ({}))
  note(`對已結束清單直接打 API：${r2.status()} ${j2.code}`)
  expect(r2.status()).toBeGreaterThanOrEqual(400)
  await p.goto('/me/claims'); await hyd(p)
  const row = p.locator('.g-item', { hasText: '積木' })
  await expect(row.getByText('清單已結束：仍可標記已購買')).toBeVisible()
  await expect(row.getByRole('button', { name: '延長 7 天' })).toHaveCount(0)
  await row.getByRole('button', { name: '標記已購買' }).click()
  await expect(row.locator('.g-badge')).toHaveText('已購買')
}, { page: S.aunt })

await run.step('D-31', '私人清單與不存在的 slug：404 頁（頁面與 HTTP 狀態）', async ({ note }) => {
  const p = S.aunt
  const hs = await Promise.all([L.private.slug, 'zzzzzzzzzz'].map(async s => (await p.request.get(`/s/${s}`)).status()))
  note(`HTTP 狀態（私人 / 不存在）：${hs.join(' / ')}`)
  expect(hs).toEqual([404, 404])
  await p.goto(`/s/${L.private.slug}`); await hyd(p)
  await expect(p.getByRole('heading', { name: '找不到這份清單' })).toBeVisible()
  await expect(p.getByText('秘密禮物')).toHaveCount(0)
  await expect(p.getByRole('link', { name: '回首頁' })).toBeVisible()
  note('私人清單對非擁有者與「不存在」回應相同（不洩漏存在）')
}, { page: S.aunt })

await run.step('D-32', '被下架清單：410 頁、不洩漏標題、HTTP 410', async ({ note }) => {
  const p = S.aunt
  const st = (await p.request.get(`/s/${L.gone.slug}`)).status()
  note('HTTP 狀態：' + st)
  await p.goto(`/s/${L.gone.slug}`); await hyd(p)
  await expect(p.getByRole('heading', { name: '此清單已被下架' })).toBeVisible()
  await expect(p.getByText(L.gone.title)).toHaveCount(0)
  expect(st).toBe(410)
}, { page: S.aunt })

S.flower = await registerApi({ name: `小花${T.slice(0, 3)}`, tag: 'don-flower' })
await run.step('D-33', '好友限定清單・未登入：看到「請先登入」，登入後（非好友）回到原頁看到「沒有權限」與加好友鈕', async ({ note }) => {
  const p = await ctxPage()
  S.pf = p
  await p.goto(`/s/${L.friends.slug}`); await hyd(p)
  await expect(p.getByRole('heading', { name: '請先登入' })).toBeVisible()
  await expect(p.getByText('安撫奶嘴')).toHaveCount(0)
  await p.getByRole('link', { name: '登入後查看' }).click()
  await expect(p).toHaveURL(new RegExp(`/login\\?redirect=(%2F|/)s(%2F|/)${L.friends.slug}$`))
  await loginForm(p, S.flower.email)
  await p.waitForURL(new RegExp(`/s/${L.friends.slug}$`), { timeout: 15000 }); await hyd(p)
  await expect(p.getByRole('heading', { name: '你沒有權限查看這份清單' })).toBeVisible()
  await expect(p.getByRole('button', { name: /送出好友邀請給/ })).toBeVisible()
  note('403 頁顯示擁有者名稱：' + await p.locator('.g-center p.g-mute').first().innerText())
  note('SSR HTTP 狀態：' + (await p.request.get(`/s/${L.friends.slug}`)).status())
}, { page: undefined })

await run.step('D-34', '送出好友邀請 → 受捐者接受（API）→ 重新整理後看得到限定清單並能開認捐 sheet', async ({ note }) => {
  const p = S.pf
  await p.getByRole('button', { name: /送出好友邀請給/ }).click()
  await expect(p.getByText('已送出好友邀請')).toBeVisible()
  const ownerMe = await (await owner.api.get('friends')).json()
  const inc = (ownerMe.incoming ?? ownerMe.requests?.incoming ?? [])[0]
  note('受捐者端 /friends 回應鍵：' + Object.keys(ownerMe).join(','))
  const rid = inc?.id ?? inc?.request?.id
  if (!rid) throw new Error('找不到受捐者的 incoming 好友邀請：' + JSON.stringify(ownerMe).slice(0, 300))
  await must(await owner.api.post(`friends/requests/${rid}/accept`), '受捐者接受（前置）')
  note('平台外：受捐者以 API 接受邀請')
  await p.reload(); await hyd(p)
  await expect(p.getByRole('heading', { level: 1, name: L.friends.title })).toBeVisible()
  await expect(card(p, '安撫奶嘴')).toBeVisible()
  await card(p, '安撫奶嘴').getByRole('button', { name: '我來送' }).click()
  await expect(p.getByRole('dialog').getByText('我來送「安撫奶嘴」')).toBeVisible()
  await p.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
  await expect(done(p)).toBeVisible()
  note('好友限定清單的認捐成功（受限路徑也可認捐）')
}, { page: undefined })

S.grass = await registerApi({ name: `小草${T.slice(0, 3)}`, tag: 'don-grass' })
await run.step('D-35', '好友邀請連結 /invite/[token]：未登入→登入→成為好友→跳到該清單', async ({ note }) => {
  const r = await must(await owner.api.post('friends/invites', { data: { wishlist_id: L.friends.id } }), '產生邀請（受捐者前置）')
  const inv = await r.json()
  const url = inv.url ?? inv.invite_url ?? (inv.token ? `${WEB}/invite/${inv.token}` : null)
  note('邀請回應鍵：' + Object.keys(inv).join(',') + '；網址：' + url)
  const path = new URL(url).pathname
  const p = await ctxPage()
  await p.goto(path); await hyd(p)
  await expect(p.getByText(/邀請你成為好友，並查看清單/)).toBeVisible()
  await p.getByRole('link', { name: '登入後成為好友' }).click()
  await expect(p).toHaveURL(/\/login\?redirect=/)
  await loginForm(p, S.grass.email)
  await p.waitForURL(u => u.pathname === path, { timeout: 15000 }); await hyd(p)
  await p.getByRole('button', { name: '成為好友' }).click()
  await p.waitForURL(new RegExp(`/s/${L.friends.slug}$`), { timeout: 15000 })
  await expect(card(p, '安撫奶嘴')).toBeVisible()
  S.invitePath = path
  S.pg = p
}, { page: undefined })

await run.step('D-36', '失效／亂填的邀請連結：顯示「已失效或被撤銷」', async () => {
  const p = await ctxPage({ user: S.grass })
  await p.goto('/invite/not-a-real-token'); await hyd(p)
  await expect(p.getByText('這個邀請連結已失效或被撤銷。')).toBeVisible()
  await p.context().close()
}, {})

await run.step('D-37', '檢舉清單：訪客（免登入）選原因送出 → 「已收到檢舉」', async ({ note }) => {
  const p = S.obs
  await p.goto(`/s/${L.main.slug}`); await hyd(p)
  await p.getByRole('button', { name: '檢舉此清單' }).click()
  const dlg = p.getByRole('dialog')
  await expect(dlg.getByRole('button', { name: '送出檢舉' })).toBeEnabled()
  await dlg.getByLabel('疑似詐騙').check()
  await dlg.getByLabel(/補充說明/).fill('QA 流程測試檢舉，請忽略')
  await dlg.getByRole('button', { name: '送出檢舉' }).click()
  await expect(p.getByText('已收到檢舉，我們會盡快處理')).toBeVisible()
  note('DB content_reports：' + sql(`SELECT reason, status FROM content_reports WHERE wishlist_id='${L.main.id}'`))
  note('本環境 Turnstile 未設 site key，前端送 dev-bypass；真實人機驗證見 D-91')
}, { page: S.obs })

await run.step('D-38', '離線／斷線：顯示離線橫幅、送禮鈕停用，恢復後可用', async ({ note }) => {
  const p = S.aunt
  await p.goto(`/s/${L.main.slug}`); await hyd(p)
  await p.context().setOffline(true)
  await p.evaluate(() => window.dispatchEvent(new Event('offline')))
  await expect(p.getByText('目前離線，顯示的是上次載入的內容')).toBeVisible()
  await expect(card(p, '嬰兒帽')).toBeVisible()
  const dis = await p.locator('li.g-card .g-btn.js-claim').first().isDisabled().catch(() => null)
  note('離線時第一個送禮鈕 disabled=' + dis)
  await p.context().setOffline(false)
  await p.evaluate(() => window.dispatchEvent(new Event('online')))
  await expect(p.getByText('目前離線')).toBeHidden()
}, { page: S.aunt })

await run.step('D-39', '切換語系（English）：分享頁文案改變並保留登入', async ({ note }) => {
  const p = S.aunt
  await p.getByLabel(/語言|Language/).selectOption({ index: 1 })
  await sleep(800)
  note('切換後頁面：' + (await p.locator('li.g-card').first().innerText()).replace(/\s+/g, ' ').slice(0, 120))
  await p.getByLabel(/語言|Language/).selectOption('zh-Hant')
  await sleep(500)
}, { page: S.aunt })

// =====================================================================
// G. 探索 / 靈感 / 品牌
// =====================================================================
await run.step('D-40', '/explore：列出公開且有品項的清單，不含好友／私人／草稿／空清單／下架；點卡片進分享頁', async ({ note }) => {
  const p = S.obs
  await p.goto('/explore'); await hyd(p)
  await expect(p.getByRole('heading', { name: '探索公開清單' })).toBeVisible()
  const seen = t => p.getByRole('link', { name: t }).count()
  const res = {}
  for (const k of ['main', 'surprise', 'friends', 'private', 'closed', 'gone', 'draft', 'empty']) res[k] = await seen(L[k].title)
  note('出現次數：' + JSON.stringify(res))
  // 更多頁：載入更多
  const more = p.getByRole('button', { name: '載入更多' })
  for (let i = 0; i < 6 && await more.isVisible().catch(() => false) && !res.main; i++) { await more.click(); await sleep(600); res.main = await seen(L.main.title) }
  note('（若清單多於一頁會按「載入更多」）最終 main=' + res.main)
  expect(res.main).toBe(1)
  for (const k of ['friends', 'private', 'gone', 'draft', 'empty']) expect(res[k]).toBe(0)
  note(`驚喜清單 ${res.surprise ? '會' : '不會'}出現在探索頁；已結束清單 ${res.closed ? '會' : '不會'}出現`)
  await p.getByRole('link', { name: L.main.title }).click()
  await expect(p).toHaveURL(new RegExp(`/s/${L.main.slug}$`))
}, { page: S.obs })

await run.step('D-41', '/inspire 選物靈感：篩選器、空狀態或商品卡', async ({ note }) => {
  const p = await ctxPage()
  await p.goto('/inspire'); await hyd(p)
  await expect(p.getByRole('heading', { name: '選物靈感' })).toBeVisible()
  const empty = await p.getByText('精選商品即將上架').count()
  note('目前 catalog 商品數（API）：' + sql('SELECT count(*) FROM catalog_products WHERE active'))
  if (!empty) { await p.getByRole('button', { name: '伴侶' }).click().catch(() => {}); await sleep(500) }
  note(empty ? '無上架商品 → 顯示「精選商品即將上架」空狀態' : '有商品，已試篩選')
  S.insp = p
}, { page: undefined })

await run.step('D-42', '/brands 品牌館與品牌頁（營運先種一個品牌與商品作前置）', async ({ note }) => {
  const slug = `qa${T.slice(0, 6)}`
  const b = await staff.api.post('admin/brands', { data: { slug, name: `QA 品牌 ${T}`, description: '捐助者流程測試品牌', active: true } })
  note(`前置：營運建品牌 ${b.status()} ${b.ok() ? '' : (await b.text()).slice(0, 200)}`)
  const bj = b.ok() ? await b.json() : null
  if (bj) {
    const pr = await staff.api.post('admin/catalog/products', { data: { ref: `qa${T}`, brand_id: bj.id ?? bj.brand?.id, title: `QA 商品 ${T}`, price_amount: 1280, product_url: 'https://example.com/p', recipients: ['baby'], occasions: ['birthday'], active: true } })
    note(`前置：營運建商品 ${pr.status()} ${pr.ok() ? '' : (await pr.text()).slice(0, 200)}`)
  }
  const p = await ctxPage()
  await p.goto('/brands'); await hyd(p)
  await expect(p.getByRole('heading', { name: '品牌館' })).toBeVisible()
  if (bj) {
    await sleep(500)
    await expect(p.getByRole('link', { name: `QA 品牌 ${T}` })).toBeVisible({ timeout: 70000 }) // 公開回應 max-age=60
    await p.getByRole('link', { name: `QA 品牌 ${T}` }).click()
    await expect(p.getByRole('heading', { name: `QA 品牌 ${T}` })).toBeVisible()
    note('品牌頁商品：' + (await p.locator('main').innerText()).replace(/\s+/g, ' ').slice(0, 160))
  }
  await p.goto('/brands/no-such-brand'); await hyd(p)
  await expect(p.getByRole('heading', { name: '找不到這個品牌' })).toBeVisible()
  S.brandPage = p
}, { page: undefined })

await run.step('D-43', '首頁（未登入）：主要入口可達 /explore；頁首導覽四個連結', async () => {
  const p = await ctxPage()
  await p.goto('/'); await hyd(p)
  await expect(p.getByRole('link', { name: '探索公開清單' })).toBeVisible()
  await p.getByRole('link', { name: '探索公開清單' }).click()
  await expect(p).toHaveURL(/\/explore$/)
  for (const n of ['探索', '靈感', '品牌館', '我的送禮']) await expect(p.locator('header').getByRole('link', { name: n })).toBeVisible()
  await p.context().close()
}, {})

// =====================================================================
// H. 心願眾籌・儲值・錢包・退款（阿明，桌面）
// =====================================================================
S.ming = await registerApi({ name: `阿明${T.slice(0, 3)}`, tag: 'don-ming' })
S.pm = await ctxPage({ user: S.ming })
await run.step('D-50', '餘額 0 的人按「心願眾籌」：看到餘額、快速金額、不足提示；送出被擋並引導「查看我的點數」', async ({ note }) => {
  const p = S.pm, dlg = p.getByRole('dialog')
  await p.goto(`/s/${L.cf.slug}`); await hyd(p)
  await expect(card(p, '嬰兒推車')).toContainText('0 / 300 點')
  await expect(card(p, '奶粉').getByRole('button', { name: '我來送' })).toBeVisible()
  await card(p, '嬰兒推車').getByRole('button', { name: '心願眾籌' }).click()
  await expect(dlg.getByText('心願眾籌「嬰兒推車」')).toBeVisible()
  await expect(dlg.getByText('還差 300 點達標')).toBeVisible()
  await expect(dlg.getByText('目前餘額')).toBeVisible()
  await dlg.getByLabel('贊助點數').fill('100')
  await expect(dlg.getByText(/餘額不足，還缺 100 點/)).toBeVisible()
  await dlg.getByRole('button', { name: /確認贊助/ }).click()
  await expect(dlg.getByRole('alert')).toBeVisible()
  note('送出後訊息：' + await dlg.getByRole('alert').innerText())
  await expect(dlg.getByRole('link', { name: '查看我的點數' })).toBeVisible()
}, { page: S.pm })

await run.step('D-51', '/me/wallet（新帳號）：餘額 0、無贊助、無可退現金說明、儲值入口', async ({ note }) => {
  const p = S.pm
  await p.getByRole('dialog').getByRole('link', { name: '查看我的點數' }).click()
  await expect(p).toHaveURL(/\/me\/wallet$/)
  await expect(p.getByText('可用點數（1 點 = NT$1）')).toBeVisible()
  await expect(p.getByText('還沒有贊助紀錄')).toBeVisible()
  await expect(p.getByTestId('refund-none')).toBeVisible()
  await expect(p.getByRole('link', { name: '儲值' })).toBeVisible()
  note('頂部：' + (await p.locator('header').first().innerText()).replace(/\s+/g, ' '))
}, { page: S.pm })

await run.step('D-52', '/me/topup 前端驗證：金額低於 100、超過 50,000、非整數', async ({ note }) => {
  const p = S.pm
  await p.getByRole('link', { name: '儲值' }).click()
  await expect(p).toHaveURL(/\/me\/topup$/)
  await expect(p.getByRole('heading', { name: '儲值', exact: true }).or(p.locator('h1'))).toBeVisible()
  for (const v of ['50', '60000', '12.5']) {
    await p.getByLabel('自訂金額').fill(v)
    await p.getByRole('button', { name: '前往付款' }).click()
    const e = await p.getByRole('alert').first().innerText()
    note(`金額 ${v} → ${e}`)
    await expect(p.getByRole('alert').first()).toBeVisible()
  }
}, { page: S.pm })

await run.step('D-53', '儲值 NT$1,000（信用卡）：建單 → 以隱藏表單 POST 導向綠界（攔截），DB 有待付款單', async ({ note }) => {
  const p = S.pm
  await p.route(ECPAY, r => r.fulfill({ status: 200, contentType: 'text/html', body: '<h1>stub ecpay</h1>' }))
  await p.getByRole('button', { name: '1,000', exact: true }).click()
  await expect(p.getByTestId('summary')).toContainText('儲值 NT$ 1,000，入帳 1,000 點')
  const nav = p.waitForRequest(r => r.url().startsWith('https://payment-stage.ecpay.com.tw') && r.method() === 'POST')
  await p.getByRole('button', { name: '前往付款' }).click()
  const req = await nav
  const f = Object.fromEntries(new URLSearchParams(req.postData() ?? ''))
  note(`導向 ${req.url()}；欄位：${Object.keys(f).join(',')}；TotalAmount=${f.TotalAmount}；ClientBackURL=${f.ClientBackURL ?? '(無)'}；OrderResultURL=${f.OrderResultURL ?? '(無)'}`)
  S.tradeNo = f.MerchantTradeNo
  expect(f.TotalAmount).toBe('1000')
  expect(sql(`SELECT status FROM topups WHERE merchant_trade_no='${q(S.tradeNo)}'`)).toBe('pending')
}, { page: S.pm })

await run.step('D-54', '付款完成（綠界通知，替身）→ 回到 /me/topup 與錢包：已入帳、餘額 1,000', async ({ note }) => {
  const p = S.pm
  const r = await ecpayPaid(S.tradeNo)
  note(`平台外：ecpayPaid(${S.tradeNo}) → ${r.status} ${r.text}`)
  await p.unroute(ECPAY)
  await p.goto('/me/topup'); await hyd(p)
  await expect(p.locator('article[data-status="paid"]')).toBeVisible()
  await expect(p.getByText('目前餘額 1,000 點', { exact: true })).toBeVisible()
  await p.goto('/me/wallet'); await hyd(p)
  await expect(p.locator('.c-big').first()).toHaveText('1,000')
  await expect(p.getByTestId('refund-entry')).toBeVisible()
  note('錢包：可退現金 ' + await p.getByText(/可退現金 [\d,]+ 點/).first().innerText())
  note('信件：' + (await listMail(S.ming.email)).map(m => m.subject).join('｜'))
}, { page: S.pm })

await run.step('D-55', '儲值選超商代碼 → 取號中（待綠界 /info 通知）→ 取消待付款訂單', async ({ note }) => {
  const p = S.pm
  await p.route(ECPAY, r => r.fulfill({ status: 200, contentType: 'text/html', body: '<h1>stub ecpay</h1>' }))
  await p.goto('/me/topup'); await hyd(p)
  await p.getByRole('button', { name: '500', exact: true }).click()
  await p.getByRole('radio', { name: /超商代碼/ }).check()
  await Promise.all([p.waitForRequest(r => r.url().startsWith('https://payment-stage.ecpay.com.tw')), p.getByRole('button', { name: '前往付款' }).click()])
  await p.unroute(ECPAY)
  await p.goto('/me/topup'); await hyd(p)
  const art = p.locator('article[data-status="pending"]').first()
  await expect(art).toBeVisible()
  note('待付款單文字：' + (await art.innerText()).replace(/\s+/g, ' '))
  await art.getByTestId('cancel-topup').click()
  await expect(p.getByText('若你已經付款，款項仍會入帳')).toBeVisible()
  await p.getByRole('dialog').getByRole('button', { name: '取消訂單' }).click()
  await expect(p.locator('article[data-status="pending"]')).toHaveCount(0, { timeout: 8000 })
  note('取消後該單狀態：' + sql(`SELECT status FROM topups WHERE wallet_id=(SELECT id FROM point_wallets WHERE user_id='${uidOf(S.ming.email)}') ORDER BY created_at DESC LIMIT 1`))
}, { page: S.pm })

await run.step('D-56', '贊助 100 點（留言、可見度）→ 成功畫面、卡片進度與贊助者名單、錢包餘額更新', async ({ note }) => {
  const p = S.pm, dlg = p.getByRole('dialog')
  await p.goto(`/s/${L.cf.slug}`); await hyd(p)
  await card(p, '嬰兒推車').getByRole('button', { name: '心願眾籌' }).click()
  await expect(dlg.getByText(/目前餘額 1,000 點/)).toBeVisible()
  await dlg.getByLabel('贊助點數').fill('100')
  await dlg.getByLabel('留言（選填）').fill('推車我出一點')
  await dlg.getByLabel('這筆贊助誰看得到').selectOption('public')
  await dlg.getByRole('button', { name: '確認贊助 100 點' }).click()
  await expect(p.getByRole('heading', { name: '✓ 感謝你的贊助！' })).toBeVisible()
  await expect(p.getByText('錢包餘額剩 900 點')).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '嬰兒推車')).toContainText('100 / 300 點')
  await expect(card(p, '嬰兒推車')).toContainText(S.ming.name)
}, { page: S.pm })

await run.step('D-57', '贊助超過剩餘：他人先贊助 150 後，自己開著的 sheet 即時更新剩餘；超額被原生驗證擋、API 超額回 CROWDFUND_TARGET_EXCEEDED；改送 50 達標', async ({ note }) => {
  const p = S.pm, dlg = p.getByRole('dialog')
  await card(p, '嬰兒推車').getByRole('button', { name: '心願眾籌' }).click()
  await expect(dlg.getByText('還差 200 點達標')).toBeVisible()
  await dlg.getByLabel('贊助點數').fill('250')
  await dlg.getByRole('button', { name: /確認贊助/ }).click()
  const vm = await dlg.getByLabel('贊助點數').evaluate(el => el.validationMessage)
  note(`超額 250 點：送出被瀏覽器原生驗證擋下（max=剩餘 200），沒有頁內 alert，原生提示「${vm}」`)
  await expect(dlg.getByRole('alert')).toHaveCount(0)
  // 小陳拿到平台發的點數後贊助 150（前置：營運人工發點）
  const chenId = (await (await S.chen.api.get('me')).json()).id
  await must(await staff.api.post('admin/wallets/grants', { data: { user_id: chenId, points: 500, reason: 'QA 捐助者流程' }, headers: { 'Idempotency-Key': randomUUID() } }), '營運發點（前置）')
  const cr = await S.chen.api.post(`items/${L.cf.byTitle['嬰兒推車']}/contributions`, { data: { points: 150, is_anonymous: true }, headers: { 'Idempotency-Key': randomUUID() } })
  expect(cr.status()).toBe(201)
  await expect(dlg.getByText('還差 50 點達標')).toBeVisible({ timeout: 15000 })
  note('他人贊助後，已開啟的 sheet 在未重開下即時改為「還差 50 點達標」')
  await dlg.getByLabel('贊助點數').fill('100')
  await dlg.getByRole('button', { name: /確認贊助/ }).click()
  note('改填 100：原生驗證訊息 ' + await dlg.getByLabel('贊助點數').evaluate(el => el.validationMessage))
  const ar = await S.ming.api.post(`items/${L.cf.byTitle['嬰兒推車']}/contributions`, { data: { points: 100 }, headers: { 'Idempotency-Key': randomUUID() } })
  const aj = await ar.json().catch(() => ({}))
  note(`API 直接超額贊助 100：${ar.status()} code=${aj.code} remaining=${aj.remaining ?? aj.remaining_points ?? ''}`)
  expect(ar.status()).toBeGreaterThanOrEqual(400)
  await dlg.getByLabel('贊助點數').fill('50')
  await dlg.getByRole('button', { name: '確認贊助 50 點' }).click()
  await expect(p.getByRole('heading', { name: '✓ 感謝你的贊助！' })).toBeVisible()
  note('達標橫幅：' + (await p.getByText('剛好達標了').count() ? '有' : '無'))
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '嬰兒推車')).toContainText('已募集完成')
  await expect(card(p, '嬰兒推車')).toContainText('匿名朋友')
}, { page: S.pm })

await run.step('D-58', '/me/wallet：我的贊助列表、狀態徽章、帳本；撤回搖籃的贊助', async ({ note }) => {
  const p = S.pm
  // 先贊助搖籃 80 再撤回
  const r = await S.ming.api.post(`items/${L.cf.byTitle['搖籃']}/contributions`, { data: { points: 80, message: 'QA' }, headers: { 'Idempotency-Key': randomUUID() } })
  expect(r.status()).toBe(201)
  note('前置：以 API 為阿明贊助搖籃 80 點（UI 贊助流程已在 D-56 驗證）')
  await p.goto('/me/wallet'); await hyd(p)
  const pl = p.locator('article.w-pledge')
  await expect(pl).toHaveCount(3)
  note('贊助列：' + (await pl.allInnerTexts()).map(t => t.replace(/\s+/g, ' ').slice(0, 90)).join(' ｜ '))
  const row = pl.filter({ hasText: '搖籃' })
  await row.getByRole('button', { name: '撤回' }).click()
  await p.getByRole('button', { name: '確認撤回' }).click()
  await expect(p.getByText('已撤回 80 點')).toBeVisible()
  note('帳本：' + (await p.locator('main').innerText()).replace(/\s+/g, ' ').slice(-260))
}, { page: S.pm })

await run.step('D-59', '眾籌截止後轉投：玩具車贊助 70 點 → （快轉：標記截止）→ 錢包「轉投其他品項」→ 搖籃', async ({ note }) => {
  const p = S.pm
  const r = await S.ming.api.post(`items/${L.cf.byTitle['玩具車']}/contributions`, { data: { points: 70 }, headers: { 'Idempotency-Key': randomUUID() } })
  expect(r.status()).toBe(201)
  sql(`UPDATE wishlist_items SET funding_status='expired', expired_at=now(), funding_deadline=now()-interval '1 hour' WHERE id='${L.cf.byTitle['玩具車']}'`)
  note('平台外：以 SQL 模擬 tick_funding 把玩具車標為 expired（背景工作每 5 分鐘一次）；贊助 70 點以 API 建立')
  await p.goto('/me/wallet'); await hyd(p)
  const row = p.locator('article.w-pledge', { hasText: '玩具車' })
  await expect(row.getByText(/募集已截止，請在/)).toBeVisible()
  await row.getByRole('button', { name: '轉投其他品項' }).click()
  await expect(p.getByText('轉投到', { exact: true })).toBeVisible()
  await p.getByRole('button', { name: '確認轉投' }).click()
  await expect(p.getByText('已轉投', { exact: true }).first()).toBeVisible()
  await expect(p.locator('article.w-pledge', { hasText: '玩具車' })).toContainText('已轉投')
  note('轉投後各列：' + (await p.locator('article.w-pledge').allInnerTexts()).map(t => t.replace(/\s+/g, ' ').slice(0, 70)).join(' ｜ '))
  note('DB：' + sql(`SELECT string_agg(i.title||':'||c.points||':'||c.status, ', ' ORDER BY c.created_at) FROM contributions c JOIN wishlist_items i ON i.id=c.item_id WHERE c.user_id='${uidOf(S.ming.email)}'`))
}, { page: S.pm })

await run.step('D-60', '修改贊助可見度（錢包頁）', async ({ note }) => {
  const p = S.pm
  const sel = p.locator('article.w-pledge').first().getByRole('combobox')
  if (!(await sel.count())) throw new Gap('錢包贊助列沒有可見度下拉')
  await sel.selectOption('private')
  await expect(p.getByText('可見度已更新')).toBeVisible()
}, { page: S.pm })

await run.step('D-61', '/me/refund 現金退款：輸入驗證（0、小數、超額）→「全部」→ 送出 200 → 餘額先扣、列為待審核', async ({ note }) => {
  const p = S.pm
  await p.goto('/me/wallet'); await hyd(p)
  const bal0 = (await p.locator('.c-big').first().innerText()).replace(/,/g, '')
  await p.getByTestId('refund-entry').click()
  await expect(p).toHaveURL(/\/me\/refund$/)
  const refundable0 = (await p.getByTestId('refundable').innerText()).replace(/,/g, '')
  note(`錢包餘額 ${bal0}；可退現金 ${refundable0}`)
  for (const v of ['0', '12.5', '99999999']) {
    await p.getByLabel('退款點數').fill(v)
    await p.getByRole('button', { name: '送出申請' }).click()
    note(`點數 ${v} → ${await p.getByRole('alert').first().innerText().catch(() => '(無錯誤)')}`)
  }
  await p.getByRole('button', { name: '全部' }).click()
  note('「全部」填入：' + await p.getByLabel('退款點數').inputValue())
  await p.getByLabel('退款點數').fill('200')
  await expect(p.getByTestId('summary')).toContainText('扣除 200 點')
  await p.getByRole('button', { name: '送出申請' }).click()
  await expect(p.getByText(/已送出申請，扣除 200 點/)).toBeVisible()
  await expect(p.getByTestId('pending')).toBeVisible()
  await expect(p.getByText('待審核')).toBeVisible()
  const bal1 = sql(`SELECT balance FROM point_wallets WHERE user_id='${uidOf(S.ming.email)}'`)
  note(`退款後餘額 ${bal1}（先扣 200）；DB cash_refund_requests：${sql(`SELECT status, amount FROM cash_refund_requests WHERE wallet_id=(SELECT id FROM point_wallets WHERE user_id='${uidOf(S.ming.email)}')`)}`)
}, { page: S.pm })

await run.step('D-63', '贊助冪等：同一把 Idempotency-Key 重送兩次，錢包只扣一次', async ({ note }) => {
  const item = L.cf.byTitle['搖籃'], key = randomUUID()
  const bal = () => Number(sql(`SELECT balance FROM point_wallets WHERE user_id='${uidOf(S.ming.email)}'`))
  const b0 = bal()
  const mk = () => S.ming.api.post(`items/${item}/contributions`, { data: { points: 20 }, headers: { 'Idempotency-Key': key } })
  const [r1, r2] = [await mk(), await mk()]
  note(`第一次 ${r1.status()}、第二次 ${r2.status()} replayed=${r2.headers()['idempotency-replayed']}；餘額 ${b0} → ${bal()}`)
  expect(b0 - bal()).toBe(20)
  const r3 = await S.ming.api.post(`items/${item}/contributions`, { data: { points: 20 }, headers: {} })
  note(`缺 Idempotency-Key：${r3.status()} ${(await r3.json().catch(() => ({}))).code}`)
}, {})

await run.step('D-64', '現金退款第二次：超過剩餘可退現金額被擋、不會變成負數', async ({ note }) => {
  const p = S.pm
  await p.goto('/me/refund'); await hyd(p)
  const refundable = Number((await p.getByTestId('refundable').innerText()).replace(/,/g, ''))
  await p.getByLabel('退款點數').fill(String(refundable + 1))
  await p.getByRole('button', { name: '送出申請' }).click()
  note(`可退現金 ${refundable}；輸入 ${refundable + 1} → ${await p.getByRole('alert').first().innerText()}`)
  // 直接打 API 超額（繞過前端）
  const r = await S.ming.api.post('wallet/cash-refunds', { data: { amount: refundable + 1 }, headers: { 'Idempotency-Key': randomUUID() } })
  note(`API 直接超額申請：${r.status()} ${(await r.text()).slice(0, 160)}`)
  expect(r.status()).toBeGreaterThanOrEqual(400)
  await expect(p.getByText('我的申請')).toBeVisible()
  note('申請列表：' + (await p.locator('main').innerText()).replace(/\s+/g, ' ').slice(-120))
}, { page: S.pm })

// =====================================================================
// I. 其他：救災據點、退訂、未登入進入會員頁
// =====================================================================
await run.step('D-70', '救災：/e → 事件頁 → 據點分享頁（組織資訊、地址）→ 認捐瓶裝水成功', async ({ note }) => {
  // 前置：SQL 種子（同 e2e/relief-public.spec.ts；組織審核/許可流程屬組織角色）
  const ownerId = uidOf(owner.email)
  const sg = p => p + (uid() + uid()).slice(0, 9)
  const orgSlug = sg('o'), evSlug = sg('q'), wlSlug = sg('w'), wl2 = sg('w')
  const orgName = `QA 救援協會 ${T}`, evName = `QA 花蓮地震 ${T}`, title = `QA 收件據點 ${T}`
  const orgId = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
    VALUES ('${orgSlug}', '${orgName}', '${orgName}', 'association', lpad((floor(random()*1e8))::text, 8, '0'), 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${ownerId}') RETURNING id`).split('\n')[0]
  const permitId = sql(`INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to, status) VALUES ('${orgId}', 'P-${T}', current_date - 1, current_date + 90, 'approved') RETURNING id`).split('\n')[0]
  const evId = sql(`INSERT INTO relief_events (slug, name, hazard_type, counties, occurred_on, created_by_org) VALUES ('${evSlug}', '${evName}', 'earthquake', ARRAY['花蓮縣'], current_date, '${orgId}') RETURNING id`).split('\n')[0]
  const mk = (slug, ttl, status) => sql(`INSERT INTO wishlists (owner_id, type, status, visibility, slug, title, org_id, event_id, permit_id, site_address, site_status)
    VALUES ('${ownerId}', 'relief', 'active', 'public', '${slug}', '${ttl}', '${orgId}', '${evId}', '${permitId}', '花蓮縣花蓮市中山路 1 號', '${status}') RETURNING id`).split('\n')[0]
  const w1 = mk(wlSlug, title, 'active'), w2 = mk(wl2, title + '（暫停）', 'active')
  sql(`INSERT INTO wishlist_items (wishlist_id, title, qty_needed, category, urgency) VALUES ('${w1}', '瓶裝水', 10, 'water', 'critical'), ('${w2}', '毛毯', 5, 'shelter', 'high')`)
  S.relief = { orgSlug, evSlug, wlSlug, wl2, w2, orgName, evName, title }
  note(`平台外：SQL 種子組織/許可/事件/兩個據點（${orgSlug}/${evSlug}/${wlSlug}）`)
  const p = S.pm
  const bad = []; p.on('response', r => { if (r.status() >= 400) bad.push(`${r.status()} ${r.url().replace(API, '')}`) })
  await p.goto('/explore'); await hyd(p)
  await p.getByRole('link', { name: /災害救援/ }).click()
  await expect(p).toHaveURL(/\/e$/)
  await p.getByRole('link', { name: evName }).click()
  await expect(p.getByRole('link', { name: title, exact: true })).toBeVisible()
  await expect(p.getByText('急缺 1')).toBeVisible()
  await p.getByRole('link', { name: title, exact: true }).click()
  await hyd(p)
  await expect(p.getByRole('link', { name: orgName }).first()).toBeVisible()
  await expect(p.getByText('已驗證組織').first()).toBeVisible()
  note('據點地址連結數（頁面 DOM）：' + await p.locator('a[href*="google.com/maps"]').count() + '；可見數：' + await p.locator('a[href*="google.com/maps"]:visible').count())
  await expect(p.getByText('花蓮縣花蓮市中山路 1 號').first()).toBeVisible()
  await card(p, '瓶裝水').getByRole('button', { name: '我來送' }).click()
  await p.getByRole('dialog').getByRole('button', { name: '增加' }).click()
  await p.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
  await expect(done(p)).toBeVisible()
  await p.getByRole('button', { name: '回到清單繼續看' }).click()
  await expect(card(p, '瓶裝水')).toContainText('2 / 10')
  note('此流程中的 4xx 回應：' + [...new Set(bad)].join('、'))
}, { page: S.pm })

await run.step('D-71', '救災・組織頁 /o/[slug]、暫停收件的據點（橫幅、送禮鈕停用）', async ({ note }) => {
  const p = S.pm, R = S.relief
  await p.goto(`/o/${R.orgSlug}`); await hyd(p)
  await expect(p.getByRole('heading', { name: new RegExp(R.orgName) })).toBeVisible()
  await expect(p.getByRole('link', { name: R.title, exact: true })).toBeVisible()
  sql(`UPDATE wishlists SET site_status='paused' WHERE id='${R.w2}'`)
  note('平台外：SQL 把第二個據點改為 paused（模擬組織暫停）')
  await p.goto(`/s/${R.wl2}`); await hyd(p)
  await expect(p.getByText('此據點目前暫停收件')).toBeVisible()
  const dis = await card(p, '毛毯').getByRole('button', { name: '我來送' }).isDisabled().catch(() => null)
  note('暫停據點送禮鈕 disabled=' + dis)
  expect(dis).toBe(true)
  const r = await apiClaim(p.request, (await sql(`SELECT id FROM wishlist_items WHERE wishlist_id='${R.w2}'`)).split('\n')[0], 1)
  note(`直接打 API 認捐暫停據點：${r.status()} ${(await r.json().catch(() => ({}))).code}`)
  expect(r.status()).toBeGreaterThanOrEqual(400)
}, { page: S.pm })

await run.step('D-72', '退訂連結 /unsubscribe：二步確認（開頁不退訂）→ 確定退訂 → 已退訂；亂填 token 顯示失效', async ({ note }) => {
  const id = uidOf(S.ming.email)
  const msg = `b:${id}:${Math.floor(Date.now() / 1000) + 86400}`
  const sig = createHmac('sha256', 'dev-unsub-secret').update(msg).digest('hex')
  const token = `${Buffer.from(msg).toString('base64url')}.${sig}`
  const p = await ctxPage()
  const r = await p.request.get(`${API}/api/v1/unsubscribe?token=${token}`, { maxRedirects: 0 })
  note(`GET /api/v1/unsubscribe → ${r.status()} Location=${r.headers().location}`)
  expect(r.status()).toBe(302)
  await p.goto(`/unsubscribe?token=${token}`); await hyd(p)
  await expect(p.getByRole('heading', { name: '確定要退訂送禮通知？' })).toBeVisible()
  const before = sql(`SELECT notification_prefs FROM users WHERE id='${id}'`)
  await p.getByRole('button', { name: '確定退訂' }).click()
  await expect(p.getByRole('heading', { name: '已退訂' })).toBeVisible()
  const after = sql(`SELECT notification_prefs FROM users WHERE id='${id}'`)
  note(`平台外：以開發機密 dev-unsub-secret 簽出「生日通知(b)」退訂 token（真實信件由 notify.rs 產生）；notification_prefs ${before} → ${after}`)
  note('頁面文案：' + (await p.locator('.x-n-body').innerText()).replace(/\s+/g, ' '))
  if (/email_birthdays.*false/.test(after)) throw new Gap('退訂的是「生日通知」(kind b)，但頁面標題寫「確定要退訂送禮通知？」、完成後寫「之後不會再寄『有人來送』的通知信」，文案與實際效果不符')
  await p.goto('/unsubscribe?token=garbage'); await hyd(p)
  await p.getByRole('button', { name: '確定退訂' }).click()
  await expect(p.getByRole('heading', { name: '連結已失效' })).toBeVisible()
}, {})

await run.step('D-73', '未登入直接進會員頁：/me/claims（空狀態有無登入引導）、/me/wallet /me/topup /me/refund（導登入）', async ({ note }) => {
  const p = await ctxPage()
  await p.goto('/me/claims'); await hyd(p)
  await expect(p.getByText('你還沒有要送的禮物')).toBeVisible()
  const loginLink = await p.getByRole('link', { name: /登入/ }).count() + await p.getByRole('button', { name: /登入/ }).count()
  note('/me/claims 未登入內文：' + (await p.locator('main').innerText()).replace(/\s+/g, ' '))
  const redirs = []
  for (const path of ['/me/wallet', '/me/topup', '/me/refund']) {
    await p.goto(path); await p.waitForURL(/\/login/, { timeout: 10000 }).catch(() => {})
    redirs.push(`${path} → ${new URL(p.url()).pathname + new URL(p.url()).search}`)
  }
  note(redirs.join('；'))
  if (!loginLink) throw new Gap('/me/claims 未登入只顯示「你還沒有要送的禮物…登入會員就能認領」空狀態，沒有任何「登入」連結；而 /me/wallet 等會直接導去登入，行為不一致')
  await p.context().close()
}, {})

await run.step('D-62', '手機 390 寬的錢包／儲值／退款頁：無橫向捲動、主要按鈕可點', async ({ note }) => {
  const p = S.aunt
  for (const path of ['/me/wallet', '/me/topup', '/me/refund', '/me/claims']) {
    await p.goto(path); await hyd(p)
    await noHScroll(p)
    note(`${path}：頁面寬度 OK；標題 ` + (await p.locator('h1').first().innerText().catch(() => '(無 h1)')))
  }
  await p.goto('/me/topup'); await hyd(p)
  const h = await p.getByRole('button', { name: '前往付款' }).evaluate(e => Math.round(e.getBoundingClientRect().height))
  note(`「前往付款」高度 ${h}px`)
}, { page: S.aunt })

await run.step('D-74', '信件盤點：阿姨（認捐、取消、延長…）實際收到哪些信', async ({ note }) => {
  const mails = await listMail(S.auntEmail)
  note('阿姨收到：' + mails.map(m => m.subject).join('｜'))
  note('受捐者小愛收到：' + (await listMail(owner.email)).map(m => m.subject).join('｜'))
  note('阿姨的排程通知（購買提醒等）：' + sql(`SELECT string_agg(kind||'/'||status::text||'/'||to_char(scheduled_at,'MM-DD HH24:MI'), '；' ORDER BY scheduled_at) FROM notifications WHERE user_id='${uidOf(S.auntEmail)}'`))
  note('阿明（儲值、贊助、退款申請、達標）收到：' + (await listMail(S.ming.email)).map(m => m.subject).join('｜'))
  note('小林（搶到嬰兒帽）收到：' + (await listMail(S.lin.email)).map(m => m.subject).join('｜'))
  note('小陳（搶輸）收到：' + (await listMail(S.chen.email)).map(m => m.subject).join('｜'))
}, {})

// =====================================================================
// J. 背景釋放逾期認捐（接 D-28）
// =====================================================================
await run.step('D-29', '購買期限逾期釋放（接 D-28）：背景工作跑完後認捐變「已逾期」、數量釋出、收到逾期信', async ({ note }) => {
  const id = L.main.byTitle['圍兜']
  const state = () => sql(`SELECT status FROM claims WHERE id='${S.expClaim}'`)
  if (process.env.DONOR_SKIP_WAIT && state() === 'reserved') throw new Blocked('DONOR_SKIP_WAIT=1：略過等待背景 expire_due（5 分鐘週期）')
  const t0 = Date.now()
  while (state() === 'reserved' && Date.now() - t0 < 330000) await sleep(5000)
  const st = state()
  note(`等了 ${Math.round((Date.now() - S.expAt) / 1000)} 秒；claim 狀態=${st}；qty_claimed=${qtyClaimed(id)}`)
  if (st === 'reserved') throw new Error('等了 5.5 分鐘仍未釋放（expire_due 未執行？）')
  const p = S.aunt
  await p.goto('/me/claims'); await hyd(p)
  await p.getByText(/過去的紀錄/).click()
  await expect(p.getByText('圍兜 × 1・已逾期')).toBeVisible()
  const mails = (await listMail(S.auntEmail)).map(m => m.subject)
  note('阿姨信件：' + mails.join('｜'))
  await p.goto(`/s/${L.main.slug}`); await hyd(p)
  await expect(card(p, '圍兜')).toContainText('0 / 2')
}, { page: S.aunt })

// =====================================================================
// K. 本環境無法驗證
// =====================================================================
await run.step('D-90', '真機 LINE 內建瀏覽器（cookie 保留、「用預設瀏覽器開啟」、分享預覽圖）', async () => { throw new Blocked('需真機與 LINE app；本環境只用 UA 字串模擬 isLineBrowser（D-02）。分享預覽（OG 圖）只能驗 SSR meta（D-01），LINE 爬蟲實際抓圖不可驗') })
await run.step('D-91', '第三方登入（Google / LINE）、Turnstile 真實驗證', async () => { throw new Blocked('需真實 OAuth 供應商與 Cloudflare Turnstile site key；登入頁按鈕只確認存在；檢舉 / 註冊在無 site key 時送 dev-bypass') })
await run.step('D-92', '真實綠界付款頁、超商代碼／ATM 取號、發票', async () => { throw new Blocked('綠界 stage 需外網；只驗到「導向表單欄位正確」與「簽章通知入帳」（替身）。超商代碼／ATM 的取號資訊需 /info 通知，替身未實作') })
await run.step('D-93', 'CDN 快取（/s/slug 公開回應快取、受限清單 no-store、purge）', async () => { throw new Blocked('本地沒有 Cloudflare；僅在 D-33/D-34 驗證受限清單走帶憑證路徑') })
await run.step('D-94', '原生殼（Tauri）App 內儲值導向網頁版', async () => { throw new Blocked('需 Tauri 殼；topup.vue 有 native 分支但此環境為純網頁') })

run.save()
await closeBrowser()
