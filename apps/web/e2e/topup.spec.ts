import { expect, test, type Page, type Route } from '@playwright/test'
import { signInPage } from './helpers'
import { hydrated } from './support/hydrate'
import { WEB } from './support/env'

// 儲值頁（/me/topup）：登入走真後端，/wallet 與 /wallet/topups 以 page.route 模擬（GET 列表為本分支的前端契約，後端另案實作；
// 綠界付款頁也不真的連外，只攔截導向的表單 POST）。
const ECPAY = 'https://payment-stage.ecpay.com.tw/Cashier/AioCheckOut/V5'
const FIELDS = { MerchantID: '3002607', MerchantTradeNo: 'T0001', TotalAmount: '1000', TradeDesc: 'Wishly', CheckMacValue: 'ABCDEF' }
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
const iso = (ms = 0) => new Date(Date.now() + ms).toISOString()
const cors = { 'access-control-allow-origin': WEB, 'access-control-allow-credentials': 'true', 'access-control-expose-headers': 'retry-after' }

type Topup = Record<string, any>
const topup = (o: Topup = {}): Topup => ({ id: 'tp1', amount: 1000, method: 'card', status: 'pending', created_at: iso(-60_000), paid_at: null, expires_at: iso(25 * 60_000), payment_info: null, ...o })

interface Mock { posts: { headers: Record<string, string>, body: any }[], gets: number }
/** list(n)：第 n 次 GET 的列表；post(n)：第 n 次 POST 的回應 */
async function mockApi(page: Page, o: { list?: (n: number) => Topup[], post?: (n: number) => { status: number, json: any, headers?: Record<string, string> }, balance?: number, delay?: number } = {}): Promise<Mock> {
  const m: Mock = { posts: [], gets: 0 }
  const json = (route: Route, status: number, body: any, headers: Record<string, string> = {}) =>
    route.fulfill({ status, headers: { ...cors, ...headers }, contentType: status >= 400 ? 'application/problem+json' : 'application/json', body: JSON.stringify(body) })
  await page.route(/\/api\/v1\/public\/status/, r => json(r, 200, { read_only: false, topup_methods: ['card', 'cvs_code', 'cvs_barcode', 'atm'] }))
  await page.route(/\/api\/v1\/wallet\?/, r => json(r, 200, { wallet: { balance: o.balance ?? 0, held_points: 0, status: 'active' }, data: [], next_cursor: null }))
  await page.route(/\/api\/v1\/wallet\/topups/, async (route) => {
    const req = route.request()
    if (req.method() === 'GET') return json(route, 200, { topups: (o.list ?? (() => []))(++m.gets) })
    m.posts.push({ headers: req.headers(), body: req.postDataJSON() })
    if (o.delay) await new Promise(r => setTimeout(r, o.delay))
    const r = (o.post ?? (() => ({ status: 201, json: { topup: { id: 'tp1', amount: 1000, method: 'card', provider: 'ecpay', status: 'pending', paid_at: null, created_at: iso() }, payment: { type: 'form_post', url: ECPAY, fields: FIELDS } } })))(m.posts.length)
    return json(route, r.status, r.json, r.headers)
  })
  return m
}
/** 攔截導向綠界的表單 POST，回一張假頁面 */
async function captureEcpay(page: Page) {
  await page.route(`${ECPAY}**`, r => r.fulfill({ status: 200, contentType: 'text/html', body: '<h1>stub ecpay</h1>' }))
}
const problem = (status: number, code: string, detail: string, extra: object = {}) => ({ status, json: { type: 'about:blank', title: code, status, code, detail, ...extra } })

async function open(page: Page) {
  await signInPage(page)
  await page.goto('/me/topup'); await hydrated(page)
}
const pay = (page: Page) => page.getByRole('button', { name: '前往付款' })

