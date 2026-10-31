// 修正後的瀏覽器重測（docs/role-flows/findings.md §6）：只測本分支在 Web 端修掉的項目。
// 前提同其他角色腳本（Web :3000 需以本分支重新建置）。
import { createHmac } from 'node:crypto'
import { registerApi, newPage, recorder, closeBrowser, sql, q, uid, randomUUID } from './lib.mjs'

const run = recorder('verify-fixes')
const owner = await registerApi({ name: '重測受捐者', tag: 'vf' })
const donor = await registerApi({ name: '重測捐助者', tag: 'vfd' })

// 前置：公開清單，設 closes_at = 台北時間明天 20:30（UTC 12:30）
const w = await (await owner.api.post('wishlists', { data: { type: 'personal', title: `重測清單 ${uid()}`, visibility: 'public' } })).json()
const it = await (await owner.api.post(`wishlists/${w.id}/items`, { data: { title: '保溫瓶', qty_needed: 1, priority: 'medium', funding_mode: 'quantity' } })).json()
const cur = await (await owner.api.get(`wishlists/${w.id}`)).json()
const tomorrow = new Date(Date.now() + 864e5).toISOString().slice(0, 10)
const closes = `${tomorrow}T12:30:00Z`
const pr = await owner.api.patch(`wishlists/${w.id}`, { data: { status: 'active', closes_at: closes, expected_updated_at: cur.wishlist.updated_at } })
if (!pr.ok()) throw new Error('publish ' + pr.status() + await pr.text())

await (async () => {
  const page = await newPage()
  await run.step('V-01', 'R-P1 分享頁結束時間為台北時間、無 hydration 警告', async ({ note }) => {
    await page.goto(`/s/${w.slug}`)
    await page.waitForLoadState('networkidle')
    const meta = await page.locator('.sx-meta').innerText()
    note(meta)
    if (!/20:30|晚上\s?8:30/.test(meta)) throw new Error('結束時間不是台北 20:30：' + meta)
    const hyd = page.problems.filter(p => /hydrat/i.test(p))
    if (hyd.length) throw new Error('hydration：' + hyd.join(' | '))
  }, { page })

  await run.step('V-02', 'D-P2 未登入 /me/claims 空狀態有登入入口', async ({ note }) => {
    await page.goto('/me/claims')
    const link = page.getByRole('link', { name: '登入' })
    await link.waitFor({ timeout: 10000 })
    note(await link.getAttribute('href'))
    if (!/redirect=(%2F|\/)me(%2F|\/)claims/.test(await link.getAttribute('href'))) throw new Error('沒有帶回原頁')
  }, { page })

  await run.step('V-03', 'D-P3 生日通知退訂連結顯示生日文案', async ({ note }) => {
    // 平台外：以 dev 預設 UNSUB_SECRET 簽出權杖（與 account.rs unsub_token 相同格式）
    const id = sql(`SELECT id FROM users WHERE email='${q(owner.email)}'`)
    const msg = `b:${id}:${Math.floor(Date.now() / 1000) + 86400}`
    const token = `${Buffer.from(msg).toString('base64url')}.${createHmac('sha256', 'dev-unsub-secret').update(msg).digest('hex')}`
    await page.goto(`/unsubscribe?token=${token}`)
    await page.getByRole('heading', { name: '確定要退訂生日通知？' }).waitFor({ timeout: 10000 })
    await page.getByRole('button', { name: '確定退訂' }).click()
    await page.getByText('之後不會再寄好友生日提醒').waitFor({ timeout: 10000 })
    note('問句與完成文案皆為生日通知')
    if (sql(`SELECT notification_prefs->>'email_birthdays' FROM users WHERE id='${id}'`) !== 'false') throw new Error('email_birthdays 未關閉')
  }, { page })

  await run.step('V-04', 'P-P15 非 staff 進 /admin 顯示 404', async ({ note }) => {
    const p2 = await newPage({ user: donor })
    await p2.goto('/admin')
    await p2.getByText('404', { exact: true }).waitFor({ timeout: 10000 })
    note(await p2.locator('.er-code').innerText())
    await p2.context().close()
  })
})()

await (async () => {
  const page = await newPage({ user: donor })
  await run.step('V-05', 'D-P1 登入的捐助者在分享頁看得到帳號選單（錢包、登出）', async ({ note }) => {
    await page.goto('/login') // 讓 ws_hint（Web 網域）在登入流程中被設定：直接用 API cookie 時改為先開 dashboard
    await page.goto('/dashboard')
    await page.waitForLoadState('networkidle')
    await page.goto(`/s/${w.slug}`)
    const btn = page.getByRole('button', { name: /帳號選單/ })
    await btn.waitFor({ timeout: 10000 })
    await btn.click()
    await page.getByRole('link', { name: '點數' }).waitFor()
    await page.getByRole('button', { name: '登出' }).waitFor()
    note('帳號選單含點數、登出')
  }, { page })

  await run.step('V-06', 'D-P4 搶輸最後一件後分享頁留下提示', async ({ note }) => {
    await page.goto(`/s/${w.slug}`)
    await page.waitForLoadState('networkidle')
    // 先打開認領 sheet，再由另一位用戶（平台外）把唯一一件認領走；SSE 會更新列表，但已開的 sheet 仍會送出
    await page.getByRole('button', { name: '我來送' }).first().click()
    const other = await registerApi({ tag: 'vfo' })
    const r = await other.api.post(`items/${it.id}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': randomUUID() } })
    if (r.status() !== 201) throw new Error('前置認領 ' + r.status())
    await page.getByRole('button', { name: '確定要送' }).click()
    await page.locator('.g-toast').waitFor({ timeout: 10000 })
    note(await page.locator('.g-toast').innerText())
    await page.waitForTimeout(1500)
    if (!(await page.locator('.g-toast').isVisible())) throw new Error('sheet 關閉後提示消失')
  }, { page })
})()

await (async () => {
  // 前置：草稿組織（未驗證）→ 後台橫幅、無新增入口
  const o = await (await owner.api.post('orgs', { data: { legal_name: `重測協會${uid()}`, display_name: `重測協會${uid()}`, org_type: 'association' } })).json()
  const page = await newPage({ user: owner })
  await run.step('V-07', 'O-P4 未驗證組織後台顯示橫幅、隱藏新增事件／據點', async ({ note }) => {
    if (!o.id) throw new Error('建立組織失敗：' + JSON.stringify(o))
    await page.goto(`/orgs/${o.id}`)
    await page.getByText('組織目前不是「已驗證」狀態').waitFor({ timeout: 10000 })
    const n = await page.getByRole('link', { name: /新增事件|新增據點/ }).count()
    note(`新增入口數：${n}`)
    if (n) throw new Error('仍顯示新增入口')
  }, { page })
})()

run.save()
await closeBrowser()
