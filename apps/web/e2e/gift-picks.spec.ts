import { readFileSync } from 'node:fs'
import { expect, test } from '@playwright/test'
import { registerUser, loginBrowser, makeStaff, sql } from './helpers'

test.beforeAll(() => { sql(readFileSync(new URL('../../../db/seed/catalog.sql', import.meta.url), 'utf8')) })

// 需要完整後端：staff 給商品標興趣 → 個人頁推薦區塊 → 情境頁「幫哪位朋友挑」→ 興趣設私人後走 fallback
test('興趣推薦：個人頁與情境頁', async ({ browser }) => {
  const staff = await registerUser({ tag: 'gp-staff' })
  makeStaff(staff.email)
  for (const ref of ['p05', 'p10']) {
    const id = sql(`SELECT id FROM catalog_products WHERE ref = '${ref}'`)
    const r = await staff.api.patch(`admin/catalog/products/${id}`, { data: { interests: ['coffee'] } })
    expect(r.ok(), await r.text()).toBeTruthy()
  }
  const a = await registerUser({ tag: 'gpa' }), b = await registerUser({ tag: 'gpb' })
  const handle = `gp${Date.now().toString(36)}`.slice(0, 20)
  const patch = async (data: object) => expect((await a.api.patch('me', { data })).ok()).toBeTruthy()
  await patch({ handle, interests: ['coffee'], interests_vis: 'public', gift_dislikes: ['筆記本'], gift_prefs_vis: 'public' })
  const meA = await (await a.api.get('me')).json()
  const inv = await (await a.api.post('friends/invites')).json()
  expect((await b.api.post(`friends/invites/${inv.token}/accept`)).ok()).toBeTruthy()

  const anon = await browser.newContext()
  const anonPage = await anon.newPage()
  await anonPage.goto(`/u/${handle}`)
  await expect(anonPage.getByRole('heading', { name: `適合送給 ${a.name} 的禮物` })).toBeVisible()
  await anon.close()

  const ctx = await browser.newContext()
  await loginBrowser(ctx, b)
  const page = await ctx.newPage()
  await page.goto(`/u/${handle}`)
  const picks = page.locator('[data-gift-picks]')
  await expect(picks.getByRole('heading', { name: `適合送給 ${a.name} 的禮物` })).toBeVisible()
  await expect(picks.locator('[data-matched]').first()).toContainText('咖啡')

  await page.goto(`/inspire/birthday?for=${meA.id}`)
  const first = page.locator('article.x-card').first()
  await expect(first.locator('[data-matched]')).toContainText(`符合 ${a.name} 的興趣：咖啡`)

  await expect(page.locator('article.x-card').filter({ hasText: '筆記本與鋼筆禮盒' })).toHaveCount(0)

  await patch({ interests_vis: 'private' })
  await page.goto(`/inspire/birthday?for=${meA.id}`)
  await expect(page.getByRole('status').filter({ hasText: `沒有找到符合 ${a.name} 興趣的商品` })).toBeVisible()
  await expect(page.locator('article.x-card').filter({ hasText: '筆記本與鋼筆禮盒' })).toHaveCount(0)
  await page.goto(`/u/${handle}`)
  await expect(page.locator('[data-gift-picks]').getByRole('heading', { name: `也許 ${a.name} 會喜歡` })).toBeVisible()
  await ctx.close()
})
