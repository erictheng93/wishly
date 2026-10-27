import { expect, test } from '@playwright/test'
import { loginBrowser, makeStaff, registerUser, sql, uid } from './helpers'

// staff 打開 pending 申請 → 核准許可 → 核准組織 → 申請人的狀態頁顯示已通過
test('後台：審核組織申請（核准許可後核准組織）', async ({ browser }) => {
  const staff = await registerUser({ tag: 'orgstaff' })
  makeStaff(staff.email)
  const owner = await registerUser({ tag: 'orgapplicant' })
  const ownerId = (await (await owner.api.get('me')).json()).id as string
  const name = `E2E 待審組織 ${uid()}`
  const orgId = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, submitted_at, created_by)
    VALUES (substr(md5(random()::text), 1, 10), '${name}', '${name}', 'association', lpad((floor(random()*1e8))::text, 8, '0'), 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'pending', now(), '${ownerId}') RETURNING id`).split('\n')[0]
  sql(`INSERT INTO org_members (org_id, user_id, role) VALUES ('${orgId}', '${ownerId}', 'owner')`)
  const permitNo = `P-${uid()}`
  sql(`INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to) VALUES ('${orgId}', '${permitNo}', current_date - 1, current_date + 90)`)

  const ctx = await browser.newContext()
  await loginBrowser(ctx, staff)
  const p = await ctx.newPage()
  await p.goto('/admin/orgs')
  const card = p.locator(`article[data-org="${orgId}"]`)
  await card.getByRole('button', { name: '檢視' }).click()

  // 許可未核准前核准組織會被後端擋下（需至少一張已核准許可），所以先核准許可
  await card.locator('[data-permit]').getByRole('button', { name: '核准許可' }).click()
  await p.getByRole('dialog').getByRole('button', { name: '核准許可' }).click()
  await expect(card.locator('[data-permit]').getByText('已核准')).toBeVisible()

  await card.getByRole('button', { name: '核准組織', exact: true }).click()
  await p.getByRole('dialog').getByRole('button', { name: '核准組織', exact: true }).click()
  await expect(p.getByRole('status').filter({ hasText: '已處理' })).toBeVisible()

  const octx = await browser.newContext()
  await loginBrowser(octx, owner)
  const op = await octx.newPage()
  await op.goto('/org/apply')
  await expect(op.getByText('已通過審核')).toBeVisible()
  await ctx.close(); await octx.close()
})
