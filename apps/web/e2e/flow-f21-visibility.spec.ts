// 流程驗證 F21 / F3（docs/02 清單可見性與存取、訪客開啟分享連結）：真實 API，補 social.spec 未涵蓋的情境。
import { expect, test, type Browser } from '@playwright/test'
import { createList, loginBrowser, registerUser, type User } from './helpers'
import { hydrated } from './support/hydrate'

const ok = async (r: import('@playwright/test').APIResponse, what: string) => { if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${await r.text()}`); return r }
const befriend = async (a: User, b: User) => {
  const inv = await (await ok(await a.api.post('friends/invites'), '產生邀請')).json()
  await ok(await b.api.post(`friends/invites/${inv.token}/accept`), '接受邀請')
}
const setVis = (u: User, id: string, visibility: string) => u.api.patch(`wishlists/${id}`, { data: { visibility } }).then(r => ok(r, `可見度 ${visibility}`))
async function pageFor(browser: Browser, u?: User) {
  const ctx = await browser.newContext()
  if (u) await loginBrowser(ctx, u)
  return ctx.newPage()
}

test('私人清單：好友開連結與不存在的清單同樣是 404；擁有者本人看得到', async ({ browser }) => {
  const owner = await registerUser({ tag: 'owner' }), friend = await registerUser({ tag: 'friend' })
  await befriend(owner, friend)
  const list = await createList(owner, { title: `私人 ${Date.now() % 1e5}` })
  await setVis(owner, list.id, 'private')

  const pf = await pageFor(browser, friend)
  const res = await pf.goto(`/s/${list.slug}`)
  expect(res?.status()).toBe(404)
  await hydrated(pf)
  await expect(pf.getByRole('heading', { name: '找不到這份清單' })).toBeVisible()
  await expect(pf.getByText(list.title)).toHaveCount(0)

  const po = await pageFor(browser, owner)
  await po.goto(`/s/${list.slug}`); await hydrated(po)
  await expect(po.getByRole('heading', { name: list.title })).toBeVisible()
})

test('收緊可見度：公開 → 好友，探索頁移除、訪客看到需登入（並顯示建立者）', async ({ browser }) => {
  const owner = await registerUser({ tag: 'owner', name: `建立者${Date.now() % 1e4}` })
  const list = await createList(owner, { title: `收緊 ${Date.now() % 1e5}` })
  const anon = await pageFor(browser)
  await anon.goto(`/s/${list.slug}`); await hydrated(anon)
  await expect(anon.getByRole('heading', { name: list.title })).toBeVisible()

  await setVis(owner, list.id, 'friends')
  // 探索頁的 API 回應會被 CDN 快取約 1 分鐘（本機無 CDN），清單頁則立即依 check_wishlist 擋下
  await anon.goto(`/s/${list.slug}`); await hydrated(anon)
  await expect(anon.getByRole('heading', { name: '請先登入' })).toBeVisible()
  await expect(anon.getByText(`擁有者：${owner.name}`)).toBeVisible()
  await expect(anon.getByText(list.title)).toHaveCount(0)
  await anon.goto('/explore'); await hydrated(anon)
  await expect(anon.getByText(list.title)).toHaveCount(0)
})

test('解除好友後立即失去僅好友清單的存取', async ({ browser }) => {
  const owner = await registerUser({ tag: 'owner' }), friend = await registerUser({ tag: 'friend' })
  await befriend(owner, friend)
  const list = await createList(owner, { title: `好友限定 ${Date.now() % 1e5}` })
  await setVis(owner, list.id, 'friends')

  const pf = await pageFor(browser, friend)
  await pf.goto(`/s/${list.slug}`); await hydrated(pf)
  await expect(pf.getByRole('heading', { name: list.title })).toBeVisible()

  const me = await (await friend.api.get('me')).json()
  await ok(await owner.api.delete(`friends/${me.user?.id ?? me.id}`), '解除好友')
  await pf.reload(); await hydrated(pf)
  await expect(pf.getByRole('heading', { name: '你沒有權限查看這份清單' })).toBeVisible()
  await expect(pf.getByText(list.title)).toHaveCount(0)
})
