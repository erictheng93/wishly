import { expect, test } from '@playwright/test'
import { registerUser, loginBrowser } from './helpers'

// 需要完整後端（Task 3–6）：連結頁匿名可見；偏好設為 friends 時，匿名看不到、好友看得到
test('個人頁：連結頁匿名可見、偏好僅好友可見', async ({ page, browser }) => {
  const a = await registerUser({ tag: 'rpa' }), b = await registerUser({ tag: 'rpb' })
  const handle = `rp${Date.now().toString(36)}`.slice(0, 20)
  const ok = async (r: any) => { expect(r.ok(), await r.text()).toBeTruthy() }
  await ok(await a.api.patch('me', { data: { handle, gift_likes: ['手沖咖啡豆'], gift_likes_vis: 'friends' } }))
  await ok(await a.api.put('me/links', { data: { links: [{ title: '我的 IG', url: 'https://instagram.com/rp_test', hidden: false }] } }))

  await page.goto(`/l/${handle}`)
  await expect(page.getByRole('link', { name: '我的 IG' })).toHaveAttribute('rel', 'nofollow ugc noopener noreferrer')
  await expect(page.getByRole('link', { name: '看我的願望清單' })).toBeVisible()

  await page.goto(`/u/${handle}`)
  await expect(page.getByRole('heading', { name: a.name })).toBeVisible()
  await expect(page.getByText('手沖咖啡豆')).toHaveCount(0)

  const inv = await (await a.api.post('friends/invites')).json()
  await ok(await b.api.post(`friends/invites/${inv.token}/accept`))
  const ctx = await browser.newContext()
  await loginBrowser(ctx, b)
  const p2 = await ctx.newPage()
  await p2.goto(`/u/${handle}`)
  await expect(p2.getByText('手沖咖啡豆')).toBeVisible()
})
