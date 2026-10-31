import { installPrompt, isIos, isStandalone, sendPwaEvent } from '~/composables/usePwaContext'
import { getPref, setPref } from '~/composables/useGuest'

// 只在 production 註冊 service worker；dev 不註冊以免干擾開發。不支援（如部分內建瀏覽器）或失敗時靜默略過。
export default defineNuxtPlugin((nuxt) => {
  // 已安裝 PWA 或原生殼：html[data-app] 供 CSS 套用「像 App」的行為（見 tokens.css）
  if (__NATIVE__ || matchMedia('(display-mode: standalone)').matches) document.documentElement.dataset.app = ''
  if (!__NATIVE__) {
    // 事件回呼不在 Nuxt context 內，sendPwaEvent 要用 runtimeConfig，所以包 runWithContext
    const send = (e: Parameters<typeof sendPwaEvent>[0]) => nuxt.runWithContext(() => sendPwaEvent(e))
    // beforeinstallprompt 早於元件掛載：在此攔下，由 InstallHint 的自訂按鈕呼叫 prompt()
    addEventListener('beforeinstallprompt', (e) => { e.preventDefault(); installPrompt.value = e })
    addEventListener('appinstalled', () => { installPrompt.value = null; send({ event: 'pwa_installed', platform: 'android', context: 'appinstalled' }) })
    // iOS 沒有 appinstalled：首次在 standalone 啟動（其 storage 與 Safari 分開）且無標記時算一次安裝；
    // 標記寫不進去就不送，免得每次啟動都重送
    if (isStandalone() && isIos() && !getPref('ws_pwa_installed')) {
      setPref('ws_pwa_installed', '1')
      if (getPref('ws_pwa_installed')) send({ event: 'pwa_installed', platform: 'ios', context: 'first_run' })
    }
  }
  if (import.meta.dev || __NATIVE__ || !('serviceWorker' in navigator)) return
  const reg = () => navigator.serviceWorker.register('/sw.js').catch(() => {})
  // 水合可能發生在 load 之後，此時 load 事件不會再來
  if (document.readyState === 'complete') reg()
  else addEventListener('load', reg)
})
