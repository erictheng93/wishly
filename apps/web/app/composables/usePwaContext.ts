// PWA 安裝提示的環境判斷（03 §5.8 InstallHint）與匿名分析事件（pwa_install_prompted / pwa_installed）。
import { computed, ref } from 'vue'
import { getPref, setPref } from './useGuest'

export type PwaContext = 'ios' | 'android' | 'line' | 'standalone'
type Ev = { event: 'pwa_install_prompted' | 'pwa_installed', platform: 'ios' | 'android', context: 'claim' | 'fund' | 'appinstalled' | 'first_run' }

// beforeinstallprompt 早於元件掛載觸發，由 plugins/pwa.client.ts 攔下存這裡
export const installPrompt = ref<any>(null)
// 「稍後」：只關到重新整理為止
const later = ref(false)

export const isIos = (ua = navigator.userAgent) => /iPad|iPhone|iPod/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)
export const isStandalone = () => matchMedia('(display-mode: standalone)').matches || (navigator as any).standalone === true

export function detectPwaContext(ua: string, standalone: boolean, hasPrompt: boolean, native = __NATIVE__): PwaContext | null {
  if (native) return null
  if (standalone) return 'standalone'
  if (/Line\//i.test(ua)) return 'line'
  if (hasPrompt) return 'android'
  // iOS 只有 Safari 本身能「加入主畫面」引導；Chrome / Firefox / 各 App 內建瀏覽器不算
  if (isIos(ua) && /Safari\//.test(ua) && !/CriOS|FxiOS|EdgiOS|OPiOS|FBAN|FBAV|Instagram/.test(ua)) return 'ios'
  return null
}

export function sendPwaEvent(e: Ev) {
  if (__NATIVE__ || !import.meta.client) return
  try {
    const { public: { apiBase } } = useRuntimeConfig()
    fetch(`${apiBase}/api/v1/pwa-events`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(e), keepalive: true }).catch(() => {})
  } catch { /* 分析失敗不影響 UI */ }
}

export function usePwaContext() {
  const context = computed<PwaContext | null>(() => {
    if (!import.meta.client || later.value || getPref('ws_hint_dismissed')) return null
    return detectPwaContext(navigator.userAgent, isStandalone(), !!installPrompt.value)
  })
  /** permanent=true 為「不再提醒」；false 為「稍後」 */
  function dismiss(permanent: boolean) { if (permanent) setPref('ws_hint_dismissed', '1'); later.value = true }
  async function install() {
    const p = installPrompt.value
    if (!p) return
    installPrompt.value = null // prompt() 只能呼叫一次
    try { await p.prompt() } catch { /* 使用者取消或瀏覽器拒絕 */ }
  }
  return { context, dismiss, install }
}
