// 流程驗證 F22（docs/02 個人頁與捐助可見度）：真實 API，補 social.spec 未涵蓋的情境。
import { expect, test } from '@playwright/test'
import { loginBrowser, registerUser, uid } from './helpers'
import { hydrated } from './support/hydrate'

test('個人頁：回應帶 noindex（client 渲染的空殼也要擋索引）；不存在的代號顯示找不到', async ({ page }) => {
  const res = await page.goto(`/u/nobody_${uid()}`)
  expect(res?.headers()['x-robots-tag']).toBe('noindex')
  await hydrated(page)
  await expect(page.getByText('找不到這位使用者。')).toBeVisible()
})

test('設定帳號代號：已被使用時顯示說明，換一個即可儲存並出現個人頁連結', async ({ page }) => {
  const taken = `t${uid()}`
  const other = await registerUser({ tag: 'other' })
  expect((await other.api.patch('me', { data: { handle: taken } })).ok()).toBe(true)
  const me = await registerUser({ tag: 'me' })
  await loginBrowser(page.context(), me)

  await page.goto('/settings'); await hydrated(page)
  const input = page.getByLabel('帳號代號（個人頁網址，可留空）')
  const form = page.locator('form', { has: input })
  await input.fill(taken)
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('這個帳號代號已被使用，換一個試試')).toBeVisible()

  const mine = `m${uid()}`
  await input.fill(mine)
  await form.getByRole('button', { name: '儲存' }).click()
  await expect(form.getByText('已儲存')).toBeVisible()
  await expect(form.getByRole('link', { name: new RegExp(`/u/${mine}$`) })).toBeVisible()
})
