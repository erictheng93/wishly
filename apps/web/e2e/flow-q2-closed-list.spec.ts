import { expect, test } from '@playwright/test'
import { createList, memberClaim, registerUser, signInPage } from './helpers'
import { hydrated } from './support/hydrate'

// Q2（CTO 決議放寬）：清單結束後，捐助者仍可標記已購買 / 已送達或取消；不能改數量、不能延長購買期限
test('清單結束後：我的捐助仍可標記已購買 / 已送達、顯示提示、不能延長，仍可取消', async ({ page }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '奶瓶組', qty: 2 }, { title: '口水巾', qty: 1 }] })
  const donor = await signInPage(page, { tag: 'q2' })
  await memberClaim(list.items[0].id, 1, donor)
  await memberClaim(list.items[1].id, 1, donor)
  const card = page.locator('.g-card.g-item', { hasText: '奶瓶組' })
  const other = page.locator('.g-card.g-item', { hasText: '口水巾' })
  const hint = '清單已結束：仍可標記已購買 / 已送達或取消，無法再更改數量'

  await test.step('清單進行中：可標記已購買、可延長', async () => {
    await page.goto('/me/claims'); await hydrated(page)
    await expect(card.getByRole('button', { name: '標記已購買' })).toBeVisible()
    await expect(card.getByRole('button', { name: '延長 7 天' })).toBeVisible()
    await expect(card.getByText(hint)).toHaveCount(0)
  })

  await test.step('擁有者結束清單', async () => {
    const cur = await (await owner.api.get(`wishlists/${list.id}`)).json()
    const r = await owner.api.patch(`wishlists/${list.id}`, { data: { status: 'closed', expected_updated_at: cur.wishlist.updated_at } })
    expect(r.ok(), await r.text()).toBeTruthy()
  })

  await test.step('結束後：有提示、沒有延長；可標記已購買 → 已送達', async () => {
    await page.reload(); await hydrated(page)
    await expect(card.getByText(hint)).toBeVisible()
    await expect(card.getByRole('button', { name: '延長 7 天' })).toHaveCount(0)
    await card.getByRole('button', { name: '標記已購買' }).click()
    await expect(card.getByText('已購買', { exact: true })).toBeVisible()
    await expect(card.getByText(/前標記已購買/)).toHaveCount(0) // 已購買後不再逾期，不顯示期限
    await card.getByRole('button', { name: '標記已送達' }).click()
    await expect(card.getByText('已送達', { exact: true })).toBeVisible()
    await expect(page.getByRole('alert')).toHaveCount(0)
  })

  await test.step('結束後仍可取消', async () => {
    page.once('dialog', d => d.accept())
    await other.getByRole('button', { name: '取消', exact: true }).click()
    await expect(other).toHaveCount(0)
    await expect(page.getByText(/過去的紀錄（1）/)).toBeVisible()
  })

  await owner.api.dispose(); await donor.api.dispose()
})
