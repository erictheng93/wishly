import { readFileSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, makeStaff, registerUser, signInPage, sql, uid, watchProblems } from './helpers'
import { hydrated } from './support/hydrate'

// 靈感頁 / 品牌館：目錄資料 → 加到清單（預填 + catalog_ref）→「誰想要這個」依可見性列出 → 去送（深連結到品項）→ 後台上架
test.describe.configure({ mode: 'serial' })
test.beforeAll(() => { sql(readFileSync(new URL('../../../db/seed/catalog.sql', import.meta.url), 'utf8')) })

const card = (page: import('@playwright/test').Page, title: string) => page.locator('article.g-card', { hasText: title })

test('訪客瀏覽靈感頁、篩選、品牌館', async ({ page }) => {
  const problems = watchProblems(page)
  await page.goto('/inspire'); await hydrated(page)
  await expect(card(page, '香氛蠟燭禮盒')).toBeVisible() // 不數總數：平行執行的後台測試會新增商品
  await page.getByRole('button', { name: '彌月' }).click()
  await expect(card(page, '純棉嬰兒包巾')).toBeVisible()
  await expect(card(page, '鑄鐵琺瑯鍋')).toHaveCount(0)
  await page.getByRole('button', { name: 'NT$1,000 以下' }).click()
  await expect(page.locator('article.g-card')).toHaveCount(2) // 木質積木 890、彌月禮盒 980
  await page.goto('/brands'); await hydrated(page)
  await expect(page.getByText('HARIO').first()).toBeVisible()
  await page.goto('/brands/hario'); await hydrated(page)
  await expect(page.getByRole('heading', { name: 'HARIO' })).toBeVisible()
  await expect(card(page, '手沖咖啡濾杯組')).toBeVisible()
  expect(problems).toEqual([])
  const res = await page.goto('/brands/no-such-brand')
  expect(res?.status()).toBe(404)
  await expect(page.getByRole('heading', { name: '找不到這個品牌' })).toBeVisible()
})

