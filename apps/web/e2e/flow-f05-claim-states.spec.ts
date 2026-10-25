// 流程驗證 F5（docs/02 捐助狀態變更與取消）：真實 API，涵蓋捐助者與擁有者兩端的 UI 操作。
import { expect, test } from '@playwright/test'
import { createList, loginBrowser, memberClaim, registerUser, signInPage } from './helpers'

test('捐助者：已購買 → 已送達；連點不出錯；送達後不可取消', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '尿布', qty: 2 }] })
  const donor = await signInPage(page)
  await memberClaim(list.items[0]!.id, 1, donor)

  await page.goto('/me/claims')
  const card = page.locator('.g-item', { hasText: '尿布' })
  await expect(card.getByText('已捐助')).toBeVisible()
  // 連點同一按鈕：第二個請求是同一目標狀態，後端冪等回 200，不可出現「不允許的狀態轉換」
  const mark = card.getByRole('button', { name: '標記已購買' })
  const patches: number[] = []
  page.on('response', r => { if (r.request().method() === 'PATCH' && r.url().includes('/claims/')) patches.push(r.status()) })
  await mark.evaluate((b: HTMLButtonElement) => { b.click(); b.click() })
  await expect.poll(() => patches.length).toBe(2)
  expect(patches).toEqual([200, 200])
  await expect(card.getByText('已購買')).toBeVisible()
  await expect(page.getByRole('alert')).toHaveCount(0)

  await card.getByRole('button', { name: '標記已送達' }).click()
  await expect(card.getByText('已送達')).toBeVisible()
  await expect(card.getByRole('button', { name: '取消' })).toHaveCount(0)
  await page.reload()
  await expect(page.locator('.g-item', { hasText: '尿布' }).getByText('已送達')).toBeVisible()
})

test('擁有者：進度頁標記已送達、取消他人捐助（需確認）後數量回補', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '奶粉', qty: 3 }] })
  await memberClaim(list.items[0]!.id, 1)
  await memberClaim(list.items[0]!.id, 2)
  await loginBrowser(page.context(), owner)

  await page.goto(`/lists/${list.id}/progress`)
  const rows = page.locator('article', { hasText: '奶粉' }).getByRole('listitem')
  await expect(rows).toHaveCount(2)
  // 標為已送達不可復原，會先跳 confirm
  const delivered = page.waitForEvent('dialog').then(d => d.accept())
  await rows.filter({ hasText: '×1' }).getByRole('button', { name: '標為已送達' }).click()
  await delivered
  // exact：否則會比對到按鈕「標為已送達」而提早通過，下一步 click 時 PATCH 還沒回、×1 列按鈕消失造成版面上移，點到 article
  await expect(rows.filter({ hasText: '×1' }).getByText('已送達', { exact: true })).toBeVisible()

  // 逐次等對話框出現再處理：進度頁會因 SSE 重抓而重繪，用 page.once 可能讓處理器留到下一次
  const cancel = async (accept: boolean) => {
    // confirm() 開著時 click 不會結束，所以對話框一出現就處理，再等 click 與處理都完成
    const handled = page.waitForEvent('dialog').then(d => accept ? d.accept() : d.dismiss())
    await rows.filter({ hasText: '×2' }).getByRole('button', { name: '取消捐助' }).click()
    await handled
  }
  await cancel(false)
  await expect(rows.filter({ hasText: '×2' }).getByText('已捐助')).toBeVisible()

  await cancel(true)
  await expect(rows.filter({ hasText: '×2' }).getByText('已取消')).toBeVisible()
  await expect(page.getByText('數量 1 / 3')).toBeVisible()
})

test('非擁有者開別人的進度頁：顯示找不到，而不是「連線失敗」', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner)
  await signInPage(page)
  await page.goto(`/lists/${list.id}/progress`)
  await expect(page.getByRole('alert')).toContainText('找不到')
  await expect(page.getByRole('alert')).not.toContainText('連線失敗')
})

test('進度頁：SSE 觸發的較早重抓晚到時，不可蓋掉操作後的新狀態', async ({ page }) => {
  const owner = await registerUser({ tag: 'owner' })
  const list = await createList(owner, { items: [{ title: '奶嘴', qty: 2 }, { title: '浴巾', qty: 2 }] })
  await memberClaim(list.items[0]!.id, 1)
  await loginBrowser(page.context(), owner)
  await page.goto(`/lists/${list.id}/progress`)
  const row = page.locator('article', { hasText: '奶嘴' }).getByRole('listitem')
  await expect(row.getByText('已捐助')).toBeVisible()
  await expect(page.getByText('即時更新中')).toBeVisible()

  // 下一次 dashboard 請求（SSE 觸發）先拿到當下快照，但延遲 2 秒才交給頁面
  let held: Promise<void> | undefined
  let release!: () => void
  await page.route('**/dashboard', async route => {
    if (held) return route.continue()
    held = new Promise(r => (release = r))
    const res = await route.fetch()
    await held
    await route.fulfill({ response: res })
  })
  await memberClaim(list.items[1]!.id, 1) // 觸發 SSE → 重抓（被延遲，內容是取消前的快照）
  await expect.poll(() => !!held).toBe(true)

  // 取消後的重抓（操作本身 + 取消觸發的 SSE）都完成之後，才放行較早的那個請求，確保它最後到
  let fresh = 0
  page.on('response', r => { if (r.url().endsWith('/dashboard')) fresh++ })
  const handled = page.waitForEvent('dialog').then(d => d.accept())
  await row.getByRole('button', { name: '取消捐助' }).click()
  await handled
  await expect(row.getByText('已取消')).toBeVisible()
  await expect.poll(() => fresh).toBeGreaterThanOrEqual(2)
  await page.waitForTimeout(800) // 沒有更多重抓排程
  release()
  await expect.poll(() => fresh).toBeGreaterThanOrEqual(3)
  await page.waitForTimeout(300)
  await expect(row.getByText('已取消')).toBeVisible()
})
