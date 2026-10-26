import { expect, test } from '@playwright/test'
import { createList, registerUser, signInPage } from './helpers'

const LINE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari Line/14.5.0'

test.describe('LINE 內建瀏覽器', () => {
  test.use({ userAgent: LINE_UA })

  test('顯示「用預設瀏覽器開啟」引導橫幅，關閉後記住', async ({ page }) => {
    const owner = await registerUser()
    const list = await createList(owner)
    await page.goto(`/s/${list.slug}`)
    const banner = page.locator('.g-banner', { hasText: '用預設瀏覽器開啟' })
    await expect(banner).toBeVisible()
    await banner.getByRole('button', { name: '知道了' }).click()
    await expect(banner).toBeHidden()
    await page.reload()
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
    await expect(banner).toBeHidden()
    await owner.api.dispose()
  })

  test('localStorage 被禁用：會員仍可認捐（身分靠 session cookie，不靠本機儲存）', async ({ page }) => {
    const owner = await registerUser()
    const list = await createList(owner)
    const member = await signInPage(page)
    await page.addInitScript(() => {
      Object.defineProperty(window, 'localStorage', { get() { throw new DOMException('The operation is insecure.', 'SecurityError') } })
    })
    await page.goto(`/s/${list.slug}`)
    const card = page.locator('li.g-card', { hasText: '奶瓶' })
    await card.getByRole('button', { name: '我來送' }).click()
    await page.getByRole('button', { name: '確定要送' }).click()
    await expect(page.getByRole('heading', { name: '✓ 這份交給你了！' })).toBeVisible()
    await expect(page.getByText('LINE 內建瀏覽器請保持登入狀態')).toBeVisible()
    await page.getByRole('button', { name: '回到清單繼續看' }).click()
    await page.reload() // session cookie 讓重新整理後仍認得
    await expect(card.getByRole('button', { name: '修改我要送的' })).toBeVisible()
    await Promise.all([owner.api.dispose(), member.api.dispose()])
  })

  test('所有本機儲存都不可用：頁面不壞，會員仍可認捐', async ({ page }) => {
    const owner = await registerUser()
    const list = await createList(owner)
    const member = await signInPage(page)
    await page.addInitScript(() => {
      const deny = () => { throw new DOMException('The operation is insecure.', 'SecurityError') }
      Object.defineProperty(window, 'localStorage', { get: deny })
      Object.defineProperty(window, 'sessionStorage', { get: deny })
      Object.defineProperty(Document.prototype, 'cookie', { get: () => '', set: () => {} })
    })
    await page.goto(`/s/${list.slug}`)
    await expect(page.getByRole('heading', { level: 1 })).toBeVisible() // 沒有因為 storage 例外而壞掉
    await page.locator('li.g-card', { hasText: '奶瓶' }).getByRole('button', { name: '我來送' }).click()
    await page.getByRole('dialog').getByRole('button', { name: '確定要送' }).click()
    await expect(page.getByRole('heading', { name: '✓ 這份交給你了！' })).toBeVisible()
    await Promise.all([owner.api.dispose(), member.api.dispose()])
  })
})
