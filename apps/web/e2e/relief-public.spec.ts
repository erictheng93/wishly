import { expect, test } from '@playwright/test'
import { registerUser, sql, uid } from './helpers'

// SQL 種子：已審核組織 + 許可 + 事件（含一個合併到它的舊事件）+ 上線據點與 critical 品項；訪客不登入即可瀏覽
test('救災公開頁：列表、事件缺口、舊 slug 轉址、分享頁組織資訊', async ({ page }) => {
  const owner = await registerUser({ tag: 'reliefpub' })
  const ownerId = (await (await owner.api.get('me')).json()).id as string
  const t = uid()
  const orgName = `E2E 公開組織 ${t}`, evName = `E2E 公開地震 ${t}`, title = `E2E 公開據點 ${t}`
  // slug 欄位為 char(10)、須正好 10 個英數字：1 字前綴 + 9 字隨機（uid 為 10 hex，取前 9）
  const sg = (p: string) => p + uid().slice(0, 9)
  const orgSlug = sg('o'), evSlug = sg('q'), oldSlug = sg('m'), wlSlug = sg('w')
  const orgId = sql(`INSERT INTO organizations (slug, legal_name, display_name, org_type, tax_id, registration_no, authority, rep_name, contact_name, contact_phone_enc, public_email, verification_status, verified_at, created_by)
    VALUES ('${orgSlug}', '${orgName}', '${orgName}', 'association', lpad((floor(random()*1e8))::text, 8, '0'), 'R-1', '內政部', '王', '王', '\\x00', 'o@example.com', 'verified', now(), '${ownerId}') RETURNING id`).split('\n')[0]
  const permitId = sql(`INSERT INTO org_permits (org_id, permit_no, valid_from, valid_to, status) VALUES ('${orgId}', 'P-${t}', current_date - 1, current_date + 90, 'approved') RETURNING id`).split('\n')[0]
  const evId = sql(`INSERT INTO relief_events (slug, name, hazard_type, counties, occurred_on, created_by_org) VALUES ('${evSlug}', '${evName}', 'earthquake', ARRAY['花蓮縣'], current_date, '${orgId}') RETURNING id`).split('\n')[0]
  sql(`INSERT INTO relief_events (slug, name, hazard_type, counties, created_by_org, merged_into) VALUES ('${oldSlug}', '舊事件 ${t}', 'earthquake', ARRAY['花蓮縣'], '${orgId}', '${evId}')`)
  const wlId = sql(`INSERT INTO wishlists (owner_id, type, status, visibility, slug, title, org_id, event_id, permit_id, site_address, site_status)
    VALUES ('${ownerId}', 'relief', 'active', 'public', '${wlSlug}', '${title}', '${orgId}', '${evId}', '${permitId}', '花蓮縣花蓮市中山路 1 號', 'active') RETURNING id`).split('\n')[0]
  sql(`INSERT INTO wishlist_items (wishlist_id, title, qty_needed, category, urgency) VALUES ('${wlId}', '瓶裝水', 10, 'water', 'critical')`)

  await page.goto('/e')
  await expect(page.getByRole('link', { name: evName })).toBeVisible()

  await page.goto(`/e/${evSlug}`)
  await expect(page.getByRole('link', { name: title })).toBeVisible()
  await expect(page.getByText('急缺 1')).toBeVisible()

  await page.goto(`/e/${oldSlug}`)
  await expect(page).toHaveURL(new RegExp(`/e/${evSlug}$`))

  await page.goto(`/s/${wlSlug}`)
  await expect(page.getByRole('link', { name: orgName })).toHaveAttribute('href', `/o/${orgSlug}`)
  await expect(page.getByText('已驗證組織')).toBeVisible()
  await expect(page.getByRole('link', { name: evName })).toHaveAttribute('href', `/e/${evSlug}`)

  await page.goto(`/o/${orgSlug}`)
  await expect(page.getByRole('link', { name: title })).toBeVisible()
})
