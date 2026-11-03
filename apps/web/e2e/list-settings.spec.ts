import { expect, test } from '@playwright/test'
import { createList, signInPage } from './helpers'
import { cors } from './support/mockApi'

// 1×1 PNG
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64')

test('清單設定：有變更才出現底部儲存列、可還原；封面選了就存', async ({ page }) => {
  const u = await signInPage(page, { tag: 'settings' })
  const list = await createList(u)
  await page.goto(`/lists/${list.id}/edit`)
  await page.getByText('清單設定', { exact: true }).click()
  const bar = page.getByText('有未儲存的變更')
  const name = page.getByLabel('清單名稱')
  await expect(bar).toBeHidden()

  await test.step('改名 → 出現儲存列 → 還原', async () => {
    await name.fill('改過的名字')
    await expect(bar).toBeVisible()
    await page.getByRole('button', { name: '還原' }).click()
    await expect(name).toHaveValue(list.title)
    await expect(bar).toBeHidden()
  })

  await test.step('改名 → 用底部「儲存」送出 → 重新整理仍在', async () => {
    await name.fill('改過的名字')
    await Promise.all([page.waitForResponse(r => r.request().method() === 'PATCH' && r.ok()), page.getByRole('button', { name: '儲存', exact: true }).click()])
    await expect(bar).toBeHidden()
    await page.reload()
    await page.getByText('清單設定', { exact: true }).click()
    await expect(page.getByLabel('清單名稱')).toHaveValue('改過的名字')
  })

  await test.step('點預設封面 → 不按儲存就生效', async () => {
    const preset = page.locator('.cp-opt input').nth(1)
    const [r] = await Promise.all([page.waitForResponse(r => r.request().method() === 'PATCH' && r.ok()), preset.check({ force: true })])
    expect(r.request().postDataJSON()).toMatchObject({ cover_image_key: null })
    await expect(page.getByText('封面已更新')).toBeVisible()
    await expect(bar).toBeHidden()
    await page.reload()
    await page.getByText('清單設定', { exact: true }).click()
    await expect(page.locator('.cp-opt input').nth(1)).toBeChecked()
  })

  await test.step('上傳封面 → 不按儲存就生效', async () => {
    // CI 的 e2e 沒有物件儲存（MinIO）：攔下直傳 PUT 與 confirm，presign 與 PATCH 仍走真後端（PATCH 只驗 key 格式）
    await page.route(u => true, r => r.request().method() === 'PUT' ? r.fulfill({ status: 200, headers: cors }) : r.fallback())
    await page.route('**/uploads/confirm', r => r.fulfill({ status: 200, contentType: 'application/json', body: '{}', headers: cors }))
    const [r] = await Promise.all([
      page.waitForResponse(r => r.request().method() === 'PATCH' && r.ok(), { timeout: 20_000 }),
      page.locator('input[type=file]').setInputFiles({ name: 'c.png', mimeType: 'image/png', buffer: PNG }),
    ])
    expect(r.request().postDataJSON()).toMatchObject({ cover_preset: null })
    expect(r.request().postDataJSON().cover_image_key).toBeTruthy()
    await expect(bar).toBeHidden()
  })
})
