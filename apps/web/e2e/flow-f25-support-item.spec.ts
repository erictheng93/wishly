// 流程驗證 F25（支持品項）：網紅放入指向他人公開清單的支持品項 → 粉絲從品項點入並認捐（歸因）→ 成果數字；對方轉私人後顯示已結束。真實 API。
import { expect, test } from '@playwright/test'
import { claimViaUi, createList, loginBrowser, makeStaff, PASSWORD, registerUser } from './helpers'
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

/** 網紅清單放一個指向 targetSlug 的支持品項，回傳網紅分享頁 slug */
async function starList(targetSlug: string) {
  const star = await registerUser({ tag: 'sup-star2', name: '網紅小星' })
  const sList = await createList(star, { title: '網紅的清單', items: [{ title: '普通品項', qty: 1 }] })
  const r = await star.api.post(`wishlists/${sList.id}/items`, { data: { title: '一起幫助他們', funding_mode: 'support', support_slug: targetSlug } })
  expect(r.ok(), await r.text()).toBe(true)
  return sList.slug
}

test('歸因跨登入：未登入從網紅頁前往支持 → 對方清單導去登入 → 登入回來認捐 → 網紅頁數字增加', async ({ browser }) => {
  const target = await registerUser({ tag: 'sup-target2' })
  const tList = await createList(target, { title: '對方清單Y', items: [{ title: '物資乙', qty: 3 }] })
  const star = await starList(tList.slug)
  const fan = await registerUser({ tag: 'sup-fan2', name: '粉絲小美' })
  const page = await (await browser.newContext()).newPage() // 未登入

  await page.goto(`/s/${star}`); await hydrated(page)
  await page.locator('li.sp-card', { hasText: '一起幫助他們' }).getByRole('link', { name: '前往支持' }).click()
  await page.waitForURL(new RegExp(`/s/${tList.slug}\\?via=`)); await hydrated(page)
  await page.locator('li.g-card', { hasText: '物資乙' }).getByRole('link', { name: '登入後來送' }).click()
  await expect(page).toHaveURL(/\/login\?redirect=/)
  await page.getByLabel('Email').fill(fan.email)
  await page.getByLabel('密碼').fill(PASSWORD)
  await page.getByRole('button', { name: '登入', exact: true }).click()
  await page.waitForURL(new RegExp(`/s/${tList.slug}`)); await hydrated(page)
  await claimViaUi(page, '物資乙', 1)

  await page.goto(`/s/${star}`); await hydrated(page)
  await expect(page.locator('li.sp-card', { hasText: '一起幫助他們' }).locator('.g-num').nth(1)).toHaveText('1')
})

test('歸因：贊助（crowdfund）經支持品項點入，數字出現在「點數」', async ({ browser }) => {
  const target = await registerUser({ tag: 'sup-target3' })
  const tList = await createList(target, { title: '對方眾籌清單Z', items: [{ title: '物資丙', qty: 1 }] })
  expect((await target.api.put(`wishlists/${tList.id}/shipping-address`, { data: { recipient_name: '王小米', phone: '0912345678', address: '台北市大安區和平東路二段 1 號 3 樓' } })).status()).toBe(200)
  const cf = await target.api.post(`wishlists/${tList.id}/items`, { data: { title: '嬰兒推車', funding_mode: 'crowdfund', target_points: 1000, funding_deadline: new Date(Date.now() + 7 * 864e5).toISOString(), priority: 'medium' } })
  expect(cf.status(), await cf.text()).toBe(201)
  const star = await starList(tList.slug)
  const fan = await registerUser({ tag: 'sup-fan3', name: '粉絲阿本' })
  const staff = await registerUser({ tag: 'sup-staff' })
  makeStaff(staff.email)
  const me = await (await fan.api.get('me')).json()
  const g = await staff.api.post('admin/wallets/grants', { data: { user_id: me.id, points: 500, reason: 'E2E 發點' }, headers: { 'Idempotency-Key': crypto.randomUUID() } })
  expect(g.status(), await g.text()).toBe(201)

  const ctx = await browser.newContext(); await loginBrowser(ctx, fan)
  const page = await ctx.newPage()
  await page.goto(`/s/${star}`); await hydrated(page)
  await page.locator('li.sp-card', { hasText: '一起幫助他們' }).getByRole('link', { name: '前往支持' }).click()
  await page.waitForURL(new RegExp(`/s/${tList.slug}\\?via=`)); await hydrated(page)
  await page.locator('li.g-card', { hasText: '嬰兒推車' }).getByRole('button', { name: '心願眾籌' }).click()
  const fund = page.getByRole('dialog')
  await fund.getByLabel('贊助點數').fill('300')
  await fund.getByRole('button', { name: /確認贊助/ }).click()
  await expect(page.getByRole('heading', { name: '✓ 感謝你的贊助！' })).toBeVisible()

  await page.goto(`/s/${star}`); await hydrated(page)
  await expect(page.locator('li.sp-card', { hasText: '一起幫助他們' }).locator('.g-num').nth(0)).toHaveText('300')
})
