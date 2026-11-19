// 品項表單：錯誤訊息一定看得到、去填收件資訊再回來草稿還在、連續新增。真實 API。
import { expect, test, type Page } from '@playwright/test'
import { loginBrowser, registerUser, uid, type User } from './helpers'
import { hydrated } from './support/hydrate'

const ADDR = { recipient_name: '王小米', phone: '0912345678', address: '台北市大安區和平東路二段 1 號 3 樓' }
const dateIn = (d: number) => new Date(Date.now() + d * 864e5 + 8 * 36e5).toISOString().slice(0, 10)
const localIn = (d: number) => new Date(Date.now() + d * 864e5 + 8 * 36e5).toISOString().slice(0, 16) // timezoneId = Asia/Taipei

async function draftList(u: User, extra: Record<string, unknown> = {}) {
  const r = await u.api.post('wishlists', { data: { type: 'personal', title: `品項表單 ${uid()}`, ...extra } })
  expect(r.ok(), await r.text()).toBe(true)
  return (await r.json()).id as string
}
async function openNew(page: Page, u: User, id: string) {
  await loginBrowser(page.context(), u)
  await page.goto(`/lists/${id}/edit`); await hydrated(page)
  await page.getByRole('button', { name: '新增品項' }).click()
  return page.getByRole('dialog', { name: '新增品項' })
}

test.describe('手機 390×659', () => {
  test.use({ viewport: { width: 390, height: 659 } })
  test('眾籌品項儲存失敗：錯誤訊息完整落在視窗內、不被黏底按鈕列蓋住', async ({ page }) => {
    const u = await registerUser({ tag: 'isheet' })
    const id = await draftList(u, { event_date: dateIn(30) }) // 有活動日：截止時間選填；沒有收件資訊 → 409
    const sheet = await openNew(page, u, id)
    await sheet.getByLabel('名稱（必填）').fill('嬰兒推車')
    await sheet.getByRole('radio', { name: '心願眾籌' }).check()
    await expect(sheet.getByLabel(/募集截止/)).not.toHaveAttribute('required', /.*/)
    await expect(sheet.getByText('眾籌品項需要先填寫收件資訊').first()).toBeVisible() // 前置提示
    await sheet.getByLabel(/目標點數/).fill('1000')
    await sheet.getByRole('button', { name: '儲存品項' }).click()
    const alert = sheet.getByRole('alert')
    await expect(alert).toContainText('收件資訊')
    await expect(alert).toBeInViewport({ ratio: 1 })
    const a = (await alert.boundingBox())!, bar = (await sheet.getByRole('button', { name: '儲存品項' }).boundingBox())!
    expect(a.y + a.height).toBeLessThanOrEqual(bar.y) // 在按鈕列上方，沒被蓋住
    await u.api.dispose()
  })

  test('眾籌品項沒填截止時間：必填欄位被捲進視窗後完整可見、在黏底按鈕列上方', async ({ page }) => {
    const u = await registerUser({ tag: 'isheet-req' })
    const id = await draftList(u) // 沒有活動日：眾籌的截止時間必填
    const sheet = await openNew(page, u, id)
    await sheet.getByLabel('名稱（必填）').fill('嬰兒推車')
    await sheet.getByRole('radio', { name: '心願眾籌' }).check()
    await sheet.getByLabel(/目標點數/).fill('1000')
    await sheet.getByRole('button', { name: '儲存品項' }).click()
    const closes = sheet.getByLabel(/募集截止/)
    await expect(closes).toBeInViewport({ ratio: 1 })
    const c = (await closes.boundingBox())!, bar = (await sheet.getByRole('button', { name: '儲存品項' }).boundingBox())!
    expect(c.y + c.height).toBeLessThanOrEqual(bar.y)
    await u.api.dispose()
  })
})

