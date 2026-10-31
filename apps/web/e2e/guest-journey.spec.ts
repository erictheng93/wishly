import { expect, test } from '@playwright/test'
import { createList, registerUser, signInPage } from './helpers'

test('會員認捐旅程：SSR meta → 捐助 → 重新整理 → 我的捐助 → 修改 → 取消', async ({ page, request }) => {
  const owner = await registerUser({ name: '小愛' })
  const list = await createList(owner, { title: '小愛的待產清單', items: [{ title: '奶瓶組', qty: 3 }, { title: '嬰兒帽', qty: 1 }] })
  const card = page.locator('li.g-card', { hasText: '奶瓶組' })
  const dialog = page.getByRole('dialog')

  await test.step('SSR 原始 HTML 含 OG meta（爬蟲看得到，不依賴 JS）', async () => {
    const res = await request.get(`/s/${list.slug}`)
    expect(res.status()).toBe(200)
    const html = await res.text()
    const meta = (p: string) => html.match(new RegExp(`<meta[^>]+(?:property|name)="${p}"[^>]+content="([^"]*)"`))?.[1]
    expect(meta('og:title')).toBe('幫小愛挑禮物｜小愛的待產清單')
    expect(meta('og:description')).toContain('登入會員後即可認領')
    expect(meta('og:type')).toBe('website')
    expect(meta('og:url')).toContain(`/s/${list.slug}`)
    expect(meta('og:image')).toMatch(/^https?:\/\/.+\.(png|jpg)/)
    expect(meta('twitter:card')).toBe('summary_large_image')
    expect(html).toContain('<h1>小愛的待產清單</h1>') // 內容也在首次 HTML 內
  })

  const member = await signInPage(page)
  await test.step('開啟分享頁，看到兩個品項與 0% 進度', async () => {
    await page.goto(`/s/${list.slug}`)
    await expect(page.getByRole('heading', { level: 1, name: '小愛的待產清單' })).toBeVisible()
    await expect(page.locator('li.g-card')).toHaveCount(2)
    await expect(card).toContainText('0 / 3')
  })

  await test.step('登入會員後開啟認捐：沒有暱稱 / 聯絡方式 / Email 欄位，以帳號名稱捐助', async () => {
    await card.getByRole('button', { name: '我來送' }).click()
    await expect(dialog.getByText('我來送「奶瓶組」')).toBeVisible()
    await expect(dialog.getByText(`以 ${member.name} 的身分送禮`)).toBeVisible()
    await expect(dialog.getByLabel(/暱稱|聯絡方式|Email/)).toHaveCount(0)
  })

  await test.step('數量 2，捐助成功', async () => {
    await dialog.getByRole('button', { name: '增加' }).click()
    await expect(dialog.locator('.g-step b')).toHaveText('2')
    await dialog.getByRole('button', { name: '確定要送' }).click()
    await expect(page.getByRole('heading', { name: '✓ 這份交給你了！' })).toBeVisible()
    await expect(page.getByText('你要送 奶瓶組 × 2')).toBeVisible()
    await page.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(card).toContainText('2 / 3')
  })

  await test.step('重新整理後仍顯示「修改我要送的」', async () => {
    await page.reload()
    await expect(card.getByRole('button', { name: '修改我要送的' })).toBeVisible()
    await expect(card).toContainText('你要送 2 件')
  })

  await test.step('/me/claims 列出這筆捐助', async () => {
    await page.getByRole('link', { name: '我的送禮' }).click()
    await expect(page).toHaveURL(/\/me\/claims$/)
    await expect(page.getByRole('link', { name: '小愛的待產清單' })).toBeVisible()
    await expect(page.getByText('奶瓶組 × 2')).toBeVisible()
    // 頁首帳號選單也會出現名字，只看內文
    await expect(page.getByRole('main').getByText(member.name)).toBeVisible()
  })

  await test.step('回清單修改數量為 1', async () => {
    await page.getByRole('link', { name: '小愛的待產清單' }).click()
    await card.getByRole('button', { name: '修改我要送的' }).click()
    await expect(dialog.getByText('修改我要送的「奶瓶組」')).toBeVisible()
    await dialog.getByRole('button', { name: '減少' }).click()
    await dialog.getByRole('button', { name: '儲存修改' }).click()
    await page.getByRole('button', { name: '回到清單繼續看' }).click()
    await expect(card).toContainText('1 / 3')
  })

  await test.step('在 /me/claims 取消，數量釋出', async () => {
    await page.goto('/me/claims')
    await expect(page.getByText('奶瓶組 × 1')).toBeVisible()
    page.once('dialog', d => d.accept())
    await page.getByRole('button', { name: '取消', exact: true }).click()
    await expect(page.getByText('過去的紀錄（1）')).toBeVisible() // 已取消的移到「過去的紀錄」
    await expect(page.getByRole('button', { name: '取消', exact: true })).toHaveCount(0)
    await page.goto(`/s/${list.slug}`)
    await expect(card).toContainText('0 / 3')
    await expect(card.getByRole('button', { name: '我來送' })).toBeVisible()
  })
  await Promise.all([owner.api.dispose(), member.api.dispose()])
})
