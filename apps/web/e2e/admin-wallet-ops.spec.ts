import { expect, test, type Page } from '@playwright/test'
import { loginBrowser, makeStaff, registerUser } from './helpers'
import { hydrated } from './support/hydrate'
import { UUID, iso, mock, ok, problem, type Res } from './support/mockApi'

// 營運後台三頁：現金退款審核、拒付欠款、儲值爭議。登入與 staff 權限走真後端（makeStaff），三組 admin 端點以 page.route 模擬。
type Row = Record<string, any>
const user = (n: number) => ({ id: `u${n}`, display_name: `使用者${n}`, email: `u${n}@example.com` })

async function asStaff(page: Page) {
  const u = await registerUser({ tag: 'ops' })
  makeStaff(u.email)
  await loginBrowser(page.context(), u)
  return u
}
/** 在確認對話框內按確認（按鈕文字與動作名稱相同） */
const dialog = (page: Page) => page.getByRole('dialog')

// ---------- 現金退款 ----------
const refund = (id: string, status: string, o: Row = {}) => ({ id, amount: 1000, status, user: user(1), lot_allocations: [], reject_reason: null, processed_by: null, created_at: iso(-36e5), completed_at: null, ...o })

async function mockRefunds(page: Page, rows: Row[], patch?: (n: number) => Res | undefined) {
  const list = await mock(page, /\/api\/v1\/admin\/cash-refunds(\?|$)/, (c) => {
    const st = c.url.searchParams.get('status')
    return ok({ data: rows.filter(r => !st || r.status === st), next_cursor: null })
  })
  const patches = await mock(page, /\/api\/v1\/admin\/cash-refunds\/[^/?]+$/, (c, n) => {
    const forced = patch?.(n); if (forced) return forced
    const row = rows.find(r => c.url.pathname.endsWith(`/${r.id}`))!
    Object.assign(row, { status: c.body.status, reject_reason: c.body.reject_reason ?? null, completed_at: c.body.status === 'processing' ? null : iso() })
    return ok({ id: row.id, amount: row.amount, status: row.status, reject_reason: row.reject_reason, processed_by: 'staff1', completed_at: row.completed_at })
  })
  return { list, patches }
}

test('現金退款：預設只看待審核，可依狀態篩選；處理中 → 已退款，每步都有二次確認', async ({ page }) => {
  await asStaff(page)
  const m = await mockRefunds(page, [refund('r1', 'requested'), refund('r2', 'processing', { amount: 800, user: user(2) }), refund('r3', 'completed', { completed_at: iso() }), refund('r4', 'rejected', { reject_reason: '資料不符' })])
  await page.goto('/admin/cash-refunds'); await hydrated(page)

  await expect.poll(() => m.list.length).toBeGreaterThan(0)
  expect(m.list[0].url.searchParams.get('status')).toBe('requested')
  const cards = page.locator('article[data-status]')
  await expect(cards).toHaveCount(1)
  await expect(cards.first().locator('.c-badge')).toHaveText('待審核')
  await expect(cards.first()).toContainText('使用者1')
  await expect(cards.first()).toContainText('u1@example.com')
  await expect(cards.first()).toContainText('1,000 點')
  // requested 只能「開始處理」或「拒絕」，不能直接標記已退款（後端狀態機）
  await expect(cards.first().getByRole('button', { name: '標記已退款' })).toHaveCount(0)

  await cards.first().getByRole('button', { name: '開始處理' }).click()
  await expect(dialog(page)).toContainText('標為處理中')
  await dialog(page).getByRole('button', { name: '取消' }).click()
  expect(m.patches).toHaveLength(0) // 取消不送出
  await cards.first().getByRole('button', { name: '開始處理' }).click()
  await dialog(page).getByRole('button', { name: '開始處理' }).click()
  await expect(page.getByRole('status').filter({ hasText: '標為處理中' })).toBeVisible()
  expect(m.patches[0].method).toBe('PATCH')
  expect(m.patches[0].url.pathname).toMatch(/\/admin\/cash-refunds\/r1$/)
  expect(m.patches[0].body).toEqual({ status: 'processing' })
  await expect(cards.first().locator('.c-badge')).toHaveText('處理中') // 列就地更新

  await cards.first().getByRole('button', { name: '標記已退款' }).click()
  await expect(dialog(page)).toContainText('確認已完成對 使用者1 的 NT$ 1,000 退款？此動作無法復原。')
  await dialog(page).getByRole('button', { name: '標記已退款' }).click()
  await expect(cards.first().locator('.c-badge')).toHaveText('已退款')
  expect(m.patches[1].body).toEqual({ status: 'completed' })
  await expect(cards.first().getByRole('button')).toHaveCount(0) // 已結案沒有動作

  await page.getByLabel('狀態').selectOption('')
  await expect(cards).toHaveCount(4)
  expect(m.list.at(-1)!.url.searchParams.get('status')).toBeNull()
  await expect(cards.nth(3)).toContainText('資料不符')
})

