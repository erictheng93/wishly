// Wishly 極小 service worker。內容是即時資料，所以刻意保守：只快取不可變的靜態檔。
// 換版：改 CACHE_VERSION 即可。不呼叫 skipWaiting/clients.claim：新版 SW 等所有分頁關閉後才接管，
// 避免舊頁面（舊 HTML 引用舊雜湊資產）在中途被新版 SW 接手而版本錯亂；/_nuxt/* 帶雜湊且 cache-first 本身不會錯配。
const CACHE_VERSION = 'v2' // v2：離線頁改為確認連得上才重新整理
const CACHE = `wishly-static-${CACHE_VERSION}`
const OFFLINE = '/offline'

// 預先快取離線頁與圖示（install 失敗則整個 SW 不安裝，不會留下半套）
self.addEventListener('install', e => {
  e.waitUntil(caches.open(CACHE).then(c => c.addAll([OFFLINE, '/icons/icon-192.png', '/icons/icon-512.png'])))
})

// 清掉舊版本快取
self.addEventListener('activate', e => {
  e.waitUntil(caches.keys().then(ks => Promise.all(ks.filter(k => k !== CACHE).map(k => caches.delete(k)))))
})

self.addEventListener('fetch', e => {
  const req = e.request
  const url = new URL(req.url)
  // 非 GET、跨來源（API 在 api.wishly.tw、SSE、Turnstile 等）一律不攔截，交給瀏覽器直連
  if (req.method !== 'GET' || url.origin !== location.origin) return

  // 導覽請求：network-only。分享頁 /s/*（SSR、LINE 爬蟲、即時進度）與已登入頁 HTML 絕不能是舊的、也不能跨使用者；
  // 只有網路失敗時才回離線頁。
  if (req.mode === 'navigate') {
    e.respondWith(fetch(req).catch(() => caches.match(OFFLINE)))
    return
  }

  // 帶雜湊的建置資產（檔名含內容雜湊，內容永不改變）與圖示：cache-first
  // /_nuxt/builds/* 是 meta（非雜湊、會變），排除
  if ((url.pathname.startsWith('/_nuxt/') && !url.pathname.startsWith('/_nuxt/builds/')) || url.pathname.startsWith('/icons/')) {
    e.respondWith(caches.match(req).then(hit => hit || fetch(req).then(res => {
      if (res.ok) { const copy = res.clone(); caches.open(CACHE).then(c => c.put(req, copy)) }
      return res
    })))
  }
  // 其餘（/api/*、text/event-stream、/_payload.json、manifest 等）不呼叫 respondWith：完全走網路
})

// Web Push（push.rs）：本文只有 { title, path }，不含信件內容。解析失敗仍要顯示通知（瀏覽器要求 push 事件必須顯示），不 throw。
self.addEventListener('push', e => {
  let d = {}
  try { d = e.data ? e.data.json() : {} } catch {}
  e.waitUntil(self.registration.showNotification(typeof d.title === 'string' && d.title ? d.title : 'Wishly', {
    icon: '/icons/icon-192.png',
    data: { path: typeof d.path === 'string' ? d.path : '/dashboard' },
  }))
})

// 只接受站內路徑；已有視窗就聚焦並導向，否則開新視窗
self.addEventListener('notificationclick', e => {
  e.notification.close()
  let url
  try {
    const u = new URL(e.notification.data?.path ?? '', location.origin)
    url = u.origin === location.origin ? u.href : new URL('/dashboard', location.origin).href
  } catch { url = new URL('/dashboard', location.origin).href }
  e.waitUntil(clients.matchAll({ type: 'window', includeUncontrolled: true }).then(ws => {
    const w = ws[0]
    return w ? w.focus().then(c => (c || w).navigate(url)) : clients.openWindow(url)
  }))
})
