import { expect, test } from '@playwright/test'
import { createList, memberClaim, registerUser, signInPage } from './helpers'
import { hydrated } from './support/hydrate'

// Q2：清單結束後，捐助者在「我的捐助」只能取消，不能再標記已購買 / 已送達
test('清單結束後：我的捐助不顯示「標記已購買」，顯示提示，仍可取消', async ({ page }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '奶瓶組', qty: 2 }] })
  const donor = await signInPage(page, { tag: 'q2' })
  await memberClaim(list.items[0].id, 1, donor)
  const card = page.locator('.g-card.g-item', { hasText: '奶瓶組' })

  await test.step('清單進行中：可標記已購買', async () => {
    await page.goto('/me/claims'); await hydrated(page)
    await expect(card.getByRole('button', { name: '標記已購買' })).toBeVisible()
    await expect(card.getByText('清單已結束，只能取消')).toHaveCount(0)
  })

  await test.step('擁有者結束清單', async () => {
    const cur = await (await owner.api.get(`wishlists/${list.id}`)).json()
    const r = await owner.api.patch(`wishlists/${list.id}`, { data: { status: 'closed', expected_updated_at: cur.wishlist.updated_at } })
    expect(r.ok(), await r.text()).toBeTruthy()
  })

  await test.step('結束後：沒有推進按鈕、有提示，可取消', async () => {
    await page.reload(); await hydrated(page)
    await expect(card.getByText('清單已結束，只能取消')).toBeVisible()
    await expect(card.getByRole('button', { name: '標記已購買' })).toHaveCount(0)
    await expect(card.getByRole('button', { name: '標記已送達' })).toHaveCount(0)
    page.once('dialog', d => d.accept())
    await card.getByRole('button', { name: '取消', exact: true }).click()
    await expect(card).toHaveCount(0)
    await expect(page.getByText(/過去的紀錄（1）/)).toBeVisible()
  })

  await owner.api.dispose(); await donor.api.dispose()
})
