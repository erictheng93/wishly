import { expect, test } from '@playwright/test'
import { loginBrowser, registerUser, sql, uid } from './helpers'

// owner 邀請 → 受邀者註冊後開連結接受 → 選單出現該組織（後端成員 API 合併後才跑得過）
test('組織成員：owner 邀請，受邀者接受後選單出現組織', async ({ browser }) => {
  const owner = await registerUser({ tag: 'orgowner' })
  const ownerId = (await (await owner.api.get('me')).json()).id as string
  const orgName = `E2E 組織 ${uid()}`
  // 直接以 SQL 種一個已審核組織與其 owner（跳過申請流程）
  const orgId = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
    VALUES (substr(md5(random()::text), 1, 10), '${orgName}', '${orgName}', 'association', lpad((floor(random()*1e8))::text, 8, '0'), 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${ownerId}') RETURNING id`).split('\n')[0]
  sql(`INSERT INTO org_members (org_id, user_id, role) VALUES ('${orgId}', '${ownerId}', 'owner')`)

  const guest = await registerUser({ tag: 'orgguest' })
  const ownerCtx = await browser.newContext()
  await loginBrowser(ownerCtx, owner)
  const op = await ownerCtx.newPage()
  await op.goto(`/orgs/${orgId}/members`)
  await op.getByLabel('Email').fill(guest.email)
  await op.getByRole('button', { name: '寄出邀請' }).click()
  const link = await op.getByLabel('邀請連結').inputValue()
  expect(link).toContain('/org/invite/')

  const guestCtx = await browser.newContext()
  await loginBrowser(guestCtx, guest)
  const gp = await guestCtx.newPage()
  await gp.goto(new URL(link).pathname)
  await gp.getByRole('button', { name: '接受邀請' }).click()
  await expect(gp).toHaveURL(new RegExp(`/orgs/${orgId}$`))
  await gp.getByRole('button', { name: new RegExp(guest.name) }).click()
  await expect(gp.getByRole('button', { name: orgName })).toBeVisible()
  await ownerCtx.close(); await guestCtx.close()
})
