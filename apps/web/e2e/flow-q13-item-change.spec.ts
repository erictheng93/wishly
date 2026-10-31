// 流程驗證 Q13：建立者修改已有捐助的品項 → 捐助者在「我的捐助」看到提醒並可按「知道了」，也立刻收到站內通知。
import { expect, test } from '@playwright/test'
import { createList, memberClaim, registerUser, signInPage } from './helpers'

test('建立者改數量：捐助者看到提醒、按知道了後消失，並收到站內通知', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '安撫奶嘴', qty: 3 }] })
  const item = list.items[0]!
  const donor = await signInPage(page)
  await memberClaim(item.id, 1, donor)

  const r = await owner.api.patch(`items/${item.id}`, { data: { qty_needed: 2 } })
  expect(r.status()).toBe(200)

  await page.goto('/me/claims')
  const card = page.locator('.g-item', { hasText: '安撫奶嘴' })
  const notice = card.locator('.js-item-changed')
  await expect(notice).toContainText('修改了這個品項，請再確認數量與規格')
  await notice.getByRole('button', { name: '知道了' }).click()
  await expect(notice).toHaveCount(0)
  await page.reload()
  await expect(page.locator('.g-item', { hasText: '安撫奶嘴' })).toBeVisible()
  await expect(page.locator('.g-item', { hasText: '安撫奶嘴' }).locator('.js-item-changed')).toHaveCount(0)

  // 站內立即出現；email 的 10 分鐘合併與寄送由後端 item_change_notice 測試涵蓋（e2e 不依賴安靜時段的時鐘）
  await page.goto('/me/notifications')
  const row = page.locator('.c-inbox li', { hasText: '內容有更新' })
  await expect(row).toContainText('安撫奶嘴')
  await expect(row).toContainText(list.title)
  // 再改一次：不用重新整理就多一則，連續修改不會被吃掉
  await expect(owner.api.patch(`items/${item.id}`, { data: { qty_needed: 3 } })).resolves.toBeOK()
  await expect(page.locator('.c-inbox li', { hasText: '內容有更新' })).toHaveCount(2)
})
