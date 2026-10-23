// Q12：清單購買期限設定、我的捐助顯示期限、延長 7 天（一次）。真實 API。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser, signInPage } from './helpers'
import { hydrated } from './support/hydrate'

test('擁有者設定購買期限 14 天並保存', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  await loginBrowser(page.context(), owner)
  await page.goto('/lists/new'); await hydrated(page)
  await page.getByLabel(/清單名稱/).fill('Q12 期限清單')
  const ttl = page.getByLabel('購買期限')
  await expect(ttl.locator('option:checked')).toHaveText('7 天（預設）')
  await ttl.selectOption({ label: '14 天' })
  await page.getByRole('button', { name: '建立並新增品項' }).click()
  await page.waitForURL(/\/lists\/[^/]+\/edit/)
  const id = page.url().match(/\/lists\/([^/]+)\/edit/)![1]
  const w = await (await owner.api.get(`wishlists/${id}`)).json()
  expect(w.wishlist.claim_ttl_hours).toBe(336)

  // 清單設定表單讀回同一個值
  await page.getByText('清單設定').click()
  await expect(page.getByRole('combobox', { name: '購買期限' }).locator('option:checked')).toHaveText('14 天')
  await owner.api.dispose()
})

test('捐助者：我的捐助顯示期限，延長 7 天後日期後移且按鈕消失', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '奶嘴', qty: 2 }] })
  const donor = await signInPage(page, { tag: 'q12' })
  const r = await memberClaim(list.items[0]!.id, 1, donor)
  const before = new Date(r.claim.expires_at).getTime()
  expect(before - new Date(r.claim.created_at).getTime()).toBe(7 * 864e5) // 預設 7 天

  await page.goto('/me/claims'); await hydrated(page)
  const card = page.locator('.g-item', { hasText: '奶嘴' })
  const deadline = card.locator('.js-deadline')
  await expect(deadline).toContainText('請在')
  await expect(deadline).toContainText('前標記已購買')
  const oldText = await deadline.textContent()

  const extended = page.waitForResponse(res => res.request().method() === 'PATCH' && res.url().includes('/claims/'))
  await card.getByRole('button', { name: '延長 7 天' }).click()
  const body = await (await extended).json()
  expect(new Date(body.claim.expires_at).getTime() - before).toBe(7 * 864e5)
  await expect(deadline).not.toHaveText(oldText!)
  await expect(card.getByRole('button', { name: '延長 7 天' })).toHaveCount(0)

  await page.reload(); await hydrated(page)
  await expect(page.locator('.g-item', { hasText: '奶嘴' }).getByRole('button', { name: '延長 7 天' })).toHaveCount(0)
  await owner.api.dispose(); await donor.api.dispose()
})
