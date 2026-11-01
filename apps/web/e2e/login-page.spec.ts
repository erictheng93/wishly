import { expect, test } from '@playwright/test'
import { newEmail, registerUser, resetOtpLimits, sql, uid, waitForOtp } from './helpers'
import { API, WEB } from './support/env'

test('LINE 帳號綁定 Email 後沒有密碼：密碼失敗有引導，改用驗證碼回到同一帳號', async ({ page }) => {
  const user = await registerUser()
  const id = (await (await user.api.get('me')).json()).id as string
  sql(`UPDATE auth_identities SET password_hash=NULL WHERE user_id='${id}'`)
  sql(`INSERT INTO auth_identities (user_id,provider,provider_uid) VALUES ('${id}','line','${uid()}')`)
  resetOtpLimits()

  await page.goto('/login')
  await page.getByLabel('Email').fill(user.email)
  await page.getByLabel('密碼').fill('not-a-password')
  await page.getByRole('button', { name: '登入', exact: true }).click()
  await expect(page.getByText('可以改用 Email 驗證碼登入')).toBeVisible()

  await page.getByRole('button', { name: '用 Email 驗證碼登入' }).click()
  await expect(page.getByLabel('Email')).toHaveValue(user.email)
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await page.getByLabel('第 1 碼').fill(await waitForOtp(user.email, /登入/))
  await expect(page).toHaveURL(/\/dashboard/)
  const me = await (await page.context().request.get(`${API}/api/v1/me`)).json()
  expect(me.id).toBe(id)
  await user.api.dispose()
})

test('驗證碼登入可切回密碼；錯誤驗證碼留在同一步', async ({ page }) => {
  const email = newEmail('code')
  resetOtpLimits()
  await page.goto('/login')
  await page.getByRole('button', { name: '用 Email 驗證碼登入' }).click()
  await page.getByLabel('Email').fill(email)
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  const code = await waitForOtp(email, /登入/)
  await page.getByLabel('第 1 碼').fill(code === '000000' ? '111111' : '000000')
  await expect(page.getByRole('alert')).toContainText('驗證碼不正確或已過期')
  await page.getByRole('button', { name: '改用密碼登入' }).click()
  await expect(page.getByLabel('密碼')).toBeVisible()
})

test('驗證碼登入保留 redirect：從分享頁來的人回到原頁', async ({ page }) => {
  const user = await registerUser()
  resetOtpLimits()
  await page.goto('/login?redirect=/lists/new')
  await page.getByRole('button', { name: '用 Email 驗證碼登入' }).click()
  await page.getByLabel('Email').fill(user.email)
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await page.getByLabel('第 1 碼').fill(await waitForOtp(user.email, /登入/))
  await expect(page).toHaveURL(/\/lists\/new/)
  await user.api.dispose()
})

test('新信箱用驗證碼開戶：進導覽頁、名稱預填信箱前綴，跳過後儀表板提示改名', async ({ page }) => {
  const email = newEmail('new')
  const prefix = email.split('@')[0]!
  resetOtpLimits()
  await page.goto('/login')
  await page.getByRole('button', { name: '用 Email 驗證碼登入' }).click()
  await expect(page.getByText('會自動建立一個')).toBeVisible()
  await page.getByLabel('Email').fill(email)
  await page.getByRole('button', { name: '寄送驗證碼' }).click()
  await page.getByLabel('第 1 碼').fill(await waitForOtp(email, /登入/))
  await expect(page).toHaveURL(/\/welcome/)
  await expect(page.getByLabel('顯示名稱')).toHaveValue(prefix)
  await expect(page.getByText('這是朋友會看到的名稱。')).toBeVisible()

  const r = await page.context().request.patch(`${API}/api/v1/me/onboarding`, { data: { done: true }, headers: { Origin: WEB } })
  expect(r.ok()).toBe(true)
  await page.goto('/dashboard')
  await expect(page.getByRole('region', { name: '設定你的顯示名稱' })).toBeVisible()
})

test('登入頁有四種方式；Google / LINE 未設憑證 → oauth_unavailable', async ({ page }) => {
  await page.goto('/login')
  await expect(page.getByRole('button', { name: '登入', exact: true })).toBeVisible() // Email + 密碼
  await expect(page.getByLabel('Email')).toBeVisible()
  await expect(page.getByRole('button', { name: '用 Email 驗證碼登入' })).toBeVisible()
  await expect(page.getByRole('button', { name: '使用 Google 繼續' })).toBeVisible()
  await expect(page.getByRole('button', { name: '使用 LINE 繼續' })).toBeVisible()

  for (const [btn, provider] of [['使用 Google 繼續', 'google'], ['使用 LINE 繼續', 'line']]) {
    await test.step(`${provider}：沒有憑證時導回登入頁並說明`, async () => {
      await page.goto('/login')
      await page.getByRole('button', { name: btn }).click()
      await expect(page).toHaveURL(/\/login\?error=oauth_unavailable/)
      await expect(page.getByRole('alert')).toContainText('此登入方式尚未開放')
    })
  }

  await test.step('oauth_failed 顯示失敗訊息', async () => {
    await page.goto('/login?error=oauth_failed')
    await expect(page.getByRole('alert')).toContainText('第三方登入失敗，請重試')
  })
})
