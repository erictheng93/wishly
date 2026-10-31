// 此裝置的 Web Push 開關。只有使用者按下開關才會跳出瀏覽器的通知權限詢問。
export type PushStatus = 'loading' | 'unavailable' | 'needs-install' | 'ready'

export function usePush() {
  const { api } = useApi()
  const status = ref<PushStatus>('loading')
  const enabled = ref(false)
  const denied = ref(false)
  const busy = ref(false)
  const error = ref('')
  let key = ''

  async function init() {
    if (__NATIVE__ || !('serviceWorker' in navigator)) return void (status.value = 'unavailable')
    try { key = (await api('/push/config', { noRedirect: true })).public_key ?? '' } catch { key = '' }
    if (!key) return void (status.value = 'unavailable')
    // iOS 只有加到主畫面的 App 才能收推播
    if (isIos() && !isStandalone()) return void (status.value = 'needs-install')
    if (!('PushManager' in window) || !('Notification' in window)) return void (status.value = 'unavailable')
    denied.value = Notification.permission === 'denied'
    enabled.value = !!(await currentPushSub().catch(() => null)) && Notification.permission === 'granted'
    status.value = 'ready'
  }

  async function set(on: boolean) {
    if (busy.value) return
    busy.value = true; error.value = ''
    try {
      if (!on) { await dropPush(api); enabled.value = false; return }
      const perm = await Notification.requestPermission()
      denied.value = perm === 'denied'
      if (perm !== 'granted') return
      const reg = await navigator.serviceWorker.getRegistration()
      if (!reg) throw new Error('no service worker')
      const sub = await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: b64urlToBytes(key) as BufferSource })
      try {
        await api('/me/push-subscriptions', { method: 'POST', body: sub.toJSON(), noRedirect: true })
      } catch (e) { await sub.unsubscribe().catch(() => {}); throw e }
      enabled.value = true
    } catch (e) {
      error.value = e instanceof CreatorApiError ? errMsg(e) : useNuxtApp().$i18n.t('settings.push.failed')
    } finally { busy.value = false }
  }

  return { status, enabled, denied, busy, error, init, set }
}
