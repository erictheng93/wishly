// 分享面板「附上好友邀請」：擁有者產生清單專屬邀請連結，新帳號開啟並接受後直接進到該清單。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, registerUser } from './helpers'
import { hydrated } from './support/hydrate'

const ok = async (r: import('@playwright/test').APIResponse, what: string) => { if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${await r.text()}`); return r }

test('好友限定清單附上邀請連結，對方接受後進入清單', async ({ browser }) => {
  const owner = await registerUser({ tag: 'inv-owner' })
  const guest = await registerUser({ tag: 'inv-guest' })
  const list = await createList(owner, { title: `邀請清單 ${Date.now() % 1e5}` })
  const cur = await (await ok(await owner.api.get(`wishlists/${list.id}`), '讀取清單')).json()
  await ok(await owner.api.patch(`wishlists/${list.id}`, { data: { visibility: 'friends', expected_updated_at: cur.wishlist.updated_at } }), '可見度 friends')

  const octx = await browser.newContext()
  await loginBrowser(octx, owner)
  const op = await octx.newPage()
  await op.goto(`/lists/${list.id}/edit`); await hydrated(op)
  await op.getByRole('button', { name: '分享' }).first().click()
  await op.getByLabel('附上好友邀請').check()
  const input = op.locator('#c-share-input')
  await expect(input).toHaveValue(/\/invite\//)
  const url = await input.inputValue()

  const gctx = await browser.newContext()
  await loginBrowser(gctx, guest)
  const gp = await gctx.newPage()
  await gp.goto(new URL(url).pathname); await hydrated(gp)
  await expect(gp.getByText(list.title)).toBeVisible()
  await gp.getByRole('button', { name: '成為好友' }).click()
  await expect(gp).toHaveURL(new RegExp(`/s/${list.slug}`))
  await expect(gp.getByRole('heading', { name: list.title })).toBeVisible()
})