test('建單成功 → 以隱藏表單 POST 導向綠界（action 與 fields 完整），帶 Idempotency-Key', async ({ page }) => {
  const m = await mockApi(page)
  await captureEcpay(page)
  await open(page)

  await page.getByRole('button', { name: '1,000', exact: true }).click()
  await expect(page.getByRole('button', { name: '1,000', exact: true })).toHaveAttribute('aria-pressed', 'true')
  await page.getByRole('radio', { name: /超商代碼/ }).check()
  await expect(page.getByTestId('summary')).toHaveText('儲值 NT$ 1,000，入帳 1,000 點（1 點 = NT$ 1）')
  await expect(page.getByText('點數不可提領給收禮者，也不可轉讓。')).toBeVisible()

  const nav = page.waitForRequest(r => r.url().startsWith(ECPAY) && r.method() === 'POST')
  await pay(page).click()
  const req = await nav
  expect(Object.fromEntries(new URLSearchParams(req.postData() ?? ''))).toEqual(FIELDS)
  expect(m.posts).toHaveLength(1)
  expect(m.posts[0].body).toEqual({ amount: 1000, method: 'cvs_code', invoice: { carrier_type: 'member' } })
  expect(m.posts[0].headers['idempotency-key']).toMatch(UUID)
})

test('送出中按鈕停用，連點只建一張單', async ({ page }) => {
  const m = await mockApi(page, { delay: 1000, post: () => problem(500, 'INTERNAL', '內部錯誤') })
  await open(page)
  await page.getByLabel('自訂金額').fill('500')
  await pay(page).click()
  await expect(page.getByRole('button', { name: '處理中，請稍候…' })).toBeDisabled()
  await page.getByRole('button', { name: '處理中，請稍候…' }).click({ force: true })
  await expect(page.getByText('系統暫時無法處理')).toBeVisible()
  expect(m.posts).toHaveLength(1)
})

test('前端先驗：金額範圍逐欄顯示，錯誤時不送出', async ({ page }) => {
  const m = await mockApi(page)
  await open(page)

  await page.getByLabel('自訂金額').fill('50')
  await pay(page).click()
  await expect(page.getByRole('alert').filter({ hasText: '金額須介於 100 至 50,000' })).toBeVisible()
  await page.getByLabel('自訂金額').fill('1e3')
  await expect(page.getByRole('alert').filter({ hasText: '請輸入整數金額' })).toBeVisible()

  expect(m.posts).toHaveLength(0)
})

test('Idempotency-Key：同一份內容重試沿用同一把 key，內容改變才換新', async ({ page }) => {
  const m = await mockApi(page, { post: () => problem(500, 'INTERNAL', '內部錯誤') })
  await open(page)

  await page.getByLabel('自訂金額').fill('500')
  await pay(page).click(); await expect(page.getByText('系統暫時無法處理')).toBeVisible()
  await pay(page).click(); await expect.poll(() => m.posts.length).toBe(2)
  await page.getByLabel('自訂金額').fill('600')
  await pay(page).click(); await expect.poll(() => m.posts.length).toBe(3)

  const keys = m.posts.map(p => p.headers['idempotency-key'])
  expect(keys[0]).toMatch(UUID)
  expect(keys[1]).toBe(keys[0])
  expect(keys[2]).not.toBe(keys[0])
})

test.describe('錯誤碼', () => {
  const cases: [string, ReturnType<typeof problem>, RegExp][] = [
    ['WALLET_CAP_EXCEEDED', problem(409, 'WALLET_CAP_EXCEEDED', 'x'), /超過上限 50,000 點/],
    ['TOPUP_LIMIT_EXCEEDED', problem(409, 'TOPUP_LIMIT_EXCEEDED', 'x', { limit_amount: 3000, remaining_amount: 500 }), /每日儲值上限 NT\$ 3,000.*還能儲值 NT\$ 500/],
    ['RATE_LIMITED', { ...problem(429, 'RATE_LIMITED', 'x'), headers: { 'retry-after': '1800' } }, /約 30 分鐘後再試/],
    ['VALIDATION_FAILED', problem(422, 'VALIDATION_FAILED', 'x', { errors: [{ pointer: '/amount', code: 'INVALID', detail: '後端：金額不合' }] }), /內容有誤/],
  ]
  for (const [name, res, text] of cases) {
    test(name, async ({ page }) => {
      await mockApi(page, { post: () => res })
      await open(page)
      await page.getByLabel('自訂金額').fill('500')
      await pay(page).click()
      await expect(page.getByRole('alert').filter({ hasText: text })).toBeVisible()
      await expect(pay(page)).toBeEnabled() // 可修正後重試
      if (name === 'VALIDATION_FAILED') await expect(page.getByText('後端：金額不合')).toBeVisible() // 逐欄訊息顯示在金額欄
    })
  }

  test('TOPUP_DISABLED：顯示尚未開放並隱藏表單', async ({ page }) => {
    await mockApi(page, { post: () => problem(503, 'TOPUP_DISABLED', 'x') })
    await open(page)
    await page.getByLabel('自訂金額').fill('500')
    await pay(page).click()
    await expect(page.getByText('儲值功能尚未開放')).toBeVisible()
    await expect(page.getByLabel('自訂金額')).toHaveCount(0)
  })

  test('後端 422 逐欄訊息顯示在對應欄位', async ({ page }) => {
    await mockApi(page, { post: () => problem(422, 'VALIDATION_FAILED', 'x', { errors: [{ pointer: '/amount', code: 'INVALID', detail: '後端：這組條碼不存在' }] }) })
    await open(page)
    await page.getByLabel('自訂金額').fill('500')
    await pay(page).click()
    await expect(page.getByRole('alert').filter({ hasText: '後端：這組條碼不存在' })).toBeVisible()
  })
})

