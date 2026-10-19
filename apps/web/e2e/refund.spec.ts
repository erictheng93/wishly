import { expect, test, type Page } from '@playwright/test'
import { signInPage } from './helpers'
import { hydrated } from './support/hydrate'
import { WEB } from './support/env'
import { UUID, iso, mock, ok, problem, type Res } from './support/mockApi'

// 現金退款（/me/refund）：登入走真後端，GET /wallet 與 /wallet/cash-refunds 以 page.route 模擬（契約見 docs/04 ep-post-wallet-cash-refunds）。
type Req = Record<string, any>
interface Opts { wallet?: Req, requests?: Req[], post?: (n: number) => Res | undefined, delay?: number }

async function setup(page: Page, o: Opts = {}) {
  const wallet = { id: 'w1', balance: 3500, held_points: 0, status: 'active', cash_refundable_balance: 3000, ...o.wallet }
  const requests: Req[] = [...(o.requests ?? [])]
  await mock(page, /\/api\/v1\/wallet(\?|$)/, () => ok({ wallet, data: [], next_cursor: null }))
  const calls = await mock(page, /\/api\/v1\/wallet\/cash-refunds/, async (c, n) => {
    if (c.method === 'GET') return ok({ requests })
    if (o.delay) await new Promise(r => setTimeout(r, o.delay))
    const forced = o.post?.(calls.filter(x => x.method === 'POST').length)
    if (forced) return forced
    const r = { id: `cr${n}`, amount: c.body.amount, status: 'requested', created_at: iso(), completed_at: null, reject_reason: null }
    requests.unshift(r); wallet.balance -= r.amount; wallet.cash_refundable_balance -= r.amount
    return ok({ request: r, wallet: { balance: wallet.balance } }, 201)
  })
  return { posts: () => calls.filter(c => c.method === 'POST'), gets: () => calls.filter(c => c.method === 'GET') }
}
async function open(page: Page, path = '/me/refund') {
  await signInPage(page)
  await page.goto(path); await hydrated(page)
}
const amount = (page: Page) => page.getByLabel('退款點數')
const submit = (page: Page) => page.getByRole('button', { name: '送出申請' })

test('申請成功：請求內容與 Idempotency-Key、清單出現新申請、餘額與審核中提示更新', async ({ page }) => {
  const m = await setup(page)
  await open(page)
  await expect(page.getByTestId('refundable')).toHaveText('3,000')
  await expect(page.getByText('只有儲值購買的點數可以退現金；禮物卡與贊助取得的點數不可退。')).toBeVisible()
  await expect(page.getByText('還沒有退款申請。')).toBeVisible()

  await amount(page).fill('1200')
  await expect(page.getByTestId('summary')).toHaveText('將先從錢包扣除 1,200 點，審核通過後退款 NT$ 1,200。')
  await submit(page).click()

  await expect(page.getByRole('status').filter({ hasText: '已送出申請，扣除 1,200 點，目前餘額 2,300 點' })).toBeVisible()
  expect(m.posts()).toHaveLength(1)
  expect(m.posts()[0].body).toEqual({ amount: 1200 })
  expect(m.posts()[0].headers['idempotency-key']).toMatch(UUID)
  const card = page.locator('article[data-status="requested"]')
  await expect(card).toHaveCount(1)
  await expect(card.locator('.c-badge')).toHaveText('待審核')
  await expect(card).toContainText('1,200 點')
  await expect(page.getByTestId('pending')).toHaveText('審核中：你有 1 筆退款申請正在處理。')
  await expect(page.getByTestId('refundable')).toHaveText('1,800') // 重讀 GET /wallet
  await expect(amount(page)).toHaveValue('')
})

test('「全部」帶入可退餘額；前端先驗：非整數 / 0 / 超過餘額都不送出', async ({ page }) => {
  const m = await setup(page)
  await open(page)
  await page.getByRole('button', { name: '全部' }).click()
  await expect(amount(page)).toHaveValue('3000')
  await expect(page.getByTestId('summary')).toContainText('扣除 3,000 點')

  await amount(page).fill('3001'); await submit(page).click()
  await expect(page.getByRole('alert').filter({ hasText: '不可超過可退現金餘額' })).toBeVisible()
  await amount(page).fill('0')
  await expect(page.getByRole('alert').filter({ hasText: '點數須介於 1 至 50,000' })).toBeVisible()
  await amount(page).fill('1e3')
  await expect(page.getByRole('alert').filter({ hasText: '請輸入整數點數' })).toBeVisible()
  await amount(page).fill('')
  await expect(page.getByRole('alert').filter({ hasText: '請輸入整數點數' })).toBeVisible()
  expect(m.posts()).toHaveLength(0)

  await amount(page).fill('3000'); await submit(page).click() // 剛好等於可退餘額可送
  await expect.poll(() => m.posts().length).toBe(1)
  expect(m.posts()[0].body).toEqual({ amount: 3000 })
})

