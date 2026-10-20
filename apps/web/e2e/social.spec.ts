import { expect, test, type Browser, type Page } from '@playwright/test'
import { createList, loginBrowser, PASSWORD, registerUser, uid, type List, type User } from './helpers'
import { hydrated } from './support/hydrate'

// ---------- API 前置（UI 只做被測行為）----------
const ok = async (r: import('@playwright/test').APIResponse, what: string) => { if (!r.ok()) throw new Error(`${what} 失敗：${r.status()} ${await r.text()}`); return r }
const withHandle = async (tag: string) => {
  const u = await registerUser({ tag }) as User & { handle: string }
  u.handle = `e${uid()}`
  await ok(await u.api.patch('me', { data: { handle: u.handle } }), '設定 handle')
  return u
}
const befriend = async (a: User, b: User) => {
  const inv = await (await ok(await a.api.post('friends/invites'), '產生邀請')).json()
  await ok(await b.api.post(`friends/invites/${inv.token}/accept`), '接受邀請')
}
/** 建立清單並改成指定可見度（發佈後 PATCH） */
async function listWith(owner: User, visibility: string, extra: Record<string, unknown> = {}, items = [{ title: '奶瓶', qty: 2 }]): Promise<List> {
  const l = await createList(owner, { items })
  const cur = await (await owner.api.get(`wishlists/${l.id}`)).json()
  await ok(await owner.api.patch(`wishlists/${l.id}`, { data: { visibility, expected_updated_at: cur.wishlist.updated_at, ...extra } }), `改可見度 ${visibility}`)
  return l
}
const claimApi = async (u: User, itemId: string, visibility: string) =>
  ok(await u.api.post(`items/${itemId}/claims`, { data: { qty: 1, visibility }, headers: { 'Idempotency-Key': crypto.randomUUID() } }), '捐助')
async function pageFor(browser: Browser, u?: User): Promise<Page> {
  const ctx = await browser.newContext()
  if (u) await loginBrowser(ctx, u)
  return ctx.newPage()
}
const closeAll = (users: User[]) => Promise.all(users.map(u => u.api.dispose()))

