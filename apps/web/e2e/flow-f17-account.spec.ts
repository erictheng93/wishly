// 流程驗證 F17（docs/02 帳號刪除與資料匯出）：真實 API。
import { expect, test } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { createList, loginBrowser, makeStaff, memberClaim, registerUser } from './helpers'
import { API } from './support/env'
import { hydrated } from './support/hydrate'

test('匯出資料：下載 JSON 含個人資料、清單與捐助；每日第 4 次顯示限制說明（看得到）', async ({ page }) => {
  const me = await registerUser({ tag: 'export' })
  const list = await createList(me, { title: `匯出 ${Date.now() % 1e5}` })
  const other = await registerUser({ tag: 'other' })
  const theirs = await createList(other)
  await memberClaim(theirs.items[0]!.id, 1, me)
  await loginBrowser(page.context(), me)

  await page.goto('/settings'); await hydrated(page)
  const btn = page.getByRole('button', { name: '匯出我的資料' })
  const [dl] = await Promise.all([page.waitForEvent('download'), btn.click()])
  expect(dl.suggestedFilename()).toMatch(/^wishly-export-\d{4}-\d{2}-\d{2}\.json$/)
  const data = JSON.parse(readFileSync((await dl.path())!, 'utf8'))
  expect(data.user.email).toBe(me.email)
  expect(data.identities).toEqual([{ provider: 'email' }])
  expect(data.wishlists.map((w: any) => w.title)).toContain(list.title)
  expect(data.claims).toHaveLength(1)

  for (let i = 0; i < 2; i++) await Promise.all([page.waitForEvent('download'), btn.click()])
  await btn.click()
  const alert = page.getByRole('alert').filter({ hasText: '匯出每日最多 3 次' })
  await expect(alert).toBeVisible()
  await expect(alert).toBeInViewport()
})

test('刪除帳號：需輸入確認字與密碼；密碼錯誤擋下；有點數時擋下並說明；刪除後登出、清單下架、無法再登入', async ({ page }) => {
  const me = await registerUser({ tag: 'delete' })
  const list = await createList(me)
  const staff = await registerUser({ tag: 'staff' }); makeStaff(staff.email)
  const meJson = await (await me.api.get('me')).json()
  const meId = meJson.user?.id ?? meJson.id
  const grant = async (points: number) => {
    const r = await staff.api.post('admin/wallets/grants', { data: { user_id: meId, points, reason: 'e2e' }, headers: { 'Idempotency-Key': crypto.randomUUID() } })
    expect(r.ok(), await r.text()).toBe(true)
  }
  await grant(50)
  await loginBrowser(page.context(), me)

  await page.goto('/settings'); await hydrated(page)
  await page.getByRole('button', { name: '刪除我的帳號' }).click()
  const dlg = page.getByRole('dialog', { name: '確定要刪除帳號嗎？' })
  const ok = dlg.getByRole('button', { name: '永久刪除' })
  await expect(ok).toBeDisabled()
  await dlg.getByLabel('請輸入「刪除」以確認').fill('刪')
  await expect(ok).toBeDisabled()
  await dlg.getByLabel('請輸入「刪除」以確認').fill('刪除')
  await expect(ok).toBeDisabled() // 有密碼登入者還要輸入目前密碼
  const pw = dlg.getByLabel('目前密碼', { exact: true })
  await pw.fill('wrong-password')
  await ok.click()
  await expect(dlg.getByText('密碼不正確')).toBeVisible()
  expect((await me.api.get('me')).status()).toBe(200)
  await pw.fill(me.password)
  await ok.click()
  await expect(dlg.getByText('50 點餘額')).toBeVisible()
  await expect(dlg).toBeVisible()

  // 營運扣回點數後可刪
  await grant(-50)
  await ok.click()
  await expect(page).toHaveURL(/\/$/)
  expect((await me.api.get('me')).status()).toBe(401)
  expect((await page.request.get(`${API}/api/v1/public/wishlists/${list.slug}`)).status()).toBe(404)
  const login = await me.api.post('auth/login', { data: { email: me.email, password: me.password } })
  expect(login.status()).toBe(401)
})
