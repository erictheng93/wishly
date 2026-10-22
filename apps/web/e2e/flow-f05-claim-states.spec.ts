// 流程驗證 F5（docs/02 捐助狀態變更與取消）：真實 API，涵蓋捐助者與擁有者兩端的 UI 操作。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser, signInPage } from './helpers'

test('捐助者：已購買 → 已送達；連點不出錯；送達後不可取消', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '尿布', qty: 2 }] })
  const donor = await signInPage(page)
  await memberClaim(list.items[0]!.id, 1, donor)

  await page.goto('/me/claims')
  const card = page.locator('.g-item', { hasText: '尿布' })
  await expect(card.getByText('已捐助')).toBeVisible()
  // 連點同一按鈕：第二個請求是同一目標狀態，後端冪等回 200，不可出現「不允許的狀態轉換」
  const mark = card.getByRole('button', { name: '標記已購買' })
  const patches: number[] = []
  page.on('response', r => { if (r.request().method() === 'PATCH' && r.url().includes('/claims/')) patches.push(r.status()) })
  await mark.evaluate((b: HTMLButtonElement) => { b.click(); b.click() })
  await expect.poll(() => patches.length).toBe(2)
  expect(patches).toEqual([200, 200])
  await expect(card.getByText('已購買')).toBeVisible()
  await expect(page.getByRole('alert')).toHaveCount(0)

  await card.getByRole('button', { name: '標記已送達' }).click()
  await expect(card.getByText('已送達')).toBeVisible()
  await expect(card.getByRole('button', { name: '取消' })).toHaveCount(0)
  await page.reload()
  await expect(page.locator('.g-item', { hasText: '尿布' }).getByText('已送達')).toBeVisible()
})

test('擁有者：進度頁標記已送達、取消他人捐助（需確認）後數量回補', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '奶粉', qty: 3 }] })
  await memberClaim(list.items[0]!.id, 1)
  await memberClaim(list.items[0]!.id, 2)
  await loginBrowser(page.context(), owner)

  await page.goto(`/lists/${list.id}/progress`)
  const rows = page.locator('article', { hasText: '奶粉' }).getByRole('listitem')
  await expect(rows).toHaveCount(2)
  await rows.filter({ hasText: '×1' }).getByRole('button', { name: '標為已送達' }).click()
  await expect(rows.filter({ hasText: '×1' }).getByText('已送達')).toBeVisible()

  page.once('dialog', d => d.dismiss())
  await rows.filter({ hasText: '×2' }).getByRole('button', { name: '取消捐助' }).click()
  await expect(rows.filter({ hasText: '×2' }).getByText('已捐助')).toBeVisible()

  page.once('dialog', d => d.accept())
  await rows.filter({ hasText: '×2' }).getByRole('button', { name: '取消捐助' }).click()
  await expect(rows.filter({ hasText: '×2' }).getByText('已取消')).toBeVisible()
  await expect(page.getByText('數量 1 / 3')).toBeVisible()
})

test('非擁有者開別人的進度頁：顯示找不到，而不是「連線失敗」', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner)
  await signInPage(page)
  await page.goto(`/lists/${list.id}/progress`)
  await expect(page.getByRole('alert')).toContainText('找不到')
  await expect(page.getByRole('alert')).not.toContainText('連線失敗')
})
