// 流程驗證 F24（D66 新手導覽）：新帳號 → /welcome 填個人資料 → 建立清單 → 加品項 → 發佈 → 捐助 → 完成；可跳過、可在設定重新開始。真實 API。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

const guide = (page: import('@playwright/test').Page) => page.getByRole('complementary', { name: '新手導覽' })

test('一步一步走完：受捐人 → 捐助人', async ({ page }) => {
  const friend = await registerUser({ tag: 'tourfriend' })
  const gift = await createList(friend, { title: '朋友的生日', items: [{ title: '藍牙耳機' }] })
  const u = await registerUser({ tag: 'tour', onboarding: true })
  await loginBrowser(page.context(), u)

  await test.step('第一次進儀表板被帶到導覽頁，先填個人資料', async () => {
    await page.goto('/dashboard')
    await expect(page).toHaveURL(/\/welcome$/)
    await hydrated(page)
    await expect(page.getByRole('heading', { name: '歡迎使用 Wishly' })).toBeVisible()
    await page.getByRole('button', { name: '儲存並建立清單' }).click()
    await expect(page.getByRole('alert')).toHaveText('請至少填寫帳號代號、自我介紹或生日其中一項')
    await page.getByLabel(/自我介紹/).fill('喜歡咖啡')
    await page.getByRole('button', { name: '儲存並建立清單' }).click()
  })

  await test.step('建立清單：導覽條說明怎麼填', async () => {
    await expect(page).toHaveURL(/\/lists\/new$/)
    await expect(guide(page)).toContainText('新手導覽 2 / 5')
    await expect(guide(page)).toContainText('建立第一份心願清單')
    await hydrated(page)
    await page.getByLabel(/清單名稱/).fill('我的生日願望')
    await page.getByRole('button', { name: '建立並新增品項' }).click()
    await expect(page).toHaveURL(/\/lists\/[^/]+\/edit$/)
  })

  await test.step('加品項 → 發佈，成為受捐人', async () => {
    await expect(guide(page)).toContainText('加入想要的品項')
    const id = page.url().match(/lists\/([0-9a-f-]+)\/edit/)![1]!
    expect((await u.api.post(`wishlists/${id}/items`, { data: { title: '手沖壺', qty_needed: 1 } })).ok()).toBe(true)
    await page.reload(); await hydrated(page)
    await expect(guide(page)).toContainText('發佈清單，成為受捐人')
    await page.getByRole('button', { name: '發佈並分享' }).click()
    await expect(guide(page)).toContainText('送出一份祝福，成為捐助人')
  })

  await test.step('到別人的清單認捐，成為捐助人', async () => {
    await page.goto(`/s/${gift.slug}`); await hydrated(page)
    await expect(guide(page)).toContainText('我來送')
    await page.getByRole('button', { name: '我來送' }).first().click()
    await page.getByRole('button', { name: '確定要送' }).click()
    await page.getByRole('button', { name: '回到清單繼續看' }).click() // 關掉成功頁，露出導覽條
    await expect(guide(page)).toContainText('導覽完成！')
  })

  await test.step('按「完成導覽」收起，不再出現', async () => {
    await guide(page).getByRole('button', { name: '完成導覽' }).click()
    await expect(guide(page)).toHaveCount(0)
    await page.goto('/dashboard'); await hydrated(page)
    await expect(page).toHaveURL(/\/dashboard$/)
    await expect(guide(page)).toHaveCount(0)
  })
})

test('懂的人可以跳過，之後在設定重新開始', async ({ page }) => {
  const u = await registerUser({ tag: 'tourskip', onboarding: true })
  await loginBrowser(page.context(), u)
  await page.goto('/welcome'); await hydrated(page)
  await page.getByRole('button', { name: '跳過導覽，直接開始' }).click()
  await expect(page).toHaveURL(/\/dashboard$/)
  await expect(guide(page)).toHaveCount(0)

  await page.goto('/settings'); await hydrated(page)
  await page.getByRole('button', { name: '重新開始導覽' }).click()
  await expect(page).toHaveURL(/\/welcome$/)
  await expect(page.getByText('完善個人資料')).toBeVisible()
  // 導覽條在其他頁也能跳過
  await page.goto('/lists/new'); await hydrated(page)
  await guide(page).getByRole('button', { name: '跳過導覽' }).click()
  await expect(guide(page)).toHaveCount(0)
})
