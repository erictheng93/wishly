import { expect, test, type Page } from '@playwright/test'
import { mkdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { loginBrowser, memberClaim, registerUser, sql, uid, type User } from './helpers'

// 組織入口、「接下來」引導、申請第 3 步把關，與「捐助者 QR → 志工掃描預填實收」
const SHOTS = resolve(dirname(fileURLToPath(import.meta.url)), '../.shots')
mkdirSync(SHOTS, { recursive: true })
// 手機 390 與桌面 1280 各截一張，截完還原寬度
async function shot(p: Page, name: string, el?: ReturnType<Page['locator']>) {
  const orig = p.viewportSize()
  for (const w of [390, 1280]) {
    await p.setViewportSize({ width: w, height: 900 })
    await p.waitForTimeout(200)
    await (el ?? p).screenshot({ path: `${SHOTS}/${name}-${w}.png`, ...(el ? {} : { fullPage: true }) })
  }
  if (orig) await p.setViewportSize(orig)
}

// 統編檢查碼：權重 1,2,1,2,1,2,4,1，各積的數字和可被 5 整除
function taxId(): string {
  const w = [1, 2, 1, 2, 1, 2, 4, 1]
  for (;;) {
    const s = String(Math.floor(Math.random() * 1e8)).padStart(8, '0')
    const sum = [...s].reduce((a, c, i) => { const p = Number(c) * w[i]!; return a + Math.floor(p / 10) + (p % 10) }, 0)
    if (sum % 5 === 0) return s
  }
}

// 欄位 blur 會自動保存並暫時停用按鈕：先點標題讓欄位失焦（Tab 會讓下一個選填欄失焦時存空字串而 422）、等按鈕可按再點，避免點擊被吃掉
async function nextStep(p: Page) {
  await p.locator('section:visible h2').first().click()
  const btn = p.getByRole('button', { name: '下一步' })
  await expect(btn).toBeEnabled()
  await btn.click()
}

async function seedOrg(owner: User) {
  const ownerId = (await (await owner.api.get('me')).json()).id as string
  const name = `E2E 組織 ${uid()}`
  const orgId = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
    VALUES (substr(md5(random()::text), 1, 10), '${name}', '${name}', 'association', '${taxId()}', 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${ownerId}') RETURNING id`).split('\n')[0]!
  sql(`INSERT INTO org_members (org_id, user_id, role) VALUES ('${orgId}', '${ownerId}', 'owner')`)
  sql(`INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to, status) VALUES ('${orgId}', 'P-${uid()}', current_date - 1, current_date + 90, 'approved')`)
  return { orgId, name }
}

test('申請精靈第 3 步：沒有文件停在第 3 步；選了許可檔沒新增會提示', async ({ browser }) => {
  const u = await registerUser({ tag: 'orgapply' })
  const ctx = await browser.newContext()
  await loginBrowser(ctx, u)
  const p = await ctx.newPage()
  await p.goto('/org/apply')
  await p.getByLabel('組織全名', { exact: false }).fill(`E2E 申請 ${uid()}`)
  await p.getByLabel('對外顯示名稱').fill('E2E 申請')
  await p.getByLabel('組織類型').selectOption('association')
  await p.getByLabel('統一編號', { exact: false }).fill(taxId())
  await p.getByLabel('立案字號').fill('R-1')
  await p.getByLabel('主管機關').fill('內政部')
  await nextStep(p)
  await p.getByLabel('負責人姓名').fill('王')
  await p.getByLabel('聯絡窗口姓名').fill('王')
  await p.getByLabel('聯絡窗口手機', { exact: false }).fill('0912345678')
  await p.getByLabel('公開聯絡 email', { exact: false }).fill('o@example.com')
  await nextStep(p)
  await expect(p.getByText('步驟 3/4')).toBeVisible()

  // 沒有任何文件
  await p.setViewportSize({ width: 390, height: 844 })
  await p.getByRole('button', { name: '下一步' }).click()
  const docsErr = p.getByText('請至少上傳 1 份登記證文件，並新增 1 筆許可')
  await expect(docsErr).toBeVisible()
  // 錯誤在頁面最上方、按鈕在最下方：手機視窗內（非 fullPage）必須看得到錯誤
  await expect(docsErr).toBeInViewport()
  await p.waitForTimeout(600) // 等捲動結束再截圖
  await p.screenshot({ path: `${SHOTS}/apply-step3-error-viewport-390.png` })
  await expect(p.getByText('步驟 3/4')).toBeVisible()
  await shot(p, 'apply-step3-docs-required')

  // 選了許可檔但沒按新增
  await p.locator('input[type=file]').last().setInputFiles({ name: 'permit.pdf', mimeType: 'application/pdf', buffer: Buffer.from('%PDF-1.4\n%%EOF') })
  await p.getByRole('button', { name: '下一步' }).click()
  await expect(p.getByText('許可檔案還沒新增，請先按「新增許可」')).toBeInViewport()
  await shot(p, 'apply-step3-permit-pending')
  await ctx.close()
})

test('沒有組織的使用者：選單有通往 /org 的項目', async ({ browser }) => {
  const u = await registerUser({ tag: 'noorg' })
  const ctx = await browser.newContext()
  await loginBrowser(ctx, u)
  const p = await ctx.newPage()
  await p.goto('/dashboard')
  await p.getByRole('button', { name: new RegExp(u.name) }).click()
  const link = p.getByRole('link', { name: '還沒有組織？申請成為公益組織' })
  await expect(link).toBeVisible()
  await expect(link).toHaveAttribute('href', '/org')
  await shot(p, 'user-menu-open')
  await ctx.close()
})

test('主控台「接下來」與 QR 實收：據點建立後掃描預填、送出後捐助者看到已收到', async ({ browser }) => {
  const owner = await registerUser({ tag: 'qrowner' })
  const { orgId } = await seedOrg(owner)
  const ctx = await browser.newContext()
  await loginBrowser(ctx, owner)
  const p = await ctx.newPage()

  // 沒有事件與據點：三步都是連結
  await p.goto(`/orgs/${orgId}`)
  const next = p.locator('.oh-next')
  await expect(next).toBeVisible()
  await expect(next.locator('a.oh-go')).toHaveCount(3)
  await shot(p, 'console-next-steps', p.locator('section', { has: next }))

  // 建事件 + 據點 + 品項 + 發布（同 org-relief.spec 的流程）
  const evName = `E2E 地震 ${uid()}`
  sql(`INSERT INTO relief_events (slug, name, hazard_type, counties, occurred_on, created_by_org) VALUES (substr(md5(random()::text), 1, 10), '${evName}', 'earthquake', ARRAY['花蓮縣'], current_date, '${orgId}')`)
  await p.goto(`/orgs/${orgId}/events/new`)
  await p.locator('select[name=county]').selectOption('花蓮縣')
  await p.locator('input[name=occurred_on]').fill(new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Taipei' }))
  await p.getByRole('link', { name: '用這個事件建立據點' }).first().click()
  await p.locator('select[name=event_id]').selectOption({ index: 1 })
  await p.locator('select[name=permit_id]').selectOption({ index: 1 })
  await p.locator('input[name=title]').fill(`E2E 據點 ${uid()}`)
  await p.locator('input[name=site_address]').fill('花蓮縣花蓮市中山路 1 號')
  await p.locator('input[name=recipient_name]').fill('王小明')
  await p.locator('input[name=recipient_phone]').fill('0912345678')
  await p.getByRole('button', { name: '建立據點' }).click()
  await expect(p).toHaveURL(/\/lists\/[0-9a-f-]+\/edit$/)
  const wid = new URL(p.url()).pathname.split('/')[2]!
  // 事件與草稿據點都有了：前兩步打勾（沒有連結），只剩「讓據點上線」是連結
  await p.goto(`/orgs/${orgId}`)
  await expect(p.locator('.oh-next li.done')).toHaveCount(2)
  await expect(p.locator('.oh-next a.oh-go')).toHaveCount(1)
  await shot(p, 'console-next-steps-partial', p.locator('section', { has: p.locator('.oh-next') }))
  await p.goto(`/lists/${wid}/edit`)
  await p.getByRole('button', { name: /新增品項/ }).click()
  await p.getByLabel('名稱（必填）').fill('瓶裝水')
  await p.locator('select[name=category]').selectOption('water')
  await p.locator('select[name=urgency]').selectOption('critical')
  await p.getByRole('button', { name: '儲存品項' }).click()
  await expect(p.getByText('瓶裝水')).toBeVisible()
  await p.getByRole('button', { name: '發佈並分享' }).click()
  const slug = (await p.locator('#c-share-input').inputValue()).split('/s/')[1]!

  // 發布後三步完成：「接下來」區塊消失
  await p.goto(`/orgs/${orgId}`)
  await expect(p.getByRole('heading', { name: '據點' })).toBeVisible()
  await expect(p.locator('.oh-next')).toHaveCount(0)

  // 捐助者：/me/claims 顯示 QR
  const itemId = sql(`SELECT id FROM wishlist_items WHERE wishlist_id = '${wid}' AND title = '瓶裝水'`)
  const donor = await registerUser({ tag: 'qrdonor' })
  const { claim } = await memberClaim(itemId, 1, donor)
  const dctx = await browser.newContext()
  await loginBrowser(dctx, donor)
  const dp = await dctx.newPage()
  await dp.goto('/me/claims')
  await expect(dp.getByText(claim.id, { exact: true })).toBeVisible()
  await dp.getByText('顯示 QR').click()
  const qr = dp.locator('[data-testid=claim-qr] [role=img]')
  await expect(qr.locator('svg')).toBeVisible()
  await shot(dp, 'me-claims-qr', dp.locator('.g-item', { has: dp.locator('[data-testid=claim-qr]') }))

  // 志工掃 QR：先經 /receive，再到實收頁，對話框自動打開並預填
  const q = new URLSearchParams({ list: slug, claim: claim.id, item: itemId, qty: '1' })
  await p.goto(`/receive?${q}`)
  await expect(p).toHaveURL(new RegExp(`/orgs/${orgId}/sites/${wid}/receipts$`))
  const dlg = p.getByRole('dialog')
  await expect(dlg).toBeVisible()
  await expect(dlg.getByLabel('認領編號', { exact: false })).toHaveValue(claim.id)
  await expect(dlg.getByLabel('實收數量', { exact: false })).toHaveValue('1')
  await shot(p, 'receipt-dialog-prefilled')
  await dlg.getByRole('button', { name: '登記', exact: true }).click()
  await expect(p.getByRole('dialog')).toHaveCount(0)
  await dp.reload()
  await expect(dp.getByText('據點已確認收到物資')).toBeVisible()

  // 格式錯誤的 query：顯示錯誤、不靜默
  await p.goto(`/orgs/${orgId}/sites/${wid}/receipts?claim=abc&item=${itemId}&qty=1`)
  await expect(p.getByTestId('scan-err')).toBeVisible()
  await expect(p).not.toHaveURL(/claim=/)
  // 其他據點的品項（找不到）
  await p.goto(`/receive?${new URLSearchParams({ list: 'nosuchslug', claim: claim.id, item: itemId, qty: '1' })}`)
  await expect(p.getByRole('alert')).toContainText('找不到這個據點')
  await dctx.close(); await ctx.close()
})