test('現金退款：拒絕必填理由（空白不送出），送出 reject_reason', async ({ page }) => {
  await asStaff(page)
  const m = await mockRefunds(page, [refund('r1', 'requested')])
  await page.goto('/admin/cash-refunds'); await hydrated(page)

  await page.locator('article[data-status]').first().getByRole('button', { name: '拒絕並退點' }).click()
  await expect(dialog(page)).toContainText('點數會依原批次退回他的錢包')
  await dialog(page).getByLabel('拒絕原因').fill('   ')
  await dialog(page).getByRole('button', { name: '拒絕並退點' }).click()
  await expect(dialog(page).getByRole('alert')).toHaveText('請填寫拒絕原因')
  expect(m.patches).toHaveLength(0)

  await dialog(page).getByLabel('拒絕原因').fill('退款帳戶資料不符')
  await dialog(page).getByRole('button', { name: '拒絕並退點' }).click()
  await expect(page.getByRole('status').filter({ hasText: '已拒絕 使用者1 的申請，1,000 點已退回其錢包' })).toBeVisible()
  expect(m.patches[0].body).toEqual({ status: 'rejected', reject_reason: '退款帳戶資料不符' })
  const card = page.locator('article[data-status="rejected"]')
  await expect(card.locator('.c-badge')).toHaveText('已拒絕')
  await expect(card).toContainText('退款帳戶資料不符')
})

test('現金退款：他人已處理（409）→ 顯示說明並重讀列表；其他錯誤顯示在對話框', async ({ page }) => {
  await asStaff(page)
  const m = await mockRefunds(page, [refund('r1', 'requested')], n => (n === 1 ? problem(409, 'INVALID_STATE_TRANSITION', 'x') : n === 2 ? problem(500, 'INTERNAL', '內部錯誤') : undefined))
  await page.goto('/admin/cash-refunds'); await hydrated(page)
  const card = page.locator('article[data-status]').first()
  await card.getByRole('button', { name: '開始處理' }).click()
  await dialog(page).getByRole('button', { name: '開始處理' }).click()
  await expect(dialog(page)).not.toBeVisible() // 對話框關閉，說明放在頁面上
  await expect(page.getByRole('alert').filter({ hasText: '這筆申請的狀態已被其他人變更，已重新載入列表。' })).toBeVisible()
  expect(m.list).toHaveLength(2)
  await card.getByRole('button', { name: '開始處理' }).click()
  await dialog(page).getByRole('button', { name: '開始處理' }).click()
  await expect(dialog(page).getByRole('alert')).toBeVisible() // 500：訊息留在對話框，可重試
  await dialog(page).getByRole('button', { name: '開始處理' }).click()
  await expect(dialog(page)).not.toBeVisible()
  await expect(card.locator('.c-badge')).toHaveText('處理中')
})

test('現金退款：空列表與載入失敗可重試', async ({ page }) => {
  await asStaff(page)
  let fail = true
  await mock(page, /\/api\/v1\/admin\/cash-refunds(\?|$)/, () => (fail ? problem(500, 'INTERNAL', '內部錯誤') : ok({ data: [], next_cursor: null })))
  await page.goto('/admin/cash-refunds'); await hydrated(page)
  await expect(page.getByRole('alert')).toContainText('系統暫時發生問題')
  fail = false
  await page.getByRole('button', { name: '重試' }).click()
  await expect(page.getByText('沒有符合的申請')).toBeVisible()
})

// ---------- 欠款 ----------
const debt = (id: string, status: string, o: Row = {}) => ({ id, amount: 1500, status, topup_id: `tp-${id}`, user: user(3), note: null, resolved_at: null, created_at: iso(-864e5), ...o })

