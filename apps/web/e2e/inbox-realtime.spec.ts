// 站內通知即時更新：不 reload，新通知就出現徽章與 toast；另一分頁全部已讀後，這頁徽章歸零。真實 API + worker（每秒領取）。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser, signInPage, sql } from './helpers'
import { hydrated } from './support/hydrate'

test('新通知即時出現徽章與 toast；他處全部已讀後徽章歸零', async ({ page, context }) => {
  const u = await registerUser({ tag: 'inbox' })
  await loginBrowser(context, u)
  await page.goto('/dashboard'); await hydrated(page)
  const bell = page.getByRole('button', { name: /^通知/ })
  await expect(bell).toBeVisible()
  await expect(bell).not.toContainText('1')

  const id = (await (await u.api.get('me')).json()).id as string
  // Gifts 類、pending：worker 領取後寫入站內並發訊號
  sql(`INSERT INTO notifications (user_id, channel, kind, payload) VALUES ('${id}', 'email', 'owner.funded', '{"title":"E2E 心願","item":"E2E 心願"}'::jsonb)`)

  await expect(bell).toHaveAttribute('aria-label', /1 則未讀/, { timeout: 15_000 })
  await expect(page.locator('.c-toast')).toBeVisible()

  // 展開面板：Esc 關閉並把焦點還給鈴鐺
  await bell.click()
  await expect(bell).toHaveAttribute('aria-expanded', 'true')
  await page.keyboard.press('Escape')
  await expect(bell).toHaveAttribute('aria-expanded', 'false')
  await expect(bell).toBeFocused()

  // 第二分頁全部已讀 → 第一分頁不 reload，徽章歸零
  const page2 = await context.newPage()
  await page2.goto('/me/notifications'); await hydrated(page2)
  await page2.getByRole('button', { name: '全部標為已讀' }).click()
  await expect(bell).not.toHaveAttribute('aria-label', /未讀/, { timeout: 15_000 })
  await u.api.dispose()
})

test('訪客版頁首（/explore）登入後也即時收到；手機寬度面板不溢出', async ({ page, context }) => {
  await page.setViewportSize({ width: 360, height: 700 })
  const u = await registerUser({ tag: 'inbox-g' })
  await loginBrowser(context, u)
  await page.goto('/dashboard'); await hydrated(page) // 真實登入後會留下 ws_hint，訪客頁才去抓 /me
  await expect(page.getByRole('button', { name: /^通知/ })).toBeVisible()
  await page.goto('/explore'); await hydrated(page)
  const bell = page.getByRole('button', { name: /^通知/ })
  await expect(bell).toBeVisible()
  const id = (await (await u.api.get('me')).json()).id as string
  sql(`INSERT INTO notifications (user_id, channel, kind, payload) VALUES ('${id}', 'email', 'owner.funded', '{"title":"E2E 心願","item":"E2E 心願"}'::jsonb)`)
  await expect(bell).toHaveAttribute('aria-label', /1 則未讀/, { timeout: 15_000 })
  await page.screenshot({ path: process.env.SHOT_DIR ? `${process.env.SHOT_DIR}/toast-360.png` : 'test-results/toast-360.png' })
  await page.getByRole('button', { name: /關閉|Close/ }).first().click()
  await bell.click()
  const panel = page.locator('.c-bell-pop')
  await expect(panel).toBeVisible()
  await expect(panel).toHaveCSS('opacity', '1')
  await expect(panel).toContainText('集資達標')
  const box = await panel.boundingBox()
  expect(box!.x).toBeGreaterThanOrEqual(0)
  expect(box!.x + box!.width).toBeLessThanOrEqual(360)
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(360)
  await page.screenshot({ path: process.env.SHOT_DIR ? `${process.env.SHOT_DIR}/panel-360.png` : 'test-results/panel-360.png' })
  await u.api.dispose()
})

test('認領確認：捐助者不用等、不用重新整理就看到站內通知', async ({ page }) => {
  const owner = await registerUser({ tag: 'inbox-o' })
  const list = await createList(owner, { items: [{ title: '即時禮物', qty: 3 }] })
  const donor = await signInPage(page)
  await page.goto('/dashboard'); await hydrated(page)
  const bell = page.getByRole('button', { name: /^通知/ })
  await expect(bell).toBeVisible()
  await memberClaim(list.items[0]!.id, 1, donor)
  await expect(page.locator('.c-toast')).toContainText('即時禮物', { timeout: 5_000 })
  await expect(bell).toHaveAttribute('aria-label', /1 則未讀/)
})
