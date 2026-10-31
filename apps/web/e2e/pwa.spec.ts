import { expect, test } from '@playwright/test'
import { claimViaUi, createList, registerUser, signInPage } from './helpers'

// PWA（F-22）：manifest、Service Worker 安裝/範圍、快取內容、離線導覽。
// 此 spec 不依賴 API；用 /login（CSR 頁）當入口。Service Worker 僅在 chromium 且 production build 可用。
test.use({ serviceWorkers: 'allow' })

async function swReady(page: import('@playwright/test').Page) {
  await page.goto('/login')
  await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined))
  // 首次載入 SW 尚未控制頁面（不 skipWaiting/claim，刻意設計）；重新載入後才受控
  await page.reload()
  await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true)
}

test('/offline 回 200 且是離線頁', async ({ request }) => {
  const r = await request.get('/offline')
  expect(r.status()).toBe(200)
  expect(r.headers()['content-type']).toContain('text/html')
  expect(await r.text()).toContain('目前離線')
})

test('manifest 可解析、圖示皆 200', async ({ request, page }) => {
  await page.goto('/login')
  const href = await page.locator('link[rel=manifest]').getAttribute('href')
  const res = await request.get(href!)
  expect(res.status()).toBe(200)
  const m = await res.json()
  expect(m).toMatchObject({ scope: '/', display: 'standalone' })
  expect(m.icons.length).toBeGreaterThan(0)
  for (const ic of m.icons) {
    const r = await request.get(ic.src)
    expect(r.status(), ic.src).toBe(200)
    expect(r.headers()['content-type']).toContain('image/png')
  }
})