test('加到清單 → 誰想要這個（公開 / 朋友）→ 去送並認捐', async ({ page, browser }) => {
  const owner = await registerUser({ tag: 'insp-owner' })
  const list = await createList(owner, { title: `靈感清單 ${uid()}`, items: [{ title: '佔位' }] })
  await loginBrowser(page.context(), owner)

  // 擁有者：靈感頁 → 選清單 → 編輯頁自動開啟預填的新增表單
  await page.goto('/inspire'); await hydrated(page)
  await card(page, '手沖咖啡濾杯組').getByRole('button', { name: '加到我的願望清單' }).click()
  await page.getByRole('dialog').getByRole('button', { name: list.title }).click()
  await expect(page).toHaveURL(new RegExp(`/lists/${list.id}/edit$`)) // query 已清掉
  const sheet = page.getByRole('dialog')
  await expect(sheet.getByLabel('名稱（必填）')).toHaveValue('手沖咖啡濾杯組')
  await expect(sheet.locator('input[type=url]')).toHaveValue(/momoshop/)
  await sheet.getByRole('button', { name: '儲存品項' }).click()
  await expect(page.getByText('手沖咖啡濾杯組')).toBeVisible()
  const itemId = sql(`SELECT id FROM wishlist_items WHERE wishlist_id = '${list.id}' AND catalog_ref = 'p02'`)
  expect(itemId).toMatch(/^[0-9a-f-]{36}$/)

  // 再手動新增品項：不可殘留預填
  await page.getByRole('button', { name: '新增品項' }).click()
  await expect(page.getByRole('dialog').getByLabel('名稱（必填）')).toHaveValue('')

  // 朋友限定清單（同一商品）：只有好友看得到
  const fw = await (await owner.api.post('wishlists', { data: { type: 'registry', title: `好友清單 ${uid()}`, visibility: 'friends' } })).json()
  await owner.api.post(`wishlists/${fw.id}/items`, { data: { title: '濾杯（好友）', qty_needed: 1, priority: 'medium', funding_mode: 'quantity', catalog_ref: 'p02' } })
  const cur = await (await owner.api.get(`wishlists/${fw.id}`)).json()
  await owner.api.patch(`wishlists/${fw.id}`, { data: { status: 'active', expected_updated_at: cur.wishlist.updated_at } })

  // 訪客：只看到公開那筆
  const guestCtx = await browser.newContext()
  const guest = await guestCtx.newPage()
  await guest.goto('/inspire'); await hydrated(guest)
  await card(guest, '手沖咖啡濾杯組').getByRole('button', { name: '誰想要這個' }).click()
  const gd = guest.getByRole('dialog')
  await expect(gd.getByText(list.title)).toBeVisible()
  await expect(gd.getByText(fw.title)).toHaveCount(0)
  await expect(gd.getByText('登入後可看到朋友與摯友的心願')).toBeVisible()
  await guestCtx.close()

  // 好友：朋友範圍看得到好友清單；去送 → 深連結到品項 → 認捐
  const donorCtx = await browser.newContext()
  const donor = await donorCtx.newPage()
  const d = await signInPage(donor, { tag: 'insp-donor' })
  const dId = sql(`SELECT id FROM users WHERE email = '${d.email}'`)
  const oId = sql(`SELECT id FROM users WHERE email = '${owner.email}'`)
  sql(`INSERT INTO friendships (user_a, user_b) VALUES (least('${dId}'::uuid, '${oId}'::uuid), greatest('${dId}'::uuid, '${oId}'::uuid))`)
  await donor.goto('/inspire'); await hydrated(donor)
  await card(donor, '手沖咖啡濾杯組').getByRole('button', { name: '誰想要這個' }).click()
  const dd = donor.getByRole('dialog')
  await dd.getByRole('button', { name: '朋友' }).click()
  await expect(dd.getByText(fw.title)).toBeVisible()
  await expect(dd.getByText(list.title)).toHaveCount(0)
  await dd.getByRole('button', { name: '公開' }).click()
  await dd.locator('li', { hasText: list.title }).getByRole('link', { name: '去送這個' }).click()
  await expect(donor).toHaveURL(new RegExp(`/s/${list.slug}\\?item=${itemId}`))
  const target = donor.locator(`#item-${itemId}`)
  await expect(target).toHaveClass(/g-hl/)
  await expect(target.getByRole('link', { name: '商品連結' })).toHaveAttribute('href', /momoshop/)
  await target.getByRole('button', { name: '我來送' }).click()
  await donor.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
  await expect(donor.getByRole('heading', { name: '✓ 這份交給你了！' })).toBeVisible()
  await donorCtx.close()
})

test('後台新增商品並上架後出現在靈感頁', async ({ page }) => {
  const staff = await registerUser({ tag: 'insp-staff' })
  makeStaff(staff.email)
  await loginBrowser(page.context(), staff)
  const ref = `e2e-${uid()}`, title = `E2E 好物 ${uid()}`
  await page.goto('/admin/catalog')
  await page.getByRole('button', { name: '商品', exact: true }).click()
  await page.getByRole('button', { name: '新增商品' }).click()
  const f = page.getByRole('dialog')
  await f.getByLabel('商品代號（ref，建立後不可改）').fill(ref)
  await f.getByLabel('商品名稱').fill(title)
  await f.getByLabel('價格（NT$）').fill('1500')
  await f.getByLabel('商品連結').fill('https://example.com/p')
  await f.getByLabel('上架中').check()
  await f.getByRole('button', { name: '儲存' }).click()
  await expect(page.getByText(title)).toBeVisible()
  // 公開 API 快取 60 秒（CDN）；e2e 直連 API 無快取
  await page.goto('/inspire'); await hydrated(page)
  await expect(card(page, title)).toBeVisible()
})