test('儲值紀錄：狀態徽章、超商代碼可複製、條碼三段、ATM 帳號、到期時間', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: WEB })
  const exp = iso(2 * 864e5 + 3 * 36e5)
  await mockApi(page, {
    list: () => [
      topup({ id: 'a', method: 'cvs_code', payment_info: { type: 'cvs_code', payment_no: 'LLL24110512345', expire_at: exp }, expires_at: exp }),
      topup({ id: 'b', method: 'cvs_barcode', amount: 500, payment_info: { type: 'cvs_barcode', barcode1: '1311053TW', barcode2: '3002607000012345', barcode3: '110512000000500', expire_at: exp }, expires_at: exp }),
      topup({ id: 'c', method: 'atm', amount: 2000, payment_info: { type: 'atm', bank_code: '012', account: '9103522175887271', expire_at: exp }, expires_at: exp }),
      topup({ id: 'd', status: 'paid', paid_at: iso(-3_600_000), amount: 3000 }),
      topup({ id: 'e', status: 'expired', amount: 700 }),
    ],
  })
  await open(page)

  const cards = page.locator('article[data-status]')
  await expect(cards).toHaveCount(5)
  await expect(cards.nth(3).locator('.c-badge')).toHaveText('已入帳')
  await expect(cards.nth(4).locator('.c-badge')).toHaveText('已逾期')

  const cvs = cards.nth(0)
  await expect(cvs.locator('.c-badge')).toHaveText('待付款')
  await expect(cvs.getByText('LLL24110512345')).toBeVisible()
  await expect(cvs.getByText(/剩 2 天 3 小時/)).toBeVisible()
  await cvs.getByRole('button', { name: '複製' }).click()
  await expect(cvs.getByRole('button', { name: '已複製' })).toBeVisible()
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('LLL24110512345')

  const bar = cards.nth(1)
  for (const v of ['1311053TW', '3002607000012345', '110512000000500']) await expect(bar.locator('code', { hasText: v })).toBeVisible()
  await expect(bar.getByRole('img')).toHaveCount(3) // 三段 Code 39 條碼
  await expect(bar.locator('code').first()).toHaveCSS('font-family', /mono|Menlo|Consolas/i)
  const atm = cards.nth(2)
  await expect(atm.getByText('9103522175887271')).toBeVisible()
  await expect(atm.getByText('銀行代碼')).toBeVisible()
})

test('取號中：payment_info 為 null 顯示「取號中…」，每 3 秒輪詢，出現代碼後停止', async ({ page }) => {
  const exp = iso(3 * 864e5)
  const m = await mockApi(page, {
    list: n => [topup({ method: 'cvs_code', expires_at: exp, payment_info: n >= 3 ? { type: 'cvs_code', payment_no: 'LLL99999', expire_at: exp } : null })],
  })
  await open(page)
  await expect(page.getByText('取號中…')).toBeVisible()
  await expect(page.getByText('LLL99999')).toBeVisible({ timeout: 15_000 })
  await expect(page.getByText('取號中…')).toHaveCount(0)
  const n = m.gets
  await page.waitForTimeout(3500)
  expect(m.gets).toBe(n) // 取到之後不再輪詢
})

