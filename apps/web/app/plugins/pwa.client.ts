// 只在 production 註冊 service worker；dev 不註冊以免干擾開發。不支援（如部分內建瀏覽器）或失敗時靜默略過。
export default defineNuxtPlugin(() => {
  // 已安裝 PWA 或原生殼：html[data-app] 供 CSS 套用「像 App」的行為（見 tokens.css）
  if (__NATIVE__ || matchMedia('(display-mode: standalone)').matches) document.documentElement.dataset.app = ''
  if (import.meta.dev || __NATIVE__ || !('serviceWorker' in navigator)) return
  const reg = () => navigator.serviceWorker.register('/sw.js').catch(() => {})
  // 水合可能發生在 load 之後，此時 load 事件不會再來
  if (document.readyState === 'complete') reg()
  else addEventListener('load', reg)
})
