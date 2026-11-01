import { expect, test, type Page } from '@playwright/test'
import { loginBrowser, makeStaff, registerUser } from './helpers'
import { hydrated } from './support/hydrate'
import { iso, mock, ok, problem } from './support/mockApi'

// 金流後台：登入與 staff 權限走真後端，/admin/finance/* 與 /admin/topups 以 page.route 模擬。
async function asStaff(page: Page) {
  const u = await registerUser({ tag: 'fin' })
  makeStaff(u.email)
  await loginBrowser(page.context(), u)
  return u
}
const bucket = (count: number, amount: number) => ({ count, amount })
const kpi = (gross: number, payers: number, first_time: number) => ({ gross, payers, first_time })
const overview = (days: number) => ({
  range: { from: '2026-10-01', to: '2026-10-31', days },
  todo: { disputed: bucket(2, 5000), cash_refunds: bucket(1, 300), debts: bucket(0, 0), pending: bucket(3, 900) },
  kpis: { cur: kpi(12000, 4, 2), prev: kpi(6000, 2, 1) },
  daily: [{ date: '2026-10-30', gross: 5000 }, { date: '2026-10-31', gross: 7000 }],
})
const payers = {
  range: { from: '2026-10-01', to: '2026-10-31', days: 30 },
  payers: 2, first_time: 1, repeat_rate: 0.5, gross: 9000,
  buckets: [{ key: 'lt500', count: 1, amount: 100 }, { key: '3000_9999', count: 1, amount: 8000 }],
  top: [{ user: { id: 'u-top', display_name: '大金主', email: 'top@example.com' }, amount: 8000, count: 3, first_paid_at: iso(-9e8), last_paid_at: iso(-1e6) }],
}
const topup = (id: string, o: Record<string, any> = {}) => ({ id, amount: 1000, method: 'card', status: 'paid', provider: 'ecpay', provider_txn_id: `T-${id}`, created_at: iso(-864e5), paid_at: iso(-864e5), user: { id: 'u-top', display_name: '大金主', email: 'top@example.com' }, wallet: { id: 'w', status: 'active' }, ...o })

test('總覽顯示待處理與核心數字，切換區間會帶 days 重查', async ({ page }) => {
  await asStaff(page)
  const calls = await mock(page, /\/api\/v1\/admin\/finance\/overview(\?|$)/, c => ok(overview(Number(c.url.searchParams.get('days')))))
  await page.goto('/admin/finance'); await hydrated(page)
  await expect(page.getByRole('region', { name: '待處理' })).toContainText('爭議中')
  await expect(page.getByRole('region', { name: '核心數字' })).toContainText('NT$ 12,000')
  expect(calls[0].url.searchParams.get('days')).toBe('30')
  await page.getByRole('button', { name: '7 天' }).click()
  await expect.poll(() => calls.at(-1)?.url.searchParams.get('days')).toBe('7')
})

test('前 20 名點擊後，交易明細帶 user_id 篩選，可清除', async ({ page }) => {
  await asStaff(page)
  await mock(page, /\/api\/v1\/admin\/finance\/payers(\?|$)/, () => ok(payers))
  const list = await mock(page, /\/api\/v1\/admin\/topups(\?|$)/, () => ok({ data: [topup('t1')], next_cursor: null }))
  await page.goto('/admin/finance?tab=payers'); await hydrated(page)
  await page.getByRole('link', { name: '大金主' }).click()
  await expect(page).toHaveURL(/tab=transactions&user_id=u-top/)
  await expect(page.locator('article[data-status]')).toHaveCount(1)
  expect(list.at(-1)!.url.searchParams.get('user_id')).toBe('u-top')
  await page.getByRole('button', { name: '顯示全部用戶' }).click()
  await expect.poll(() => list.at(-1)!.url.searchParams.get('user_id')).toBeNull()
  await expect(page).not.toHaveURL(/user_id/)
})

