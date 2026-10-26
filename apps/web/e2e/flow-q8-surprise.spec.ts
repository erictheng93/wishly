import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser } from './helpers'

// Q8：驚喜清單——訪客看到「先別透露」提示；建立者看得到數量與人數，看不到是誰送的
test('驚喜清單：訪客看到提示，建立者看得到數量但看不到捐助者', async ({ browser }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { surprise: true, showNames: true, items: [{ title: '拍立得', qty: 3 }] })
  const donor = await registerUser({ tag: 'donor', name: '偷偷送禮人' })
  await memberClaim(list.items[0]!.id, 2, donor)

  const guestCtx = await browser.newContext(), ownerCtx = await browser.newContext()
  await loginBrowser(ownerCtx, owner)
  const gp = await guestCtx.newPage(), op = await ownerCtx.newPage()

  await test.step('訪客分享頁：顯示驚喜提示、看得到數量，看不到捐助者（鎖定期間 API 不給姓名）', async () => {
    await gp.goto(`/s/${list.slug}`)
    await expect(gp.getByText('這是驚喜清單：活動日前建立者看不到誰送了什麼，請先別透露喔')).toBeVisible()
    await expect(gp.locator('li.g-card', { hasText: '拍立得' })).toContainText('2 / 3')
    await expect(gp.getByText('偷偷送禮人')).toHaveCount(0)
  })

  await test.step('建立者分享頁：看得到數量、看不到是誰，不顯示訪客提示、不能捐助', async () => {
    await op.goto(`/s/${list.slug}`)
    await expect(op.getByText('你正以建立者身分瀏覽')).toBeVisible()
    await expect(op.getByText('請先別透露喔')).toHaveCount(0)
    const card = op.locator('li.g-card', { hasText: '拍立得' })
    await expect(card).toContainText('2 / 3')
    await expect(card).toContainText('建立者不能送自己清單上的禮物')
    await expect(op.getByText('偷偷送禮人')).toHaveCount(0)
  })

  await test.step('建立者進度頁：x / y 與「N 人認領」，名單於活動日解鎖', async () => {
    await op.goto(`/lists/${list.id}/progress`)
    await expect(op.getByRole('status').filter({ hasText: '名單會在活動日當天解鎖' })).toBeVisible()
    const card = op.locator('article', { hasText: '拍立得' })
    await expect(card).toContainText('2 / 3')
    await expect(card).toContainText('1 人認領')
    await expect(op.getByText('偷偷送禮人')).toHaveCount(0)
    await expect(op.getByRole('button', { name: '標為已送達' })).toHaveCount(0)
  })

  await test.step('儀表板 API 本身不含捐助者', async () => {
    const body = await (await owner.api.get(`wishlists/${list.id}/dashboard`)).json()
    expect(body.surprise_locked).toBe(true)
    expect(body.claims).toEqual([])
    expect(body.items[0]).toMatchObject({ qty_claimed: 2, claimer_count: 1 })
    expect(JSON.stringify(body)).not.toContain('偷偷送禮人')
  })
  await Promise.all([guestCtx.close(), ownerCtx.close(), owner.api.dispose(), donor.api.dispose()])
})