test('欠款：預設 open；結案（已追回 / 核銷）備註必填並送 status + note', async ({ page }) => {
  await asStaff(page)
  const rows = [debt('d1', 'open'), debt('d2', 'open', { amount: 400, user: user(4) }), debt('d3', 'settled', { note: '已匯款', resolved_at: iso() })]
  const list = await mock(page, /\/api\/v1\/admin\/wallet-debts(\?|$)/, c => ok({ data: rows.filter(r => r.status === c.url.searchParams.get('status')), next_cursor: null }))
  const patches = await mock(page, /\/api\/v1\/admin\/wallet-debts\/[^/?]+$/, (c) => {
    const row = rows.find(r => c.url.pathname.endsWith(`/${r.id}`))!
    Object.assign(row, { status: c.body.status, note: c.body.note, resolved_at: iso() })
    return ok({ id: row.id, status: row.status, note: row.note, resolved_at: row.resolved_at })
  })
  await page.goto('/admin/wallet-debts'); await hydrated(page)

  await expect.poll(() => list.length).toBeGreaterThan(0) // 列表在 onMounted 才請求，hydrated 後不一定已送出
  expect(list[0].url.searchParams.get('status')).toBe('open')
  const cards = page.locator('article[data-status]')
  await expect(cards).toHaveCount(2)
  await expect(cards.first()).toContainText('1,500 點')
  await expect(cards.first()).toContainText('tp-d1')

  await cards.first().getByRole('button', { name: '標記已追回' }).click()
  await dialog(page).getByRole('button', { name: '標記已追回' }).click()
  await expect(dialog(page).getByRole('alert')).toHaveText('請填寫備註')
  expect(patches).toHaveLength(0)
  await dialog(page).getByLabel('備註').fill('已向使用者追回')
  await dialog(page).getByRole('button', { name: '標記已追回' }).click()
  await expect(page.getByRole('status').filter({ hasText: '已將 使用者3 的 1,500 點欠款標為已追回' })).toBeVisible()
  expect(patches[0].body).toEqual({ status: 'settled', note: '已向使用者追回' })
  await expect(cards.first().locator('.c-badge')).toHaveText('已追回')
  await expect(cards.first().getByRole('button')).toHaveCount(0)

  await cards.nth(1).getByRole('button', { name: '核銷' }).click()
  await expect(dialog(page)).toContainText('平台吸收損失並列帳')
  await dialog(page).getByLabel('備註').fill('金額小於追討成本')
  await dialog(page).getByRole('button', { name: '核銷' }).click()
  await expect(page.getByRole('status').filter({ hasText: '已核銷 使用者4 的 400 點欠款' })).toBeVisible()
  expect(patches[1].body).toEqual({ status: 'written_off', note: '金額小於追討成本' })
  await expect(cards.nth(1).locator('.c-badge')).toHaveText('已核銷')

  await page.getByLabel('狀態').selectOption('settled')
  await expect.poll(() => list.at(-1)!.url.searchParams.get('status')).toBe('settled')
  await expect(cards.filter({ hasText: '已匯款' })).toHaveCount(1) // 先前已結案的那筆
  await expect(cards).toHaveCount(2) // 加上剛才標為已追回的 d1（d2 已核銷，不在此篩選）
})

test('欠款：已被結案（409）顯示說明並重讀', async ({ page }) => {
  await asStaff(page)
  const list = await mock(page, /\/api\/v1\/admin\/wallet-debts(\?|$)/, () => ok({ data: [debt('d1', 'open')], next_cursor: null }))
  await mock(page, /\/api\/v1\/admin\/wallet-debts\/[^/?]+$/, () => problem(409, 'INVALID_STATE_TRANSITION', 'x'))
  await page.goto('/admin/wallet-debts'); await hydrated(page)
  await page.locator('article[data-status]').first().getByRole('button', { name: '核銷' }).click()
  await dialog(page).getByLabel('備註').fill('x')
  await dialog(page).getByRole('button', { name: '核銷' }).click()
  await expect(dialog(page)).not.toBeVisible()
  await expect(page.getByRole('alert').filter({ hasText: '這筆欠款已被結案，已重新載入列表。' })).toBeVisible()
  expect(list).toHaveLength(2)
})