test('交易明細：渠道與搜尋帶入查詢；刪除用戶（無 email）不留空白分隔', async ({ page }) => {
  await asStaff(page)
  const list = await mock(page, /\/api\/v1\/admin\/topups(\?|$)/, () => ok({ data: [topup('t2', { user: { id: 'u9', display_name: '路人', email: null } })], next_cursor: null }))
  await page.goto('/admin/finance?tab=transactions&status=pending'); await hydrated(page)
  await expect.poll(() => list.length).toBeGreaterThan(0)
  expect(list[0].url.searchParams.get('status')).toBe('pending')
  await expect(page.getByLabel('狀態')).toHaveValue('pending')
  await page.getByLabel('渠道').selectOption('atm')
  await expect.poll(() => list.at(-1)!.url.searchParams.get('method')).toBe('atm')
  await page.getByLabel('搜尋交易').fill('ming'); await page.keyboard.press('Enter')
  await expect.poll(() => list.at(-1)!.url.searchParams.get('q')).toBe('ming')
  await expect(page.locator('article[data-status]').first()).toContainText('路人')
})

test('帶 user_id 開啟後選了狀態，清除用戶仍保留狀態篩選', async ({ page }) => {
  await asStaff(page)
  const list = await mock(page, /\/api\/v1\/admin\/topups(\?|$)/, () => ok({ data: [topup('t5')], next_cursor: null }))
  await page.goto('/admin/finance?tab=transactions&user_id=u-top'); await hydrated(page)
  await page.getByLabel('狀態').selectOption('disputed')
  await expect.poll(() => list.at(-1)!.url.searchParams.get('status')).toBe('disputed')
  expect(list.at(-1)!.url.searchParams.get('user_id')).toBe('u-top')
  await page.getByRole('button', { name: '顯示全部用戶' }).click()
  await expect.poll(() => list.at(-1)!.url.searchParams.get('user_id')).toBeNull()
  expect(list.at(-1)!.url.searchParams.get('status')).toBe('disputed')
  await expect(page.getByLabel('狀態')).toHaveValue('disputed')
})

test('/admin/topups 導向交易明細分頁', async ({ page }) => {
  await asStaff(page)
  await mock(page, /\/api\/v1\/admin\/topups(\?|$)/, () => ok({ data: [], next_cursor: null }))
  await page.goto('/admin/topups')
  await expect(page).toHaveURL(/\/admin\/finance\?tab=transactions$/)
  await expect(page.getByRole('button', { name: '交易明細' })).toHaveAttribute('aria-current', 'page')
})

test('API 失敗時顯示錯誤與重試', async ({ page }) => {
  await asStaff(page)
  let fail = true // ofetch 對 GET 的 5xx 會自動重試，所以用旗標而非呼叫次數
  const calls = await mock(page, /\/api\/v1\/admin\/finance\/overview(\?|$)/, () => fail ? problem(500, 'INTERNAL', '內部錯誤') : ok(overview(30)))
  await page.goto('/admin/finance'); await hydrated(page)
  await expect(page.getByRole('alert')).toBeVisible()
  fail = false
  const before = calls.length
  await page.getByRole('button', { name: '重試' }).click()
  await expect(page.getByRole('region', { name: '核心數字' })).toContainText('NT$ 12,000')
  expect(calls.length).toBeGreaterThan(before)
})

test('後台首頁有金流後台入口，可導向', async ({ page }) => {
  await asStaff(page)
  await mock(page, /\/api\/v1\/admin\/finance\/overview(\?|$)/, () => ok(overview(30)))
  await page.goto('/admin'); await hydrated(page)
  await page.locator('a[href^="/admin/finance"]').click()
  await expect(page).toHaveURL(/\/admin\/finance$/)
})

test('待處理的現金退款連結帶 status=all，退款頁以全部查詢', async ({ page }) => {
  await asStaff(page)
  await mock(page, /\/api\/v1\/admin\/finance\/overview(\?|$)/, () => ok(overview(30)))
  const calls = await mock(page, /\/api\/v1\/admin\/cash-refunds(\?|$)/, () => ok({ data: [], next_cursor: null }))
  await page.goto('/admin/finance'); await hydrated(page)
  await page.getByRole('region', { name: '待處理' }).getByRole('link', { name: /待處理退款/ }).click()
  await expect(page).toHaveURL(/\/admin\/cash-refunds\?status=all$/)
  await expect.poll(() => calls.length).toBeGreaterThan(0)
  expect(calls[0].url.searchParams.get('status')).toBeNull()
})
