import { expect, test } from '@playwright/test'
import { createList, memberClaim, registerUser, signInPage } from './helpers'

test('防超賣：兩位會員同時搶最後 1 件，恰好 1 位成功', async ({ browser, request }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '限量玩偶', qty: 1 }] })

  // 兩個獨立 context = 兩位互不相識的會員（各自註冊、各自的 session）
  const [ca, cb] = await Promise.all([browser.newContext(), browser.newContext()])
  const [a, b] = await Promise.all([ca.newPage(), cb.newPage()])
  const pages = [a, b]
  const members = await Promise.all(pages.map(p => signInPage(p)))
  const success = (p: typeof a) => p.getByRole('heading', { name: '✓ 這份交給你了！' })
  const lost = (p: typeof a) => p.getByRole('alert').getByText('剛剛被別人認領完了')

  await test.step('兩人都打開分享頁並填好捐助表單（尚未送出）', async () => {
    await Promise.all(pages.map(async (p) => {
      await p.goto(`/s/${list.slug}`)
      await p.locator('li.g-card', { hasText: '限量玩偶' }).getByRole('button', { name: '我來送' }).click()
    }))
  })

  await test.step('同時按下確定要送', async () => {
    await Promise.all(pages.map(p => p.getByRole('button', { name: '確定要送' }).click()))
  })

  await test.step('恰好一人成功、另一人看到「剛剛被別人認領完了」', async () => {
    await expect.poll(async () => (await success(a).isVisible()) + (await success(b).isVisible())).toBe(1)
    const [winner, loser] = (await success(a).isVisible()) ? [a, b] : [b, a]
    await expect(lost(loser)).toBeVisible()
    await expect(success(loser)).toHaveCount(0)
    await expect(loser.getByRole('dialog')).toBeHidden() // 約 1.2 秒後自動關閉
    await expect(loser.locator('li.g-card', { hasText: '限量玩偶' })).toContainText('已全數認領')
    await expect(loser.locator('li.g-card', { hasText: '限量玩偶' }).getByRole('button', { name: '我來送' })).toHaveCount(0)
    await winner.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(winner.locator('li.g-card', { hasText: '限量玩偶' })).toContainText('1 / 1')
  })

  await test.step('伺服器端數量沒有超賣（qty_claimed = qty_needed = 1）', async () => {
    const r = await request.get(`${process.env.E2E_API ?? 'http://localhost:8081'}/api/v1/public/wishlists/${list.slug}`)
    const item = (await r.json()).items[0]
    expect(item.qty_claimed).toBe(1)
    expect(item.is_fully_claimed).toBe(true)
  })
  await Promise.all([ca.close(), cb.close(), owner.api.dispose(), ...members.map(m => m.api.dispose())])
})

test('防超賣：頁面開著時別人搶走最後 1 件，頁面自動更新為「已全數認領」（SSE / 輪詢）', async ({ page }) => {
  const viewer = await signInPage(page)
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '最後一個', qty: 1 }] })
  await page.goto(`/s/${list.slug}`)
  const card = page.locator('li.g-card', { hasText: '最後一個' })
  await expect(page.getByText('即時更新中')).toBeVisible() // SSE 已連上
  await expect(card.getByRole('button', { name: '我來送' })).toBeVisible()
  await memberClaim(list.items[0].id, 1)
  await expect(card).toContainText('已全數認領')
  await Promise.all([owner.api.dispose(), viewer.api.dispose()])
})
