// 流程驗證 F23（D65 生日清單通知）：新增 / 編輯清單的開關、設定頁通知開關、退訂 'b' 權杖。真實 API。
import { expect, test, type Page } from '@playwright/test'
import { createHmac } from 'node:crypto'
import { loginBrowser, registerUser, type User } from './helpers'
import { hydrated } from './support/hydrate'

const LABEL = '生日前 7 天通知看得到這份清單的朋友'
const openSettings = (page: Page) => page.getByText('清單設定', { exact: true }).click() // 編輯頁設定預設收合
const sw = (page: Page) => page.getByLabel(LABEL)

async function member(page: Page, birthday?: { date: string, vis?: string }) {
  const u = await registerUser({ tag: 'bday' })
  if (birthday) expect((await u.api.patch('me', { data: { birthday: birthday.date, birthday_visibility: birthday.vis ?? 'friends' } })).ok()).toBe(true)
  await loginBrowser(page.context(), u)
  return u
}
async function newList(page: Page, title: string, opts: { tick?: boolean } = {}) {
  await page.goto('/lists/new'); await hydrated(page)
  await page.getByLabel(/清單名稱/).fill(title)
  if (opts.tick) await sw(page).check()
  await page.getByRole('button', { name: '建立並新增品項' }).click()
  await page.waitForURL(/\/lists\/[0-9a-f-]+\/edit/)
  await hydrated(page)
  await openSettings(page)
  return page.url().match(/lists\/([0-9a-f-]+)\/edit/)![1]!
}

test('沒填生日：開關停用，顯示原因與「前往設定」連結', async ({ page }) => {
  await member(page)
  await page.goto('/lists/new'); await hydrated(page)
  await expect(sw(page)).toBeDisabled()
  await expect(page.getByText('請先在設定填寫生日才能開啟。')).toBeVisible()
  await expect(page.getByRole('link', { name: '前往設定' })).toHaveAttribute('href', '/settings')
})

test('設定頁填生日後，新增清單勾選 → 建立成功 → 編輯頁顯示已開啟', async ({ page }) => {
  await member(page)
  await page.goto('/settings'); await hydrated(page)
  await page.getByLabel('月', { exact: true }).selectOption('06')
  await page.getByLabel('日', { exact: true }).selectOption('15')
  const form = page.locator('form', { has: page.getByLabel('帳號代號（個人頁網址，可留空）') })
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('已儲存')).toBeVisible()

  await page.goto('/lists/new'); await hydrated(page)
  await expect(sw(page)).toBeEnabled()
  await expect(page.getByText('會通知你的所有好友。')).toBeVisible()
  await newList(page, 'bday-a', { tick: true })
  await expect(sw(page)).toBeChecked()
})

test('可見度切 private 自動取消並停用；切摯友說明改摯友版本', async ({ page }) => {
  await member(page, { date: '06-15' })
  await page.goto('/lists/new'); await hydrated(page)
  await sw(page).check()
  await page.getByRole('radio', { name: /私人/ }).check()
  await expect(sw(page)).toBeDisabled()
  await expect(sw(page)).not.toBeChecked()
  await expect(page.getByText('私人清單不會通知任何人，請先改成其他可見度。')).toBeVisible()
  await page.getByRole('radio', { name: /摯友|指定/ }).first().check()
  await expect(sw(page)).toBeEnabled()
  await expect(page.getByText('只會通知你在這份清單勾選的摯友。')).toBeVisible()
})

test('開第二份清單的生日通知，第一份維持開啟', async ({ page }) => {
  await member(page, { date: '06-15' })
  const first = await newList(page, 'bday-1', { tick: true })
  await expect(sw(page)).toBeChecked()
  await newList(page, 'bday-2', { tick: true })
  await expect(sw(page)).toBeChecked()
  await page.goto(`/lists/${first}/edit`); await hydrated(page); await openSettings(page)
  await expect(sw(page)).toBeChecked()
})

test('生日可見度改 private：編輯頁開關停用且已關閉', async ({ page }) => {
  const u = await member(page, { date: '06-15' })
  const id = await newList(page, 'bday-v', { tick: true })
  expect((await u.api.patch('me', { data: { birthday_visibility: 'private' } })).ok()).toBe(true)
  await page.goto(`/lists/${id}/edit`); await hydrated(page); await openSettings(page)
  await expect(sw(page)).toBeDisabled()
  await expect(sw(page)).not.toBeChecked()
  await expect(page.getByText('你的生日可見度為「只有自己」，請先在設定調整。')).toBeVisible()
})

test('設定頁社交與生日 Email 開關重新整理後保持', async ({ page }) => {
  await member(page)
  await page.goto('/settings'); await hydrated(page)
  const box = page.getByLabel('社交與生日 Email')
  await expect(box).toBeChecked()
  const saved = page.waitForResponse(r => r.url().endsWith('/me') && r.request().method() === 'PATCH')
  await box.uncheck()
  expect((await saved).ok()).toBe(true)
  await page.reload(); await hydrated(page)
  await expect(page.getByLabel('社交與生日 Email')).not.toBeChecked()
})

test("退訂 'b' 權杖：確認後設定頁開關變關閉", async ({ page }) => {
  const u: User = await member(page)
  const id = (await (await u.api.get('me')).json()).id as string
  const msg = `b:${id}:${Math.floor(Date.now() / 1000) + 86400}` // 開發環境固定 UNSUB_SECRET
  const token = `${Buffer.from(msg).toString('base64url')}.${createHmac('sha256', 'dev-unsub-secret').update(msg).digest('hex')}`
  await page.goto(`/unsubscribe?token=${token}`); await hydrated(page)
  await page.getByRole('button', { name: '確定退訂' }).click()
  await expect(page.getByRole('heading', { name: '已退訂' })).toBeVisible()
  await page.goto('/settings'); await hydrated(page)
  await expect(page.getByLabel('社交與生日 Email')).not.toBeChecked()
})