test.describe('手機 390×844 建立頁', () => {
  test.use({ viewport: { width: 390, height: 844 } })
  test('只填名稱就能建立；靈感頁帶來的預填轉到編輯頁並打開新增品項', async ({ page }) => {
    const u = await registerUser({ tag: 'isnew' })
    await loginBrowser(page.context(), u)
    await page.goto('/lists/new?add=1&title=' + encodeURIComponent('靈感奶瓶') + '&brand=Pigeon'); await hydrated(page)
    const submit = page.getByRole('button', { name: '建立並新增品項' })
    await expect(submit).toBeInViewport() // 不必捲動就看得到送出鈕
    await page.getByLabel(/清單名稱/).fill('只填名稱')
    await submit.click()
    await page.waitForURL(/\/lists\/[0-9a-f-]+\/edit/)
    const id = page.url().match(/lists\/([0-9a-f-]+)\/edit/)![1]
    const sheet = page.getByRole('dialog', { name: '新增品項' })
    await expect(sheet.getByLabel('名稱（必填）')).toHaveValue('靈感奶瓶')
    await expect(sheet.getByLabel('品牌（選填）')).toHaveValue('Pigeon') // 有預填值，「更多選項」自動展開
    const w = (await (await u.api.get(`wishlists/${id}`)).json()).wishlist
    expect([w.visibility, w.claim_ttl_hours, w.cover_preset]).toEqual(['friends', 168, 'gift'])
    await u.api.dispose()
  })
})

test('去填收件資訊再回來：草稿還在，存好後能成功儲存', async ({ page }) => {
  const u = await registerUser({ tag: 'isheet' })
  const id = await draftList(u) // 沒有活動日：截止時間必填
  const sheet = await openNew(page, u, id)
  await sheet.getByLabel('名稱（必填）').fill('嬰兒推車')
  await sheet.getByRole('radio', { name: '心願眾籌' }).check()
  const deadline = sheet.getByLabel('募集截止（台北時間，必填）')
  await expect(deadline).toHaveAttribute('required', '')
  await sheet.getByLabel(/目標點數/).fill('3000')
  await deadline.fill(localIn(20))
  await sheet.getByText('更多選項').click()
  await sheet.getByLabel('品牌（選填）').fill('Combi')
  await sheet.getByRole('button', { name: '填寫收件資訊' }).click()
  await expect(sheet).toBeHidden()

  await page.getByLabel(/收件人/).fill(ADDR.recipient_name)
  await page.getByLabel(/電話/).fill(ADDR.phone)
  await page.getByLabel(/地址/).fill(ADDR.address)
  await page.getByRole('button', { name: '儲存收件資訊' }).click()

  await expect(sheet).toBeVisible() // 自動回到品項表單
  await expect(sheet.getByLabel('名稱（必填）')).toHaveValue('嬰兒推車')
  await expect(sheet.getByRole('radio', { name: '心願眾籌' })).toBeChecked()
  await expect(sheet.getByLabel(/目標點數/)).toHaveValue('3000')
  await expect(sheet.getByLabel(/募集截止/)).toHaveValue(localIn(20))
  await expect(sheet.getByLabel('品牌（選填）')).toHaveValue('Combi')
  await expect(sheet.getByRole('button', { name: '填寫收件資訊' })).toHaveCount(0) // 已有收件資訊，不再提示
  await sheet.getByRole('button', { name: '儲存品項' }).click()
  await expect(sheet).toBeHidden()
  await expect(page.getByRole('article').filter({ hasText: '嬰兒推車' })).toContainText('心願眾籌')

  // 取消會清掉草稿
  await page.getByRole('button', { name: '新增品項' }).click()
  await sheet.getByLabel('名稱（必填）').fill('暫時的')
  await sheet.getByRole('button', { name: '取消' }).click()
  await page.getByRole('button', { name: '新增品項' }).click()
  await expect(sheet.getByLabel('名稱（必填）')).toHaveValue('')
  await u.api.dispose()
})

test('連續新增：儲存後表單清空、不關閉、焦點回名稱', async ({ page }) => {
  const u = await registerUser({ tag: 'isheet' })
  const id = await draftList(u)
  const sheet = await openNew(page, u, id)
  await sheet.getByLabel('儲存後繼續新增下一個').check()
  await sheet.getByLabel('名稱（必填）').fill('奶瓶')
  await sheet.getByRole('button', { name: '儲存品項' }).click()
  await expect(page.getByRole('article').filter({ hasText: '奶瓶' })).toBeVisible()
  await expect(sheet).toBeVisible()
  await expect(sheet.getByLabel('名稱（必填）')).toHaveValue('')
  await expect(sheet.getByLabel('名稱（必填）')).toBeFocused()
  await expect(sheet.getByLabel('儲存後繼續新增下一個')).toBeChecked()
  await sheet.getByLabel('名稱（必填）').fill('口水巾')
  await sheet.getByRole('button', { name: '儲存品項' }).click()
  await expect(page.getByRole('article')).toHaveCount(2)
  await u.api.dispose()
})
