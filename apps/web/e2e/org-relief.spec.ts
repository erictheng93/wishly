import { expect, test } from '@playwright/test'
import { loginBrowser, registerUser, sql, uid } from './helpers'

// 已審核組織 + 有效許可 + 既有事件 → 查重命中選既有事件 → 建據點 → 加 critical 品項 → 發布 → 公開頁可見
// （後端救災 API 合併後才跑得過）
test('救災：建立據點、加急缺品項、發布後公開頁可見', async ({ browser }) => {
  const owner = await registerUser({ tag: 'reliefowner' })
  const ownerId = (await (await owner.api.get('me')).json()).id as string
  const orgName = `E2E 救災組織 ${uid()}`
  const orgId = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
    VALUES (substr(md5(random()::text), 1, 10), '${orgName}', '${orgName}', 'association', lpad((floor(random()*1e8))::text, 8, '0'), 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${ownerId}') RETURNING id`).split('\n')[0]
  sql(`INSERT INTO org_members (org_id, user_id, role) VALUES ('${orgId}', '${ownerId}', 'owner')`)
  sql(`INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to, status) VALUES ('${orgId}', 'P-${uid()}', current_date - 1, current_date + 90, 'approved')`)
  const evName = `E2E 地震 ${uid()}`
  sql(`INSERT INTO relief_events (slug, name, hazard_type, counties, occurred_on, created_by_org) VALUES (substr(md5(random()::text), 1, 10), '${evName}', 'earthquake', ARRAY['花蓮縣'], current_date, '${orgId}')`)

  const ctx = await browser.newContext()
  await loginBrowser(ctx, owner)
  const p = await ctx.newPage()
  await p.goto(`/orgs/${orgId}/events/new`)
  await p.locator('select[name=county]').selectOption('花蓮縣')
  await p.locator('input[name=occurred_on]').fill(new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Taipei' }))
  await expect(p.getByText(evName)).toBeVisible()
  await p.getByRole('link', { name: '用這個事件建立據點' }).first().click()

  const title = `E2E 據點 ${uid()}`
  await p.locator('select[name=permit_id]').selectOption({ index: 1 })
  await p.locator('input[name=title]').fill(title)
  await p.locator('input[name=site_address]').fill('花蓮縣花蓮市中山路 1 號')
  await p.locator('input[name=recipient_name]').fill('王小明')
  await p.locator('input[name=recipient_phone]').fill('0912345678')
  await p.getByRole('button', { name: '建立據點' }).click()
  await expect(p).toHaveURL(/\/lists\/[0-9a-f-]+$/)

  await p.getByRole('button', { name: /新增品項/ }).click()
  await p.getByLabel('名稱（必填）').fill('瓶裝水')
  await p.locator('select[name=category]').selectOption('water')
  await p.locator('select[name=urgency]').selectOption('critical')
  await p.getByRole('button', { name: '儲存品項' }).click()
  await expect(p.getByText('瓶裝水')).toBeVisible()
  await p.getByRole('button', { name: '發佈並分享' }).click()

  const slug = (await p.locator('#c-share-input').inputValue()).split('/s/')[1]
  const pub = await ctx.newPage()
  await pub.goto(`/s/${slug}`)
  await expect(pub.getByText(title)).toBeVisible()
  await expect(pub.getByText('瓶裝水')).toBeVisible()
  await ctx.close()
})
