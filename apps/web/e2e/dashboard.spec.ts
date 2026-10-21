import { expect, test } from '@playwright/test'
import { createList, loginBrowser, registerUser } from './helpers'

test('儀表板：整張卡片點進編輯頁，卡內按鈕各自運作，封存後卡片消失', async ({ page }) => {
  const u = await registerUser({ tag: 'dash' })
  const list = await createList(u, { items: [{ title: '奶瓶' }] })
  await loginBrowser(page.context(), u)
  const card = page.locator('article.c-card', { hasText: list.title })

  await test.step('點卡片空白處 → 編輯頁', async () => {
    await page.goto('/dashboard')
    await expect(card.getByRole('link', { name: '編輯' })).toHaveCount(0) // 編輯按鈕已移除
    await card.click({ position: { x: 8, y: 8 } }) // 左上角內距，不在標題上
    await expect(page).toHaveURL(new RegExp(`/lists/${list.id}/edit$`))
  })

  await test.step('點「進度」→ 進度頁，不會被卡片連結攔走', async () => {
    await page.goto('/dashboard')
    await card.getByRole('link', { name: '進度' }).click()
    await expect(page).toHaveURL(new RegExp(`/lists/${list.id}/progress$`))
  })

  await test.step('點「封存」→ 確認窗（不跳頁）→ 確認後卡片從儀表板消失', async () => {
    await page.goto('/dashboard')
    await card.getByRole('button', { name: '封存' }).click()
    await expect(page).toHaveURL(/\/dashboard$/)
    await expect(page.getByText(/尚未達標的認捐點數會全數退回捐贈者錢包，之後取消封存也不會恢復/)).toBeVisible()
    await page.getByRole('button', { name: '確認封存' }).click()
    await expect(card).toHaveCount(0)
    // 直接開網址仍可進編輯頁，但只能看
    await page.goto(`/lists/${list.id}/edit`)
    await expect(page.getByText('清單已封存，品項無法修改。').first()).toBeVisible()
  })

  await test.step('已封存列表 → 取消封存 → 回到我的清單，狀態為已結束', async () => {
    await page.goto('/dashboard')
    await page.getByRole('button', { name: '已封存的清單' }).click()
    await card.getByRole('button', { name: '取消封存' }).click()
    await expect(card).toHaveCount(0)
    await expect(page.getByText('沒有已封存的清單')).toBeVisible()
    await page.getByRole('button', { name: '← 返回我的清單' }).click()
    await expect(card.getByText('已結束')).toBeVisible()
    await expect(card.getByRole('button', { name: '封存' })).toBeVisible()
  })

  await test.step('編輯頁也能取消封存', async () => {
    await card.getByRole('button', { name: '封存' }).click()
    await page.getByRole('button', { name: '確認封存' }).click()
    await expect(card).toHaveCount(0)
    await page.goto(`/lists/${list.id}/edit`)
    await page.getByRole('button', { name: '取消封存' }).click()
    await expect(page.getByText('清單已結束，品項無法修改。')).toBeVisible()
  })
})

test('儀表板：已封存的清單不佔一般列表的分頁', async ({ page }) => {
  const u = await registerUser({ tag: 'dashpage' })
  const keep = await createList(u) // 最早建立，id 最小；被 20 份較新的封存清單排在後面
  for (let i = 0; i < 20; i++) {
    const w = await (await u.api.post('wishlists', { data: { type: 'personal', title: `封存 ${i}` } })).json()
    expect((await u.api.delete(`wishlists/${w.id}`)).status()).toBe(204)
  }
  await loginBrowser(page.context(), u)
  await page.goto('/dashboard')
  await expect(page.getByRole('link', { name: keep.title })).toBeVisible()
  await expect(page.locator('article.c-card')).toHaveCount(1)
  await expect(page.getByRole('button', { name: '載入更多' })).toHaveCount(0)
  await page.getByRole('button', { name: '已封存的清單' }).click()
  await expect(page.locator('article.c-card')).toHaveCount(20)
  await expect(page.getByRole('link', { name: keep.title })).toHaveCount(0)
})
