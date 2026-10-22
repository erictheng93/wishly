// 流程驗證 Q9：好友限定清單的門檻頁可直接用擁有者 ID 送好友邀請（擁有者沒設 handle、沒有個人頁也行）。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

const ok = async (r: import('@playwright/test').APIResponse, what: string) => { if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${await r.text()}`); return r }

test('非好友在門檻頁送出好友邀請，擁有者接受後即可看到清單', async ({ browser }) => {
  const owner = await registerUser({ tag: 'owner', name: `無代號${Date.now() % 1e4}` }) // 註冊後 handle 預設為空
  const stranger = await registerUser({ tag: 'stranger' })
  const list = await createList(owner, { title: `好友限定 ${Date.now() % 1e5}` })
  await ok(await owner.api.patch(`wishlists/${list.id}`, { data: { visibility: 'friends' } }), '可見度 friends')

  const ctx = await browser.newContext()
  await loginBrowser(ctx, stranger)
  const page = await ctx.newPage()
  await page.goto(`/s/${list.slug}`); await hydrated(page)
  await expect(page.getByRole('heading', { name: '你沒有權限查看這份清單' })).toBeVisible()
  await expect(page.getByRole('link', { name: /前往 .* 的頁面加好友/ })).toHaveCount(0) // 沒有 handle 就沒有個人頁連結

  const send = page.getByRole('button', { name: `送出好友邀請給 ${owner.name}` })
  await send.click()
  await expect(page.getByText('已送出好友邀請，對方接受後就能看到這份清單')).toBeVisible()
  await expect(send).toBeDisabled()

  const f = await (await ok(await owner.api.get('friends'), '讀取好友')).json()
  expect(f.incoming).toHaveLength(1)
  await ok(await owner.api.post(`friends/requests/${f.incoming[0].id}/accept`), '接受邀請')

  await page.reload(); await hydrated(page)
  await expect(page.getByRole('heading', { name: list.title })).toBeVisible()
})