test('好友：邀請連結 + QR → 未登入開連結帶 redirect → 登入後接受 → 互為好友；以 handle 送申請 → 對方接受', async ({ browser }) => {
  const a = await withHandle('fa'), b = await withHandle('fb'), c = await withHandle('fc')
  const pa = await pageFor(browser, a), pb = await pageFor(browser)

  await test.step('A 產生邀請連結，看得到 QR', async () => {
    await pa.goto('/friends'); await hydrated(pa)
    await expect(pa.getByText('目前還沒有好友。')).toBeVisible() // 沒好友也顯示動態框架與引導
    await expect(pa.getByText('30 天內沒有好友生日。')).toBeVisible()
    await pa.getByRole('button', { name: '產生邀請連結' }).click()
    await expect(pa.locator('main svg').first()).toBeVisible()
  })
  const url = await pa.getByLabel('邀請網址').inputValue()
  expect(url).toMatch(/\/invite\/[^/]+$/)

  await test.step('B 未登入開連結：顯示邀請者、登入連結帶 redirect；登入後接受', async () => {
    await pb.goto(url)
    await expect(pb.getByRole('heading', { name: `${a.name} 邀請你成為好友` })).toBeVisible()
    const login = pb.getByRole('link', { name: '登入後成為好友' })
    await expect(login).toHaveAttribute('href', /\/login\?redirect=(%2F|\/)invite(%2F|\/)/)
    await login.click(); await hydrated(pb)
    await pb.getByLabel('Email').fill(b.email); await pb.getByLabel('密碼').fill(PASSWORD)
    await pb.getByRole('button', { name: '登入', exact: true }).click()
    await expect(pb).toHaveURL(/\/invite\//)
    await pb.getByRole('button', { name: '成為好友' }).click()
    await expect(pb).toHaveURL(/\/friends$/)
    await expect(pb.getByText(a.name, { exact: true })).toBeVisible()
  })

  await test.step('A 重新整理後也看到 B', async () => {
    await pa.reload(); await expect(pa.getByText(b.name, { exact: true })).toBeVisible()
    await expect(pa.getByRole('heading', { name: '我的好友（1）' })).toBeVisible()
  })

  await test.step('C 在 /u/{A} 送出好友邀請 → A 在 /friends 接受', async () => {
    const pc = await pageFor(browser, c)
    await pc.goto(`/u/${a.handle}`); await hydrated(pc)
    await pc.getByRole('button', { name: '送出好友邀請' }).click()
    await expect(pc.getByText('已送出邀請')).toBeVisible()
    await pa.goto('/friends')
    await pa.locator('.c-row', { hasText: c.name }).getByRole('button', { name: '接受' }).click()
    await expect(pa.getByRole('heading', { name: '我的好友（2）' })).toBeVisible()
    await pc.reload(); await expect(pc.locator('.c-badge', { hasText: '好友' })).toBeVisible()
  })
  await closeAll([a, b, c])
})

test('僅好友清單：匿名請先登入、非好友被擋並可前往個人頁、好友可看並捐助（可見度預設帶帳號設定）', async ({ browser }) => {
  const owner = await withHandle('fo'), friend = await registerUser({ tag: 'ff' }), stranger = await registerUser({ tag: 'fs' })
  await befriend(owner, friend)
  await ok(await friend.api.patch('me', { data: { default_claim_visibility: 'friends' } }), '帳號預設可見度')
  const list = await listWith(owner, 'friends')

  const anon = await pageFor(browser)
  await anon.goto(`/s/${list.slug}`)
  await expect(anon.getByRole('heading', { name: '請先登入' })).toBeVisible()
  await expect(anon.getByRole('link', { name: '登入後查看' })).toHaveAttribute('href', /login\?redirect=(%2F|\/)s(%2F|\/)/)

  const ps = await pageFor(browser, stranger)
  await ps.goto(`/s/${list.slug}`)
  await expect(ps.getByRole('heading', { name: '你沒有權限查看這份清單' })).toBeVisible()
  await expect(ps.getByRole('link', { name: new RegExp(`前往 ${owner.name} 的頁面加好友`) })).toHaveAttribute('href', `/u/${owner.handle}`)

  const pf = await pageFor(browser, friend)
  // 不等 /me：頁面開啟 sheet 前會自己等登入身分（回歸測試：手快點「我要送」要等到身分確認，不能誤判成未登入）
  await pf.goto(`/s/${list.slug}`); await hydrated(pf)
  await expect(pf.getByRole('heading', { level: 1, name: list.title })).toBeVisible()
  await pf.locator('li.g-card', { hasText: '奶瓶' }).getByRole('button', { name: '我要送' }).click()
  const sheet = pf.getByRole('dialog')
  await expect(sheet.getByLabel('這筆捐助誰看得到')).toHaveValue('friends') // 帳號預設
  await sheet.getByRole('button', { name: '確認捐助' }).click()
  await expect(pf.getByRole('heading', { name: '✓ 捐助成功！' })).toBeVisible()
  await closeAll([owner, friend, stranger])
})

test('個人頁 /u/{handle}：公開 / 僅好友清單依身分可見；捐助依可見度與清單類型呈現', async ({ browser }) => {
  const owner = await withHandle('uo'), friend = await registerUser({ tag: 'uf' }), stranger = await registerUser({ tag: 'us' })
  await befriend(owner, friend)
  const pub = await listWith(owner, 'public'), fr = await listWith(owner, 'friends')

  // 另一位送禮者（donor）：對公開清單做公開 / 私人捐助，兩種品項
  const donor = await withHandle('ud')
  const two = await listWith(owner, 'public', {}, [{ title: '公開禮', qty: 1 }, { title: '私人禮', qty: 1 }])
  await claimApi(donor, two.items[0].id, 'public')
  await claimApi(donor, two.items[1].id, 'private')

  const anon = await pageFor(browser), pf = await pageFor(browser, friend)
  await anon.goto(`/u/${owner.handle}`)
  await expect(anon.getByRole('link', { name: pub.title })).toBeVisible()
  await expect(anon.getByRole('link', { name: fr.title })).toHaveCount(0)
  await expect(anon.getByRole('link', { name: '登入以加好友' })).toBeVisible()

  await pf.goto(`/u/${owner.handle}`)
  await expect(pf.getByRole('link', { name: pub.title })).toBeVisible()
  await expect(pf.getByRole('link', { name: fr.title })).toBeVisible()

  // 捐助紀錄在 donor 的個人頁
  await anon.goto(`/u/${donor.handle}`)
  const don = anon.locator('article', { hasText: '公開禮' })
  await expect(don).toContainText(two.title)
  await expect(don.getByRole('link')).toHaveCount(1) // 公開清單給連結
  await expect(anon.getByText('私人禮')).toHaveCount(0)
  const ps = await pageFor(browser, stranger)
  await ps.goto(`/u/${donor.handle}`)
  await expect(ps.getByText('公開禮')).toBeVisible()
  await expect(ps.getByText('私人禮')).toHaveCount(0)
  // 本人看得到自己的私人捐助
  const pd = await pageFor(browser, donor)
  await pd.goto(`/u/${donor.handle}`)
  await expect(pd.getByText('私人禮')).toBeVisible()
  await closeAll([owner, friend, stranger, donor])
})

test('指定好友清單：被指定的好友可開，其他好友被擋', async ({ browser }) => {
  const owner = await withHandle('so'), allowed = await registerUser({ tag: 'sa' }), other = await registerUser({ tag: 'sb' })
  await befriend(owner, allowed); await befriend(owner, other)
  const list = await listWith(owner, 'selected')
  const me = await (await ok(await allowed.api.get('me'), 'me')).json()
  await ok(await owner.api.put(`wishlists/${list.id}/allowed-users`, { data: { user_ids: [me.id] } }), '指定名單')

  const pa = await pageFor(browser, allowed), po = await pageFor(browser, other)
  await pa.goto(`/s/${list.slug}`)
  await expect(pa.getByRole('heading', { level: 1, name: list.title })).toBeVisible()
  await po.goto(`/s/${list.slug}`)
  await expect(po.getByRole('heading', { name: '你沒有權限查看這份清單' })).toBeVisible()
  await closeAll([owner, allowed, other])
})

test('探索頁 /explore：訪客看得到公開清單，看不到僅好友清單', async ({ browser }) => {
  const owner = await registerUser({ tag: 'xo' })
  const pub = await listWith(owner, 'public'), fr = await listWith(owner, 'friends')
  const anon = await pageFor(browser)
  await anon.goto('/explore'); await hydrated(anon)
  // 其他測試同時建公開清單，目標可能不在第一頁：一路「載入更多」直到出現（最多 10 頁）
  const card = anon.getByRole('link', { name: pub.title })
  for (let i = 0; i < 10 && !(await card.isVisible()); i++) {
    const more = anon.getByRole('button', { name: '載入更多' })
    if (!(await more.isVisible())) break
    await more.click(); await anon.waitForTimeout(300)
  }
  await expect(card).toBeVisible()
  await expect(anon.getByRole('link', { name: fr.title })).toHaveCount(0)
  await card.click()
  await expect(anon.getByRole('heading', { level: 1, name: pub.title })).toBeVisible()
  await closeAll([owner])
})

test('好友動態：好友新增願望出現在 /friends，點「去送禮」到分享頁', async ({ browser }) => {
  const owner = await withHandle('fo'), friend = await withHandle('ff')
  await befriend(owner, friend)
  const list = await listWith(owner, 'friends')
  const p = await pageFor(browser, friend)
  await p.goto('/friends'); await hydrated(p)
  const card = p.locator('article', { hasText: owner.name })
  await expect(card).toContainText('奶瓶')
  await card.getByRole('link', { name: '去送禮' }).click()
  await expect(p).toHaveURL(new RegExp(`/s/${list.slug}$`))
  await closeAll([owner, friend])
})
