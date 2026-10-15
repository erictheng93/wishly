import { expect, test } from '@playwright/test'
import { createList, guestClaim, loginBrowser, makeStaff, registerUser } from './helpers'

// RWD：手機 / 平板 / 桌面三種寬度的版面行為（斷點見 app/assets/tokens.css）
const SIZES = { small320: { width: 320, height: 568 }, phone: { width: 390, height: 844 }, tablet: { width: 820, height: 1180 }, desktop: { width: 1440, height: 900 } } as const

for (const [name, vp] of Object.entries(SIZES)) {
  test(`版面 @${name} ${vp.width}`, async ({ page, context }) => {
    await page.setViewportSize(vp)
    const owner = await registerUser()
    const list = await createList(owner, { items: [{ title: '奶瓶', qty: 2 }, { title: '尿布', qty: 3 }] })
    await guestClaim(list.items[0].id, 1, '訪客甲')
    makeStaff(owner.email)
    await loginBrowser(context, owner)
    const noOverflow = () => page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)
    const cols = (sel: string) => page.locator(sel).first().evaluate(el => getComputedStyle(el).gridTemplateColumns.split(' ').length)

    await test.step('dashboard：標題不被擠成一字寬、無溢出', async () => {
      await page.goto('/dashboard')
      await expect(page.locator('.c-head h1')).toBeVisible()
      expect((await page.locator('.c-head h1').boundingBox())!.width).toBeGreaterThan(60)
      expect(await noOverflow()).toBe(true)
      expect(await page.locator('.c-rail').isVisible()).toBe(vp.width >= 1280)
      expect(await page.getByRole('link', { name: '好友' }).first().isVisible()).toBe(true) // header 或側欄其一
    })
    await test.step('edit：標題寬度、雙欄、無溢出', async () => {
      await page.goto(`/lists/${list.id}/edit`)
      await expect(page.locator('.c-head h1')).toBeVisible()
      expect((await page.locator('.c-head h1').boundingBox())!.width).toBeGreaterThan(60)
      expect(await noOverflow()).toBe(true)
      const split = await cols('.c-split')
      expect(split).toBe(vp.width >= 1100 ? 2 : 1)
      if (vp.width >= 1100) { // sticky 側欄不得被 header 蓋住，捲動後亦然
        await page.evaluate(() => scrollTo(0, 400))
        const head = (await page.locator('.c-head').boundingBox())!
        const aside = (await page.locator('.c-split>aside').boundingBox())!
        expect(aside.y).toBeGreaterThanOrEqual(head.y + head.height)
      }
    })
    if (vp.width >= 1280) await test.step('rail 與內容不重疊', async () => {
      await page.goto('/dashboard')
      const rail = (await page.locator('.c-rail').boundingBox())!
      const main = (await page.locator('.c-page').boundingBox())!
      expect(rail.x + rail.width).toBeLessThanOrEqual(main.x)
      // header 內與 rail 重複的導覽按鈕必須隱藏
      expect(await page.locator('.c-head .c-nav-dup:visible').count()).toBe(0)
    })
    await test.step('滑鼠 hover 有視覺回饋', async () => {
      await page.goto(`/lists/${list.id}/edit`)
      const btn = page.locator('.c-btn.primary').first()
      const bg = () => btn.evaluate(el => getComputedStyle(el).backgroundColor)
      const before = await bg()
      await btn.hover()
      await expect.poll(bg).not.toBe(before) // 有 .2s transition，需輪詢
    })
    await test.step('後台採購單：篩選在平板以上並排', async () => {
      await page.goto('/admin/orders')
      await expect(page.locator('.c-filters select')).toHaveCount(2)
      const ys = await page.locator('.c-filters select').evaluateAll(els => els.map(e => Math.round(e.getBoundingClientRect().top)))
      expect(ys).toHaveLength(2)
      expect(ys[0] === ys[1]).toBe(vp.width >= 768)
      expect(await noOverflow()).toBe(true)
    })
    await test.step('分享頁：品項欄數與無溢出', async () => {
      await page.goto(`/s/${list.slug}`)
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
      expect(await noOverflow()).toBe(true)
      expect(await cols('ul.g-grid')).toBe(vp.width >= 768 ? 2 : 1)
      for (const h of await page.locator('.g-foot a').evaluateAll(as => as.map(a => a.getBoundingClientRect().height))) expect(h).toBeGreaterThanOrEqual(44)
    })
    await test.step('認領 sheet 在視窗內', async () => {
      await page.locator('li.g-card').first().getByRole('button', { name: '我要送' }).click()
      const box = (await page.getByRole('dialog').boundingBox())!
      expect(box.x).toBeGreaterThanOrEqual(0)
      expect(box.x + box.width).toBeLessThanOrEqual(vp.width)
      expect(box.y + box.height).toBeLessThanOrEqual(vp.height)
    })
    await owner.api.dispose()
  })
}

test('App 模式標記：standalone 時 html 帶 data-app 並停用橡皮筋', async ({ page }) => {
  await page.emulateMedia({ media: 'screen' })
  await page.addInitScript(() => {
    const mm = window.matchMedia.bind(window)
    window.matchMedia = (q: string) => (q.includes('display-mode') ? ({ matches: true, media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false } as any) : mm(q))
  })
  await page.goto('/login')
  await expect(page.locator('html[data-app]')).toHaveCount(1)
  expect(await page.evaluate(() => getComputedStyle(document.documentElement).overscrollBehaviorY)).toBe('none')
})

test('橫放手機（844×390）：header / 底欄不固定、無溢出', async ({ page, context }) => {
  await page.setViewportSize({ width: 844, height: 390 })
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '奶瓶', qty: 2 }] })
  await loginBrowser(context, owner)
  const pos = (sel: string) => page.locator(sel).first().evaluate(el => getComputedStyle(el).position)
  await page.goto(`/lists/${list.id}/edit`)
  await expect(page.locator('.c-head h1')).toBeVisible()
  expect(await pos('.c-head')).toBe('static')
  expect(await pos('.c-bottom')).toBe('static')
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
  await page.goto(`/s/${list.slug}`)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  expect(await pos('.g-top')).toBe('static')
  await owner.api.dispose()
})
