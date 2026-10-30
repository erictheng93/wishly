import { expect, test } from '@playwright/test'
import { registerUser, newEmail, sql, uid, waitForOtp } from './helpers'

test('註冊入口保留第三方登入與導向', async ({ page }) => {
  await page.goto('/register?redirect=/lists/new')
  await expect(page.getByRole('button', { name: '使用 Google 繼續' })).toBeVisible()
  await expect(page.getByRole('button', { name: '使用 LINE 繼續' })).toBeVisible()
  await page.screenshot({ path: '/tmp/wishly-register-desktop.png', fullPage: true })
  await page.setViewportSize({ width: 390, height: 844 })
  await page.screenshot({ path: '/tmp/wishly-register-mobile.png', fullPage: true })
  let startUrl = ''
  await page.route('**/auth/oauth/google/start?**', route => { startUrl = route.request().url(); return route.abort() })
  await page.getByRole('button', { name: '使用 Google 繼續' }).click()
  await expect.poll(() => startUrl).toContain('redirect=%2Flists%2Fnew')
})

test('LINE 設定：補 Email、OTP 驗證、解除綁定及最後登入方式保護', async ({ page, context }) => {
  page.on('pageerror', error => { throw error })
  const user = await registerUser()
  const id = (await (await user.api.get('me')).json()).id as string
  sql(`DELETE FROM auth_identities WHERE user_id='${id}'`)
  sql(`UPDATE users SET email=NULL WHERE id='${id}'`)
  sql(`INSERT INTO auth_identities (user_id,provider,provider_uid) VALUES ('${id}','line','${uid()}')`)
  const cookies = (await user.api.storageState()).cookies
  await context.addCookies(cookies)
  await page.goto('/settings')
  const logins = page.getByRole('region', { name: '登入方式' })
  await expect(logins.getByRole('button', { name: '解除 LINE 綁定' })).toBeDisabled()
  await expect(page.getByText('尚未綁定 Email，無法收取通知信或使用密碼重設。')).toBeVisible()
  await page.screenshot({ path: '/tmp/wishly-settings-desktop.png', fullPage: true })
  await page.setViewportSize({ width: 390, height: 844 })
  await expect.poll(() => page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
  await page.screenshot({ path: '/tmp/wishly-settings-mobile.png', fullPage: true })
  const email = newEmail('link')
  await logins.getByLabel('綁定 Email').fill(email)
  await logins.getByRole('button', { name: '寄送綁定驗證碼' }).click()
  await logins.getByLabel('第 1 碼').fill(await waitForOtp(email, /綁定/))
  await expect(logins.getByRole('status')).toHaveText('已綁定 Email，可收取通知並設定密碼')
  await logins.getByRole('button', { name: '解除 Email 綁定' }).click()
  const dialog = page.getByRole('dialog', { name: '解除 Email 綁定？' })
  await dialog.getByRole('button', { name: '解除綁定', exact: true }).click()
  await expect(logins.getByRole('button', { name: '解除 LINE 綁定' })).toBeDisabled()
  await user.api.dispose()
})

for (const [error, message] of [
  ['account_blocked', '帳號已停權，請聯絡客服'],
  ['identity_in_use', '此登入方式或 Email 已屬於另一個帳號，請使用原帳號登入'],
]) {
  test(`OAuth 登入錯誤顯示原因：${error}`, async ({ page }) => {
    await page.goto(`/login?error=${error}`)
    await expect(page.getByRole('alert')).toContainText(message!)
    if (error === 'account_blocked') await page.screenshot({ path: '/tmp/wishly-account-blocked.png', fullPage: true })
  })
}

for (const path of ['/settings', '/admin']) {
  test(`首次載入 ${path} 發現停權仍保留原因`, async ({ page, context }) => {
    const user = await registerUser()
    const id = (await (await user.api.get('me')).json()).id as string
    await context.addCookies((await user.api.storageState()).cookies)
    sql(`UPDATE users SET blocked_at=now() WHERE id='${id}'`)
    await page.goto(path)
    await expect(page).toHaveURL(/\/login\?.*error=account_blocked/)
    await expect(page.getByRole('alert')).toContainText('帳號已停權，請聯絡客服')
    await user.api.dispose()
  })
}

test('已登入帳號停權後 API 請求清除登入並顯示原因', async ({ page, context }) => {
  const user = await registerUser()
  const id = (await (await user.api.get('me')).json()).id as string
  await context.addCookies((await user.api.storageState()).cookies)
  await page.goto('/settings')
  await expect(page.getByLabel('顯示名稱')).toBeVisible()
  // 頁首的新手導覽載入後也會打 API；等它先完成再停權，否則它可能搶先撞到停權並導走，「儲存」就點不到
  await page.waitForLoadState('networkidle')
  sql(`UPDATE users SET blocked_at=now() WHERE id='${id}'`)
  await page.locator('form').filter({ has: page.getByLabel('顯示名稱') }).getByRole('button', { name: '儲存', exact: true }).click()
  await expect(page).toHaveURL(/\/login\?.*error=account_blocked/)
  await expect(page.getByRole('alert')).toContainText('帳號已停權，請聯絡客服')
  await user.api.dispose()
})