test('SW 註冊、scope 為 /，快取只含離線頁與靜態資產', async ({ page }) => {
  await swReady(page)
  const reg = await page.evaluate(async () => {
    const r = await navigator.serviceWorker.getRegistration()
    return { scope: r?.scope, active: r?.active?.state }
  })
  expect(new URL(reg.scope!).pathname).toBe('/')
  expect(reg.active).toBe('activated')

  // 再走訪幾個頁面（含 /s/*、API 呼叫），確保它們不會被寫入快取
  await page.goto('/s/does-not-exist').catch(() => {})
  await page.goto('/register')
  await page.evaluate(() => fetch('/api/v1/me').catch(() => {}))

  const keys = await page.evaluate(async () => {
    const out: string[] = []
    for (const n of await caches.keys()) for (const r of await (await caches.open(n)).keys()) out.push(new URL(r.url).pathname)
    return out
  })
  expect(keys).toContain('/offline')
  expect(keys).toContain('/icons/icon-192.png')
  for (const p of keys) {
    expect(p, '不得快取分享頁').not.toMatch(/^\/s\//)
    expect(p, '不得快取 API').not.toMatch(/^\/api\//)
    expect(p, '不得快取 HTML 導覽').not.toMatch(/^\/(login|register|dashboard|lists|settings|admin|forgot-password)?$/)
  }
  // 只允許：/offline、/icons/*、/_nuxt/*（不含 /_nuxt/builds/*）
  for (const p of keys) expect(p).toMatch(/^\/(offline|icons\/.+|_nuxt\/(?!builds\/).+)$/)
})

// 限制：context.setOffline 讓「網路」失敗，但 SW 本身仍在；等同真實離線時 fetch() 拒絕的情況，足以驗證 fallback。
test('離線時導覽顯示離線頁，恢復連線後可正常載入', async ({ page, context }) => {
  await swReady(page)
  await context.setOffline(true)
  await page.goto('/register').catch(() => {})
  await expect(page.getByRole('heading', { name: '目前離線' })).toBeVisible()
  await context.setOffline(false)
  // 離線頁監聽 online 事件並自動重新整理，所以不另外 goto（會與自動重載競態）
  await expect(page.getByRole('heading', { name: '目前離線' })).toBeHidden()
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
})

// --- SW：/_nuxt/* 快取數量上限（sw.js MAX_NUXT = 300）---
test('/_nuxt 快取超過上限時，最舊的被清掉；/offline 與圖示不受影響', async ({ page }) => {
  await swReady(page)
  const MAX = 300
  // 直接塞 MAX + 10 筆假的舊資產（插入順序即新舊順序）
  const asset = await page.evaluate(async (n) => {
    const c = await caches.open((await caches.keys())[0]!)
    for (let i = 0; i < n; i++) await c.put(`/_nuxt/old-${i}.js`, new Response('x'))
    // 挑一個真實資產：移除快取後重新 fetch，讓 SW 走「寫入 + 修剪」路徑
    const src = document.querySelector<HTMLScriptElement>('script[type=module][src*="/_nuxt/"]')!.src
    await c.delete(src)
    await fetch(src)
    return new URL(src).pathname
  }, MAX + 10)
  const paths = () => page.evaluate(async () => {
    const c = await caches.open((await caches.keys())[0]!)
    return (await c.keys()).map(r => new URL(r.url).pathname)
  })
  await expect.poll(async () => (await paths()).filter(p => p.startsWith('/_nuxt/')).length).toBeLessThanOrEqual(MAX)
  const left = await paths()
  expect(left).not.toContain('/_nuxt/old-0.js') // 最舊的被刪
  expect(left).toContain('/_nuxt/old-309.js') // 較新的保留
  expect(left).toContain(asset) // 剛寫入的保留
  expect(left).toContain('/offline')
  expect(left).toContain('/icons/icon-192.png')
})

// --- InstallHint（03 §5.8）---
const IOS_SAFARI = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Mobile/15E148 Safari/604.1'
const LINE_UA = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_4 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari Line/14.5.0'

/** 登入並認捐「奶瓶」，停在成功畫面；可選擇先派送合成的 beforeinstallprompt */
async function claimFirst(page: import('@playwright/test').Page, opts: { prompt?: boolean } = {}) {
  const owner = await registerUser()
  const list = await createList(owner, { items: [{ title: '奶瓶' }, { title: '尿布' }] })
  const member = await signInPage(page)
  await page.goto(`/s/${list.slug}`)
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await expect(page.locator('li.g-card', { hasText: '奶瓶' }).getByRole('button', { name: '我來送' })).toBeVisible() // 水合完成（plugin 已註冊監聽）
  if (opts.prompt) await firePrompt(page)
  await claimViaUi(page, '奶瓶')
  return { list, cleanup: () => Promise.all([owner.api.dispose(), member.api.dispose()]) }
}

async function firePrompt(page: import('@playwright/test').Page) {
  await page.evaluate(() => {
    const e: any = new Event('beforeinstallprompt', { cancelable: true })
    e.prompt = () => { (window as any).__prompted = true; return Promise.resolve() }
    ;(window as any).__defaultPrevented = !window.dispatchEvent(e)
  })
}

test('Android：攔下 beforeinstallprompt，按鈕呼叫 prompt() 並送出 prompted / installed 事件', async ({ page }) => {
  const ev = page.waitForRequest(r => r.url().endsWith('/api/v1/pwa-events') && r.method() === 'POST')
  const { cleanup } = await claimFirst(page, { prompt: true })
  expect(await page.evaluate(() => (window as any).__defaultPrevented)).toBe(true)
  const hint = page.locator('.js-install-hint')
  await expect(hint).toBeVisible()
  expect(JSON.parse((await ev).postData()!)).toMatchObject({ event: 'pwa_install_prompted', platform: 'android', context: 'claim' })
  await hint.getByRole('button', { name: '加入主畫面' }).click()
  expect(await page.evaluate(() => (window as any).__prompted)).toBe(true)
  await expect(hint).toBeHidden()
  const inst = page.waitForRequest(r => r.url().endsWith('/api/v1/pwa-events') && r.postData()?.includes('pwa_installed') === true)
  await page.evaluate(() => window.dispatchEvent(new Event('appinstalled')))
  expect(JSON.parse((await inst).postData()!)).toMatchObject({ event: 'pwa_installed', platform: 'android', context: 'appinstalled' })
  await cleanup()
})

test('Android：「不再提醒」之後不再顯示', async ({ page }) => {
  const { cleanup } = await claimFirst(page, { prompt: true })
  await page.locator('.js-install-hint').getByRole('button', { name: '不再提醒' }).click()
  await expect(page.locator('.js-install-hint')).toBeHidden()
  await page.reload()
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible()
  await firePrompt(page)
  await claimViaUi(page, '尿布')
  await expect(page.locator('.js-install-hint')).toHaveCount(0)
  await cleanup()
})

test('standalone 不顯示安裝提示', async ({ page }) => {
  await page.addInitScript(() => Object.defineProperty(navigator, 'standalone', { value: true }))
  const { cleanup } = await claimFirst(page, { prompt: true })
  await expect(page.locator('.js-install-hint')).toHaveCount(0)
  await cleanup()
})

test.describe('iOS Safari', () => {
  test.use({ userAgent: IOS_SAFARI })
  test('顯示「分享 → 加入主畫面」指引，沒有安裝按鈕；稍後只關本次', async ({ page }) => {
    const { cleanup } = await claimFirst(page)
    const hint = page.locator('.js-install-hint')
    await expect(hint).toContainText('分享')
    await expect(hint.getByRole('button', { name: '加入主畫面' })).toHaveCount(0)
    await hint.getByRole('button', { name: '稍後' }).click()
    await expect(hint).toBeHidden()
    await cleanup()
  })
})

test.describe('LINE 內建瀏覽器', () => {
  test.use({ userAgent: LINE_UA })
  test('不顯示安裝提示（維持外開引導）', async ({ page }) => {
    const { cleanup } = await claimFirst(page, { prompt: true })
    await expect(page.getByText('LINE 內建瀏覽器請保持登入狀態')).toBeVisible()
    await expect(page.locator('.js-install-hint')).toHaveCount(0)
    await cleanup()
  })
})