test('輪詢上限 10 次後停止，改由手動重新整理', async ({ page }) => {
  const m = await mockApi(page, { list: () => [topup({ method: 'atm', payment_info: null })] })
  await page.clock.install()
  await open(page)
  await expect(page.getByText('取號中…')).toBeVisible()
  await expect.poll(async () => { await page.clock.runFor(3100); return page.getByText('尚未更新？請按「重新整理」').isVisible() }, { timeout: 30_000 }).toBe(true)
  const n = m.gets
  expect(n).toBe(11) // 初次載入 1 次 + 輪詢 10 次
  await page.clock.runFor(10_000)
  expect(m.gets).toBe(n)
  await page.getByRole('button', { name: '重新整理' }).click()
  await expect.poll(() => m.gets).toBe(n + 1)
})

test('從綠界返回：對 pending 單輪詢，入帳後顯示成功與新餘額', async ({ page }) => {
  await page.addInitScript(() => sessionStorage.setItem('ws_topup_watch', 'tp1'))
  await mockApi(page, { balance: 1000, list: n => [topup({ status: n >= 2 ? 'paid' : 'pending', paid_at: n >= 2 ? iso() : null })] })
  await open(page)
  await expect(page.locator('article[data-status="pending"]')).toBeVisible()
  await expect(page.getByRole('status').filter({ hasText: '儲值成功，已入帳 1,000 點，目前餘額 1,000 點' })).toBeVisible({ timeout: 15_000 })
  await expect(page.locator('article[data-status="paid"]')).toBeVisible()
})

test('錢包頁有「儲值」入口', async ({ page }) => {
  await mockApi(page)
  await signInPage(page)
  await page.goto('/me/wallet'); await hydrated(page)
  await page.getByRole('link', { name: '儲值' }).click()
  await expect(page).toHaveURL(/\/me\/topup$/)
  await expect(page.getByRole('heading', { name: '儲值', exact: true }).first()).toBeVisible()
})

test('語系：英文介面的關鍵文案與錯誤訊息', async ({ page, context }) => {
  await context.addCookies([{ name: 'ws_locale', value: 'en', url: WEB }])
  await mockApi(page, { post: () => problem(409, 'TOPUP_LIMIT_EXCEEDED', 'x', { limit_amount: 3000, remaining_amount: 500 }) })
  await open(page)
  await expect(page.getByRole('heading', { name: 'Top up', exact: true }).first()).toBeVisible()
  await expect(page.getByRole('radio', { name: /Credit card/ })).toBeVisible()
  await expect(page.getByText('Complete payment within 30 minutes.').first()).toBeVisible()
  await page.getByRole('button', { name: '500', exact: true }).click()
  await expect(page.getByTestId('summary')).toHaveText('Top up NT$ 500, credited as 500 points (1 point = NT$ 1)')
  await page.getByRole('button', { name: 'Continue to payment' }).click()
  await expect(page.getByRole('alert').filter({ hasText: 'New accounts can top up up to NT$ 3,000 per day. You can still add NT$ 500 today.' })).toBeVisible()
})

test('取消待付款單：確認後 POST cancel 並重讀列表；已付款則 409 後重讀', async ({ page }) => {
  let status = 'pending'
  await mockApi(page, { list: () => [topup({ status })] })
  const cancels: string[] = []
  await page.route(/\/api\/v1\/wallet\/topups\/tp1\/cancel/, (r) => {
    cancels.push(r.request().method()); status = 'failed'
    return r.fulfill({ status: 200, headers: cors, contentType: 'application/json', body: JSON.stringify({ topup: topup({ status }) }) })
  })
  await open(page)
  await page.getByTestId('cancel-topup').click()
  await page.getByRole('button', { name: '取消訂單' }).last().click() // 最後一顆在確認對話框內
  await expect(page.locator('article[data-status="failed"]')).toBeVisible()
  expect(cancels).toEqual(['POST'])
})

test('後端未開放任何儲值方式：顯示尚未開放，不渲染表單', async ({ page }) => {
  await mockApi(page)
  await page.route(/\/api\/v1\/public\/status/, r => r.fulfill({ status: 200, headers: cors, contentType: 'application/json', body: JSON.stringify({ topup_methods: [] }) }))
  await open(page)
  await expect(page.getByText('儲值功能尚未開放')).toBeVisible()
  await expect(page.getByLabel('自訂金額')).toHaveCount(0)
})
