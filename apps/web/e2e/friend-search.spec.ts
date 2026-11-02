// 加好友改為邊打邊搜（2026-11-02 推翻防列舉）：依名字找到人、按「加好友」後對方確實收到申請。
import { expect, test } from '@playwright/test'
import { loginBrowser, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

test('依名字搜尋使用者並送出好友邀請，對方收到申請', async ({ browser }) => {
  const tag = `搜尋${Date.now() % 1e6}`
  const me = await registerUser({ tag: 'searcher' })
  const target = await registerUser({ tag: 'target', name: `${tag} 小明` })

  const ctx = await browser.newContext()
  await loginBrowser(ctx, me)
  const page = await ctx.newPage()
  await page.goto('/friends'); await hydrated(page)

  const box = page.getByRole('searchbox', { name: '搜尋使用者' })
  await box.fill(`${tag} 小`)
  const add = page.getByRole('button', { name: `加好友 ${target.name}` })
  await add.click()
  await expect(page.getByRole('button', { name: '已送出' })).toBeDisabled()

  const f = await (await target.api.get('friends')).json()
  expect(f.incoming.map((r: any) => r.user.display_name)).toEqual([me.name])

  await box.fill('完全不存在的名字zzz')
  await expect(page.getByText('找不到符合的使用者')).toBeVisible()
})
