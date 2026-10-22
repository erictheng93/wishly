import { expect, test, type Page, type Route } from '@playwright/test'
import { createList, registerUser, signInPage } from './helpers'
import { hydrated } from './support/hydrate'

// F-05 / F-01 防呆 / F-10：認捐（會員）的錯誤處理（瀏覽器端請求用 page.route 注入錯誤；SSR 仍打真實 API）
const CLAIMS = '**/api/v1/items/*/claims'
const TOKEN_KEY = 'ws_guest_token'
const ls = (page: Page) => page.evaluate(k => localStorage.getItem(k), TOKEN_KEY)

let slug: string
test.beforeAll(async () => {
  const owner = await registerUser()
  slug = (await createList(owner, { items: [{ title: '奶瓶組', qty: 5 }] })).slug
  await owner.api.dispose()
})
async function open(page: Page, legacyToken = false) {
  await signInPage(page) // 認捐需登入會員
  await page.goto(`/s/${slug}`)
  await hydrated(page)
  if (legacyToken) await page.evaluate(([k]) => localStorage.setItem(k, 'legacy-guest-token'), [TOKEN_KEY])
  await page.getByRole('button', { name: '我要送' }).click()
}
const submit = (page: Page) => page.getByRole('button', { name: '確認捐助' }).click()
const json = (route: Route, status: number, body: object) => route.fulfill({
  status, contentType: 'application/json', body: JSON.stringify(body),
  headers: { 'access-control-allow-origin': route.request().headers().origin ?? '*', 'access-control-allow-credentials': 'true' },
})

test('登入已失效（401）：導到登入頁並帶回本頁，且不清掉既有的舊訪客 token', async ({ page }) => {
  await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback() : json(route, 401, { code: 'UNAUTHORIZED' }))
  await open(page, true)
  await page.route('**/api/v1/me', route => route.request().method() === 'OPTIONS' ? route.fallback() : json(route, 401, { code: 'UNAUTHORIZED' })) // session 真的過期了：登入頁才不會把人彈回來
  await submit(page)
  await expect(page).toHaveURL(new RegExp(`/login\\?redirect=(%2F|/)s(%2F|/)${slug}`))
  expect(await ls(page)).toBe('legacy-guest-token') // 舊訪客的既有捐助管理權不能因這次 401 丟掉
})

for (const [status, body, text] of [
  [404, { code: 'NOT_FOUND' }, '這個品項已不存在'],
  [410, { code: 'WISHLIST_REMOVED' }, '這份清單已被下架'],
  [429, { code: 'RATE_LIMITED' }, '稍後再試'],
  [503, { code: 'SERVICE_UNAVAILABLE' }, '服務暫時無法使用'],
] as const) {
  test(`${status} 沒有 detail 時依 code / 狀態顯示「${text}」`, async ({ page }) => {
    await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback() : json(route, status, body))
    await open(page)
    await submit(page)
    await expect(page.getByRole('dialog').getByRole('alert')).toContainText(text)
  })
}

test('留言欄位錯誤只出現一次（橫幅不重複），且表單沒有暱稱欄位', async ({ page }) => {
  await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback()
    : json(route, 422, { code: 'VALIDATION_FAILED', detail: '留言太長', errors: [{ pointer: '/note', detail: '留言太長' }] }))
  await open(page)
  const dlg = page.getByRole('dialog')
  await expect(dlg.getByLabel(/暱稱/)).toHaveCount(0)
  await expect(dlg.getByLabel('留言（選填）')).toHaveAttribute('maxlength', '200')
  await submit(page)
  await expect(page.getByText('留言太長')).toHaveCount(1)
})

test('超過上限的舊訪客加量：後端 403 LEGACY_CLAIM_READONLY 的說明原文顯示給使用者', async ({ page }) => {
  const detail = '舊訪客捐助無法增加數量，請以會員身分另行認捐'
  await page.route(CLAIMS, route => route.request().method() === 'OPTIONS' ? route.fallback() : json(route, 403, { code: 'LEGACY_CLAIM_READONLY', detail }))
  await open(page)
  await submit(page)
  await expect(page.getByRole('dialog').getByRole('alert')).toHaveText(detail)
})