// ---------- 儲值爭議 ----------
const topup = (id: string, status: string, o: Row = {}) => ({ id, amount: 3000, method: 'card', status, provider: 'ecpay', provider_txn_id: `T-${id}`, created_at: iso(-864e5), paid_at: iso(-864e5 + 6e4), user: user(5), wallet: { id: 'w5', status: status === 'disputed' ? 'frozen' : 'active' }, ...o })

async function mockTopups(page: Page, rows: Row[], dispute: (n: number, c: { body: any }) => Res) {
  const list = await mock(page, /\/api\/v1\/admin\/topups(\?|$)/, (c) => {
    const st = c.url.searchParams.get('status')
    return ok({ data: rows.filter(r => !st || r.status === st), next_cursor: null })
  })
  const posts = await mock(page, /\/api\/v1\/admin\/topups\/[^/]+\/dispute$/, (c, n) => dispute(n, c))
  return { list, posts }
}

test('爭議：預設看爭議中；顯示使用者與錢包凍結；勝訴 / 敗訴（扣回）各有二次確認與結果', async ({ page }) => {
  await asStaff(page)
  const rows = [topup('t1', 'disputed'), topup('t2', 'disputed', { amount: 1000, user: user(6) })]
  const m = await mockTopups(page, rows, (n, c) => {
    const row = rows[n - 1]
    if (c.body.action === 'won') { row.status = 'paid'; row.wallet.status = 'active'; return ok({ topup_id: row.id, status: 'paid', applied: true }) }
    row.status = 'charged_back'; return ok({ topup_id: row.id, status: 'charged_back', applied: true, recovered: 600, debt: 2400 })
  })
  await page.goto('/admin/topups'); await hydrated(page)

  await expect.poll(() => m.list.length).toBeGreaterThan(0)
  expect(m.list[0].url.searchParams.get('status')).toBe('disputed')
  const cards = page.locator('article[data-status]')
  await expect(cards).toHaveCount(2)
  await expect(cards.first()).toContainText('使用者5')
  await expect(cards.first()).toContainText('u5@example.com')
  await expect(cards.first()).toContainText('T-t1')
  await expect(cards.first().getByText('錢包已凍結')).toBeVisible()
  await expect(cards.first().locator('.c-badge.disputed')).toHaveText('爭議中')
  await expect(cards.first().getByRole('button', { name: '標記爭議' })).toHaveCount(0)

  // 勝訴
  await cards.first().getByRole('button', { name: '爭議勝訴' }).click()
  await expect(dialog(page)).toContainText('儲值單恢復為已入帳')
  await dialog(page).getByLabel('備註').fill('金流商撤銷拒付')
  await dialog(page).getByRole('button', { name: '爭議勝訴' }).click()
  await expect(page.getByRole('status').filter({ hasText: '已將 使用者5 的 NT$ 3,000 判為爭議勝訴' })).toBeVisible()
  expect(m.posts[0].body).toEqual({ action: 'won', note: '金流商撤銷拒付' })
  expect(m.posts[0].headers['idempotency-key']).toMatch(UUID)
  expect(m.posts[0].url.pathname).toMatch(/\/admin\/topups\/t1\/dispute$/)
  await expect(cards.first().locator('.c-badge.paid')).toHaveText('已入帳')

  // 敗訴（扣回）：note 不填就不送 note；結果顯示扣回與欠款
  await cards.nth(1).getByRole('button', { name: '拒付敗訴（扣回）' }).click()
  await expect(dialog(page)).toContainText('會扣回該筆儲值批次剩餘的點數，已花掉的部分記為欠款')
  await dialog(page).getByRole('button', { name: '拒付敗訴（扣回）' }).click()
  await expect(page.getByRole('status').filter({ hasText: '扣回 600 點，記為欠款 2,400 點' })).toBeVisible()
  expect(m.posts[1].body).toEqual({ action: 'lost' })
  expect(m.posts[1].headers['idempotency-key']).toMatch(UUID)
  expect(m.posts[1].headers['idempotency-key']).not.toBe(m.posts[0].headers['idempotency-key'])
  await expect(cards.nth(1).locator('.c-badge.charged_back')).toHaveText('已拒付')
  await expect(cards.nth(1).getByRole('button')).toHaveCount(0)
})