test('送出中按鈕停用，連點只送一次', async ({ page }) => {
  const m = await setup(page, { delay: 800 })
  await open(page)
  await amount(page).fill('500')
  await submit(page).click()
  const busy = page.getByRole('button', { name: '送出中…' })
  await expect(busy).toBeDisabled()
  await busy.click({ force: true })
  await expect(page.getByRole('status').filter({ hasText: '已送出申請' })).toBeVisible()
  expect(m.posts()).toHaveLength(1)
})

test('Idempotency-Key：同一份內容重試沿用同一把 key，內容改變才換新，成功後再申請是新 key', async ({ page }) => {
  const m = await setup(page, { post: n => (n <= 2 ? problem(500, 'INTERNAL', '內部錯誤') : undefined) })
  await open(page)
  await amount(page).fill('500')
  await submit(page).click(); await expect(page.getByRole('alert').filter({ hasText: '系統暫時無法處理' })).toBeVisible()
  await submit(page).click(); await expect.poll(() => m.posts().length).toBe(2)
  await amount(page).fill('600')
  await submit(page).click(); await expect(page.getByRole('status').filter({ hasText: '已送出申請' })).toBeVisible()
  await amount(page).fill('600')
  await submit(page).click(); await expect.poll(() => m.posts().length).toBe(4)

  const keys = m.posts().map(p => p.headers['idempotency-key'])
  expect(keys[0]).toMatch(UUID)
  expect(keys[1]).toBe(keys[0])
  expect(keys[2]).not.toBe(keys[0])
  expect(keys[3]).not.toBe(keys[2]) // 成功後 reset
})

test.describe('錯誤碼', () => {
  const cases: [string, Res, RegExp][] = [
    ['NOT_CASH_REFUNDABLE', problem(409, 'NOT_CASH_REFUNDABLE', 'x', { refundable_points: 300, balance: 3500 }), /可退現金的點數只有 300 點（禮物卡與贊助點數不可退現金）/],
    ['INSUFFICIENT_POINTS', problem(409, 'INSUFFICIENT_POINTS', 'x', { balance: 50 }), /點數餘額不足（目前 50 點）/],
    ['VALIDATION_FAILED', problem(422, 'VALIDATION_FAILED', 'x', { errors: [{ pointer: '/amount', code: 'RANGE', detail: 'amount 必須介於 1 與 50000' }] }), /內容有誤，請檢查點數/],
    ['RATE_LIMITED', { ...problem(429, 'RATE_LIMITED', 'x'), headers: { 'retry-after': '1800' } }, /約 30 分鐘後再試/],
  ]
  for (const [name, res, text] of cases) {
    test(name, async ({ page }) => {
      await setup(page, { post: () => res })
      await open(page)
      await amount(page).fill('500')
      await submit(page).click()
      await expect(page.getByRole('alert').filter({ hasText: text })).toBeVisible()
      await expect(submit(page)).toBeEnabled() // 可修正後重試
    })
  }

  test('WALLET_FROZEN：顯示帳戶已凍結請聯絡客服，並隱藏表單', async ({ page }) => {
    await setup(page, { post: () => problem(403, 'WALLET_FROZEN', 'x') })
    await open(page)
    await amount(page).fill('500')
    await submit(page).click()
    await expect(page.getByTestId('frozen')).toContainText('帳戶已凍結，暫時無法申請現金退款，請聯絡客服')
    await expect(amount(page)).toHaveCount(0)
    await expect(submit(page)).toHaveCount(0)
  })
})

test('不可退時：顯示說明而不是表單（餘額都是禮物卡 / 贊助點數）', async ({ page }) => {
  await setup(page, { wallet: { balance: 800, cash_refundable_balance: 0 } })
  await open(page)
  await expect(page.getByTestId('not-refundable')).toContainText('只有儲值購買的點數可以退現金，禮物卡與贊助取得的點數不可退')
  await expect(page.getByTestId('refundable')).toHaveText('0')
  await expect(amount(page)).toHaveCount(0)
  await expect(submit(page)).toHaveCount(0)
})

test('錢包頁入口：可退餘額 > 0 才有主要按鈕；否則顯示說明', async ({ page }) => {
  await setup(page)
  await signInPage(page)
  await page.goto('/me/wallet'); await hydrated(page)
  await expect(page.getByText('可退現金 3,000 點')).toBeVisible()
  await page.getByRole('link', { name: '申請現金退款' }).click()
  await expect(page).toHaveURL(/\/me\/refund$/)
  await expect(page.getByRole('heading', { name: '申請現金退款', exact: true }).first()).toBeVisible()
})

