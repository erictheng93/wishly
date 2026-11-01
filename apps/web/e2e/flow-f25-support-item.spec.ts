// 流程驗證 F25（支持品項）：網紅放入指向他人公開清單的支持品項 → 粉絲從品項點入並認捐（歸因）→ 成果數字；對方轉私人後顯示已結束。真實 API。
import { expect, test } from '@playwright/test'
import { claimViaUi, createList, loginBrowser, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

test('支持品項：前往支持 → 認捐計入成果、不洩漏粉絲暱稱；對方轉私人後已結束且不洩漏對方資訊', async ({ browser }) => {
  const target = await registerUser({ tag: 'sup-target', name: '受支持者' })
  const tList = await createList(target, { title: '對方的公開清單X', items: [{ title: '物資甲', qty: 5 }] })
  const star = await registerUser({ tag: 'sup-star', name: '網紅小星' })
  const sList = await createList(star, { title: '網紅的清單', items: [{ title: '普通品項', qty: 1 }] })
  const r = await star.api.post(`wishlists/${sList.id}/items`, { data: { title: '一起幫助他們', funding_mode: 'support', support_slug: tList.slug } })
  expect(r.ok(), await r.text()).toBe(true)

  const fan = await registerUser({ tag: 'sup-fan', name: '粉絲阿花' })
  const ctx = await browser.newContext()
  await loginBrowser(ctx, fan)
  const page = await ctx.newPage()

  await page.goto(`/s/${sList.slug}`); await hydrated(page)
  const card = page.locator('li.sp-card', { hasText: '一起幫助他們' })
  await expect(card).toContainText('對方的公開清單X')
  await expect(card.locator('.g-num').nth(1)).toHaveText('0')
  await card.getByRole('link', { name: '前往支持' }).click()
  await page.waitForURL(new RegExp(`/s/${tList.slug}\\?via=`)); await hydrated(page)
  await claimViaUi(page, '物資甲', 2)

  // 回網紅頁：測試環境無 CDN 快取，數字直接出現；頁面不得出現粉絲暱稱
  await page.goto(`/s/${sList.slug}`); await hydrated(page)
  await expect(page.locator('li.sp-card', { hasText: '一起幫助他們' }).locator('.g-num').nth(1)).toHaveText('2')
  await expect(page.locator('main')).not.toContainText('粉絲阿花') // 頁首會顯示登入者自己的名字，只查內容區

  // 對方改私人 → 已結束，只剩累計數字，不含對方標題
  const cur = await (await target.api.get(`wishlists/${tList.id}`)).json()
  const p = await target.api.patch(`wishlists/${tList.id}`, { data: { visibility: 'private', expected_updated_at: cur.wishlist.updated_at } })
  expect(p.ok(), await p.text()).toBe(true)
  await page.goto(`/s/${sList.slug}`); await hydrated(page)
  const ended = page.locator('li.sp-card', { hasText: '一起幫助他們' })
  await expect(ended).toContainText('已結束')
  await expect(ended.getByRole('link', { name: '前往支持' })).toHaveCount(0)
  await expect(ended.locator('.g-num').nth(1)).toHaveText('2')
  await expect(page.locator('body')).not.toContainText('對方的公開清單X')
  await ctx.close()
})