test('爭議：切到已入帳 → 標記爭議（錢包被凍結）；取消不送出', async ({ page }) => {
  await asStaff(page)
  const rows = [topup('t3', 'paid')]
  const m = await mockTopups(page, rows, () => { rows[0].status = 'disputed'; return ok({ topup_id: 't3', status: 'disputed', applied: true }) })
  await page.goto('/admin/topups'); await hydrated(page)
  await expect(page.getByText('沒有符合的儲值單')).toBeVisible()
  await page.getByLabel('狀態').selectOption('paid')
  const card = page.locator('article[data-status="paid"]')
  await expect(card).toHaveCount(1)
  await expect(card.getByText('錢包已凍結')).toHaveCount(0)

  await card.getByRole('button', { name: '標記爭議' }).click()
  await expect(dialog(page)).toContainText('錢包會被凍結')
  await dialog(page).getByRole('button', { name: '取消' }).click()
  expect(m.posts).toHaveLength(0)
  await card.getByRole('button', { name: '標記爭議' }).click()
  await dialog(page).getByRole('button', { name: '標記爭議' }).click()
  await expect(page.getByRole('status').filter({ hasText: '標為爭議中，錢包已凍結' })).toBeVisible()
  expect(m.posts[0].body).toEqual({ action: 'open' })
  const after = page.locator('article[data-status="disputed"]')
  await expect(after.getByText('錢包已凍結')).toBeVisible()
  await expect(after.getByRole('button', { name: '爭議勝訴' })).toBeVisible()
})

test('爭議：5xx 留在對話框可重試；409 關閉並說明；applied=false（已處理過）不謊報成功', async ({ page }) => {
  await asStaff(page)
  const rows = [topup('t4', 'disputed')]
  const m = await mockTopups(page, rows, (n) => {
    if (n === 1) return { status: 500, json: { code: 'INTERNAL', detail: '內部錯誤' } }
    if (n === 2) return problem(409, 'INVALID_STATE_TRANSITION', 'x')
    return ok({ topup_id: 't4', status: 'paid', applied: false })
  })
  await page.goto('/admin/topups'); await hydrated(page)
  const card = page.locator('article[data-status]').first()
  await card.getByRole('button', { name: '爭議勝訴' }).click()
  await dialog(page).getByRole('button', { name: '爭議勝訴' }).click()
  await expect(dialog(page).getByRole('alert')).toBeVisible()
  await dialog(page).getByRole('button', { name: '爭議勝訴' }).click()
  await expect(dialog(page)).not.toBeVisible()
  await expect(page.getByRole('alert').filter({ hasText: '這筆儲值單的狀態不允許此操作，已重新載入列表。' })).toBeVisible()
  expect(m.list).toHaveLength(2)
  await card.getByRole('button', { name: '爭議勝訴' }).click()
  await dialog(page).getByRole('button', { name: '爭議勝訴' }).click()
  await expect(page.getByRole('status').filter({ hasText: '這筆儲值單的狀態已變更，沒有做任何動作' })).toBeVisible()
  await expect(page.getByRole('status').filter({ hasText: '判為爭議勝訴' })).toHaveCount(0)
  expect(m.posts).toHaveLength(3)
})

// ---------- 導覽與權限 ----------
test('後台首頁有三個新入口，可導向各頁', async ({ page }) => {
  await asStaff(page)
  await mockRefunds(page, [])
  await mock(page, /\/api\/v1\/admin\/wallet-debts(\?|$)/, () => ok({ data: [], next_cursor: null }))
  await mock(page, /\/api\/v1\/admin\/topups(\?|$)/, () => ok({ data: [], next_cursor: null }))
  await page.goto('/admin'); await hydrated(page)
  for (const [name, path, heading] of [['現金退款', 'cash-refunds', '現金退款審核'], ['拒付欠款', 'wallet-debts', '拒付欠款'], ['儲值與爭議', 'topups', '儲值與拒付爭議']]) {
    await page.getByRole('link', { name, exact: true }).click()
    await expect(page).toHaveURL(new RegExp(`/admin/${path}$`))
    await expect(page.getByRole('heading', { name: heading, exact: true }).first()).toBeVisible()
    await page.goBack(); await hydrated(page)
  }
})

test('非營運人員看不到三個後台頁（看起來像 404）', async ({ page, context }) => {
  const u = await registerUser()
  await loginBrowser(context, u)
  for (const p of ['cash-refunds', 'wallet-debts', 'topups']) {
    await page.goto(`/admin/${p}`)
    await expect(page.getByRole('heading', { name: '找不到頁面' })).toBeVisible()
  }
  await u.api.dispose()
})