test('錢包頁沒有可退點數：沒有入口、顯示為何不可退', async ({ page }) => {
  await setup(page, { wallet: { cash_refundable_balance: 0 } })
  await signInPage(page)
  await page.goto('/me/wallet'); await hydrated(page)
  await expect(page.getByTestId('refund-none')).toContainText('目前沒有可退現金的點數')
  await expect(page.getByRole('link', { name: '申請現金退款' })).toHaveCount(0)
})

test('舊後端沒有 status 欄位：視為正常，不顯示凍結', async ({ page }) => {
  await setup(page, { wallet: { status: undefined } })
  await open(page)
  await expect(page.getByTestId('frozen')).toHaveCount(0)
  await expect(amount(page)).toBeVisible()
})

test('凍結中的錢包：/me/wallet、/me/topup、/me/refund 都顯示提示，退款表單隱藏', async ({ page }) => {
  await setup(page, { wallet: { status: 'frozen' } })
  await mock(page, /\/api\/v1\/wallet\/topups/, () => ok({ topups: [] }))
  await signInPage(page)

  await page.goto('/me/wallet'); await hydrated(page)
  await expect(page.getByText('錢包已被凍結，暫時無法贊助，請聯絡客服。')).toBeVisible()
  await expect(page.getByTestId('refund-none')).toContainText('錢包已凍結，暫時無法申請現金退款')
  await expect(page.getByRole('link', { name: '申請現金退款' })).toHaveCount(0)

  await page.goto('/me/topup'); await hydrated(page)
  await expect(page.getByTestId('frozen')).toContainText('錢包已凍結：暫時無法贊助或申請現金退款，請聯絡客服')

  await page.goto('/me/refund'); await hydrated(page)
  await expect(page.getByTestId('frozen')).toContainText('帳戶已凍結，暫時無法申請現金退款，請聯絡客服')
  await expect(amount(page)).toHaveCount(0)
})

test('我的申請：四種狀態徽章、金額、時間、被拒絕理由；有未完成的申請時顯示審核中', async ({ page }) => {
  const base = { completed_at: null, reject_reason: null }
  await setup(page, {
    requests: [
      { ...base, id: 'a', amount: 1000, status: 'requested', created_at: iso(-3_600_000) },
      { ...base, id: 'b', amount: 800, status: 'processing', created_at: iso(-7_200_000) },
      { ...base, id: 'c', amount: 500, status: 'completed', created_at: iso(-864e5), completed_at: iso(-36e5) },
      { ...base, id: 'd', amount: 300, status: 'rejected', created_at: iso(-2 * 864e5), completed_at: iso(-864e5), reject_reason: '退款帳戶資料不符' },
    ],
  })
  await open(page)
  const cards = page.locator('article[data-status]')
  await expect(cards).toHaveCount(4)
  await expect(cards.nth(0).locator('.c-badge')).toHaveText('待審核')
  await expect(cards.nth(1).locator('.c-badge')).toHaveText('處理中')
  await expect(cards.nth(2).locator('.c-badge')).toHaveText('已退款')
  await expect(cards.nth(2)).toContainText('完成')
  await expect(cards.nth(3).locator('.c-badge')).toHaveText('已拒絕')
  await expect(cards.nth(3)).toContainText('拒絕原因：退款帳戶資料不符')
  await expect(cards.nth(3)).toContainText('點數已退回你的錢包。')
  await expect(cards.nth(0)).toContainText('1,000 點')
  await expect(page.getByTestId('pending')).toHaveText('審核中：你有 2 筆退款申請正在處理。')
})

test('語系：英文介面的關鍵文案與錯誤訊息', async ({ page, context }) => {
  await context.addCookies([{ name: 'ws_locale', value: 'en', url: WEB }])
  await setup(page, { post: () => problem(409, 'NOT_CASH_REFUNDABLE', 'x', { refundable_points: 300 }) })
  await open(page)
  await expect(page.getByRole('heading', { name: 'Cash refund', exact: true }).first()).toBeVisible()
  await page.getByLabel('Points to refund').fill('500')
  await expect(page.getByTestId('summary')).toHaveText('500 points will be deducted from your wallet first. After approval, NT$ 500 is refunded.')
  await page.getByRole('button', { name: 'Submit request' }).click()
  await expect(page.getByRole('alert').filter({ hasText: "Only 300 points are refundable as cash (gift card and contribution points can't be refunded)." })).toBeVisible()
})
