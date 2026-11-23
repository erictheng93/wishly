// 永久刪除清單：封存後輸入名稱確認、進行中認領擋下、送禮者端標示「清單已刪除」。真實 API。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser } from './helpers'

/** 擁有者以 API 封存（對應儀表板「封存」按鈕） */
async function archive(owner: Awaited<ReturnType<typeof registerUser>>, id: string) {
  expect((await owner.api.delete(`wishlists/${id}`)).status()).toBe(204)
}

test('封存 → 已封存檢視 → 輸入清單名稱才能永久刪除；卡片消失且重整後仍不在', async ({ page }) => {
  const owner = await registerUser({ tag: 'purge' })
  const list = await createList(owner, { items: [{ title: '奶瓶' }] })
  await archive(owner, list.id)
  await loginBrowser(page.context(), owner)

  await page.goto('/dashboard')
  await page.getByRole('button', { name: '已封存的清單' }).click()
  const card = page.locator('article.c-card', { hasText: list.title })
  await card.getByRole('button', { name: '永久刪除' }).click()
  const dlg = page.getByRole('dialog', { name: '永久刪除清單' })
  const ok = dlg.getByRole('button', { name: '確認永久刪除' })
  await expect(ok).toBeDisabled()
  await dlg.getByRole('textbox').fill(list.title)
  await expect(ok).toBeEnabled()
  await ok.click()
  await expect(dlg).toHaveCount(0)
  await expect(card).toHaveCount(0)
  await page.reload()
  await page.getByRole('button', { name: '已封存的清單' }).click()
  await expect(page.locator('article.c-card', { hasText: list.title })).toHaveCount(0)
})

test('草稿清單：卡片只有「永久刪除」沒有封存；視窗免輸入名稱，確認後清單消失', async ({ page }) => {
  const owner = await registerUser({ tag: 'purge' })
  const list = await createList(owner, { items: [{ title: '奶瓶' }], publish: false })
  await loginBrowser(page.context(), owner)

  await page.goto('/dashboard')
  const card = page.locator('article.c-card', { hasText: list.title })
  await expect(card.getByTestId('purge-btn')).toBeVisible()
  await expect(card.getByRole('button', { name: '封存' })).toHaveCount(0)
  await card.getByTestId('purge-btn').click()
  const dlg = page.getByRole('dialog', { name: '永久刪除清單' })
  await expect(dlg.getByRole('textbox')).toHaveCount(0)
  const ok = dlg.getByRole('button', { name: '確認永久刪除' })
  await expect(ok).toBeEnabled()
  await ok.click()
  await expect(dlg).toHaveCount(0)
  await expect(card).toHaveCount(0)
  await page.getByRole('button', { name: '已封存的清單' }).click()
  await expect(page.locator('article.c-card', { hasText: list.title })).toHaveCount(0)
  expect((await owner.api.get(`wishlists/${list.id}`)).status()).toBe(404)
})

test('有進行中的認領時永久刪除：視窗內顯示原因與數量，清單仍在', async ({ page }) => {
  const owner = await registerUser({ tag: 'purge' })
  const list = await createList(owner, { items: [{ title: '尿布', qty: 3 }] })
  await memberClaim(list.items[0]!.id, 1)
  await archive(owner, list.id)
  await loginBrowser(page.context(), owner)

  await page.goto('/dashboard')
  await page.getByRole('button', { name: '已封存的清單' }).click()
  await page.locator('article.c-card', { hasText: list.title }).getByRole('button', { name: '永久刪除' }).click()
  const dlg = page.getByRole('dialog', { name: '永久刪除清單' })
  await dlg.getByRole('textbox').fill(list.title)
  await dlg.getByRole('button', { name: '確認永久刪除' }).click()
  await expect(dlg.getByRole('alert')).toContainText('1 筆進行中的認領')
  await dlg.getByRole('button', { name: '取消' }).click()
  await expect(page.locator('article.c-card', { hasText: list.title })).toHaveCount(1)
})

test('送禮者在「我的捐助」看到清單已刪除，標題不再是連結', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const donor = await registerUser({ tag: 'donor' })
  const list = await createList(owner, { items: [{ title: '推車' }] })
  const { claim } = await memberClaim(list.items[0]!.id, 1, donor)
  // 認領須先完成（已購買 → 已送達）清單才能刪
  for (const status of ['purchased', 'delivered'])
    expect((await donor.api.patch(`claims/${claim.id}`, { data: { status } })).status()).toBe(200)
  await archive(owner, list.id)
  expect((await owner.api.delete(`wishlists/${list.id}`, { params: { permanent: 'true' } })).status()).toBe(204)

  await loginBrowser(page.context(), donor)
  await page.goto('/me/claims')
  await expect(page.getByTestId('list-deleted')).toContainText('清單已刪除')
  await expect(page.getByRole('heading', { name: list.title })).toBeVisible()
  await expect(page.locator('main a[href^="/s/"]')).toHaveCount(0)
})
