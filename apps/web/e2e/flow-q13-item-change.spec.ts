// 流程驗證 Q13：建立者修改已有捐助的品項 → 捐助者在「我的捐助」看到提醒並可按「知道了」，也收到（合併延後的）通知信。
import { expect, test } from '@playwright/test'
import { createList, memberClaim, registerUser, signInPage, sql, waitForMail } from './helpers'

test('建立者改數量：捐助者看到提醒、按知道了後消失，並收到通知信', async ({ page }) => {
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

  // 通知信延後 10 分鐘合併寄出；測試直接把排程提前，交給背景 worker 寄
  sql(`UPDATE notifications SET scheduled_at = now() WHERE kind = 'claim.item_changed' AND status = 'pending' AND payload->>'item_id' = '${item.id}'`)
  const mail = await waitForMail(donor.email, { subject: /內容有更新/ })
  expect(mail.subject).toContain('安撫奶嘴')
  expect(mail.text).toContain(list.title)
  expect(mail.text).toContain('/me/claims')
})
