import { expect, test } from '@playwright/test'
import { createList, PASSWORD, registerUser } from './helpers'
import { API } from './support/env'
import { hydrated } from './support/hydrate'

// 認捐需登入：未登入只能瀏覽，認捐鈕是「登入後認捐」，登入後回到同一份清單才能認捐
test('未登入：看到「登入後認捐」與說明，從頭到尾沒有認捐表單；登入後回到同一個 /s/slug 並可認捐', async ({ page }) => {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '奶瓶組', qty: 2 }] })
  const member = await registerUser({ tag: 'lc' })
  const card = page.locator('li.g-card', { hasText: '奶瓶組' })

  await test.step('未登入：只有登入連結，沒有「我要送」、沒有認捐表單', async () => {
    await page.goto(`/s/${list.slug}`); await hydrated(page)
    await expect(page.getByText('認捐需要登入會員')).toBeVisible()
    await expect(card.getByRole('link', { name: '登入後認捐' })).toBeVisible()
    await expect(page.getByRole('button', { name: '我要送' })).toHaveCount(0)
    await expect(page.getByRole('dialog')).toHaveCount(0)
    await expect(page.getByLabel(/暱稱|聯絡方式/)).toHaveCount(0)
  })

  await test.step('點「登入後認捐」→ 登入頁帶 redirect；登入後回到同一份清單', async () => {
    await card.getByRole('link', { name: '登入後認捐' }).click()
    await expect(page).toHaveURL(new RegExp(`/login\\?redirect=(%2F|/)s(%2F|/)${list.slug}$`))
    await page.getByLabel('Email').fill(member.email)
    await page.getByLabel('密碼').fill(PASSWORD)
    await page.getByRole('button', { name: '登入', exact: true }).click()
    await expect(page).toHaveURL(new RegExp(`/s/${list.slug}$`))
    await expect(page.getByRole('heading', { level: 1, name: list.title })).toBeVisible()
  })

  await test.step('已登入：按鈕變「我要送」，表單沒有暱稱欄位，可認捐', async () => {
    await expect(card.getByRole('link', { name: '登入後認捐' })).toHaveCount(0)
    await card.getByRole('button', { name: '我要送' }).click()
    const dialog = page.getByRole('dialog')
    await expect(dialog.getByText(`以 ${member.name} 的身分捐助`)).toBeVisible()
    await expect(dialog.getByLabel(/暱稱|聯絡方式|Email/)).toHaveCount(0)
    await dialog.getByRole('button', { name: '確認捐助' }).click()
    await expect(page.getByRole('heading', { name: '✓ 捐助成功！' })).toBeVisible()
  })
  await Promise.all([owner.api.dispose(), member.api.dispose()])
})

test('未登入直接呼叫認捐 API 回 401 UNAUTHORIZED', async ({ request }) => {
  const owner = await registerUser()
  const list = await createList(owner)
  const r = await request.post(`${API}/api/v1/items/${list.items[0].id}/claims`, { data: { qty: 1 }, headers: { 'Idempotency-Key': crypto.randomUUID() } })
  expect(r.status()).toBe(401)
  expect((await r.json()).code).toBe('UNAUTHORIZED')
  await owner.api.dispose()
})
