// 流程驗證 F2 / F8（docs/02 建立清單與品項、清單關閉 / 封存）：真實 API。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser } from './helpers'
import { API } from './support/env'

test('品項排序重整後保留；有捐助的品項刪除需二次確認並取消捐助', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '甲', qty: 2 }, { title: '乙' }, { title: '丙' }] })
  await memberClaim(list.items[0]!.id, 1)
  await loginBrowser(page.context(), owner)

  await page.goto(`/lists/${list.id}/edit`)
  const names = page.locator('article strong')
  await expect(names).toHaveText(['甲', '乙', '丙'])
  // 連按兩次：前端須依序送出，後端最後留下的是畫面上的順序
  let saved = 0
  page.on('response', r => { if (r.url().includes('/items/reorder')) saved++ })
  await page.locator('article', { hasText: '丙' }).getByRole('button', { name: '上移' }).click()
  await page.locator('article', { hasText: '丙' }).getByRole('button', { name: '上移' }).click()
  await expect(names).toHaveText(['丙', '甲', '乙'])
  await expect.poll(() => saved).toBe(2)
  await page.reload()
  await expect(names).toHaveText(['丙', '甲', '乙'])

  await page.locator('article', { hasText: '甲' }).getByRole('button', { name: '刪除' }).click()
  const sheet = page.getByRole('dialog', { name: '刪除品項' })
  await sheet.getByRole('button', { name: '刪除', exact: true }).click()
  await expect(sheet.getByRole('alert')).toContainText('已有人認領')
  await sheet.getByRole('button', { name: '連同認領一併刪除' }).click()
  await expect(names).toHaveText(['丙', '乙'])
  const pub = await (await page.request.get(`${API}/api/v1/public/wishlists/${list.slug}`)).json()
  expect(pub.items.map((i: any) => i.title)).toEqual(['丙', '乙'])
})

test('結束 → 重新開放 → 封存 → 取消封存；結束與封存時編輯頁只留可用的操作', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { title: `F8 生命週期 ${Date.now() % 1e5}` })
  await loginBrowser(page.context(), owner)
  const share = page.getByRole('button', { name: '分享', exact: true })
  const settings = page.getByText('清單設定')

  await page.goto(`/lists/${list.id}/edit`)
  await settings.click()
  await page.getByRole('button', { name: '結束清單' }).click()
  await expect(page.getByText('清單已結束，品項無法修改。')).toBeVisible()
  await expect(page.getByRole('button', { name: '＋ 新增品項' })).toHaveCount(0)
  await expect(page.getByRole('button', { name: '上移' })).toHaveCount(0)
  await expect(share).toBeVisible() // 結束的清單公開頁仍可看（唯讀）
  await page.getByRole('button', { name: '重新開放' }).click()
  await expect(page.getByRole('button', { name: '＋ 新增品項' })).toBeVisible()

  await page.goto('/dashboard')
  await page.locator('.c-card', { hasText: list.title }).getByRole('button', { name: '封存' }).click()
  await page.getByRole('dialog', { name: '封存清單' }).getByRole('button', { name: '確認封存' }).click()
  await expect(page.locator('.c-card', { hasText: list.title })).toHaveCount(0)
  expect((await page.request.get(`${API}/api/v1/public/wishlists/${list.slug}`)).status()).toBe(404)

  // 封存後後端只接受取消封存：不顯示設定表單與分享（分享出去也是 404）
  await page.goto(`/lists/${list.id}/edit`)
  await expect(page.getByRole('button', { name: '取消封存' })).toBeVisible()
  await expect(settings).toHaveCount(0)
  await expect(share).toHaveCount(0)
  await page.getByRole('button', { name: '取消封存' }).click()
  await expect(page.getByText('清單已結束，品項無法修改。')).toBeVisible()
  await expect(share).toBeVisible()
})
