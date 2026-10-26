// 部署：Cloudflare Pages（docs D48，使用者基於成本選擇）
// 原生殼（Tauri）建置：NUXT_NATIVE=1 nuxt generate → 純 SPA 靜態檔（無 SSR、無 service worker、不走 Cloudflare preset）
const native = !!process.env.NUXT_NATIVE

// 語系檔依功能區拆開（i18n/locales/<語系>/<區>.json，檔內最外層 key 即區名），各區獨立維護
const AREAS = ['common', 'errors', 'guest', 'creator', 'auth', 'lists', 'me', 'admin', 'settings', 'legal']
const loc = (code: string, language: string, name: string) => ({ code, language, name, files: AREAS.map(a => `${code}/${a}.json`) })

export default defineNuxtConfig({
  modules: ['@nuxtjs/i18n'],
  // 不加網址前綴（routeRules、分享連結 /s/[slug] 都維持原樣）；語言存 cookie ws_locale，首次依瀏覽器語言
  i18n: {
    strategy: 'no_prefix',
    defaultLocale: 'zh-Hant',
    locales: [loc('zh-Hant', 'zh-Hant-TW', '繁體中文'), loc('en', 'en', 'English'), loc('vi', 'vi', 'Tiếng Việt'), loc('id', 'id', 'Bahasa Indonesia')],
    langDir: 'locales',
    detectBrowserLanguage: { useCookie: true, cookieKey: 'ws_locale', fallbackLocale: 'zh-Hant', redirectOn: 'root' },
  },
  compatibilityDate: '2026-10-01',
  ssr: !native,
  nitro: { preset: native ? 'static' : 'cloudflare_pages' },
  vite: { define: { __NATIVE__: JSON.stringify(native) } },
  runtimeConfig: {
    // 建置時讀取：NUXT_DEMO=1 才是 S1 示範（/s/demo 回固定資料），只有 npm run deploy:demo 會設
    demo: process.env.NUXT_DEMO ?? '',
    // 建置（production）預設指向正式 API；dev 預設本機。可用 NUXT_PUBLIC_API_BASE 覆寫（建置時或 Pages 後台變數）
    public: { apiBase: process.env.NUXT_PUBLIC_API_BASE ?? (process.env.NODE_ENV === 'production' ? 'https://api.wishly.tw' : 'http://localhost:8080'),
      // 建置時寫入：Pages 執行期沒有這個變數（wrangler.jsonc 刻意不放 vars），只靠 NUXT_PUBLIC_* 執行期覆寫會是空字串
      turnstileSiteKey: process.env.NUXT_PUBLIC_TURNSTILE_SITE_KEY ?? '',
      // 原生殼沒有「網站網域」可用 location.origin，分享連結以此為準
      siteUrl: process.env.NUXT_PUBLIC_SITE_URL ?? 'https://wishly.tw' },
  },
  // 創建者頁面靠 cookie 驗證，只在 client 渲染；/s/** 維持 SSR（OG 預覽）
  routeRules: {
    ...Object.fromEntries(['/dashboard/**', '/lists/**', '/me/wallet', '/me/topup', '/me/refund', '/me/claims', '/settings', '/friends', '/invite/**', '/u/**', '/admin/**', '/login', '/register', '/forgot-password'].map(r => [r, { ssr: false }])),
    // 個人頁一律 noindex（F22）；client 渲染的空殼沒有 robots meta，改用回應標頭
    '/u/**': { ssr: false, headers: { 'X-Robots-Tag': 'noindex' } },
    // 測試用：NUXT_API_PROXY=<後端網址> 時，同網域的 /api/v1/** 代理到該後端（只有 npm run deploy:preview 會設）。
    // 為什麼：Pages 與臨時通道是不同網站，cookie 會被當第三方擋掉；同源代理後 cookie 是第一方。
    ...(process.env.NUXT_API_PROXY ? { '/api/v1/**': { proxy: `${process.env.NUXT_API_PROXY.replace(/\/$/, '')}/api/v1/**` } } : {}),
  },
  // PWA：手寫 manifest + public/sw.js（不用 @vite-pwa/nuxt，避免模組預設快取策略碰到即時資料與 SSR 分享頁）
  app: {
    head: {
      meta: [
        { name: 'viewport', content: 'width=device-width, initial-scale=1, viewport-fit=cover' },
        { name: 'theme-color', content: '#c8102e', media: '(prefers-color-scheme: light)' },
        { name: 'theme-color', content: '#0e0e0e', media: '(prefers-color-scheme: dark)' },
        { name: 'apple-mobile-web-app-capable', content: 'yes' },
        { name: 'mobile-web-app-capable', content: 'yes' },
        { name: 'apple-mobile-web-app-status-bar-style', content: 'default' },
        { name: 'apple-mobile-web-app-title', content: 'Wishly' },
      ],
      // 首繪前套用手動主題，避免閃白；key 與 composables/useTheme.ts 一致
      script: [{ innerHTML: "try{var t=localStorage.getItem('ws_theme');if(t==='dark'||t==='light')document.documentElement.dataset.theme=t;else if(t!=='auto')document.documentElement.dataset.theme='light'}catch(e){document.documentElement.dataset.theme='light'}", tagPriority: 'critical' }],
      link: [
        { rel: 'manifest', href: '/manifest.webmanifest' },
        { rel: 'apple-touch-icon', href: '/apple-touch-icon.png' },
      ],
    },
  },
})
