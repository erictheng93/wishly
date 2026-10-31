// 全站唯一一條站內通知串流：登入才開、登出即停（見 utils/inboxStream.ts）。
// 掛在 app:mounted 之後，避免在 hydration 期間改動畫面狀態。
import { startInboxStream } from '../utils/inboxStream'
import { nativeHeaders } from '../utils/native'

export default defineNuxtPlugin((nuxtApp) => {
  nuxtApp.hook('app:mounted', () => {
    const { user } = useAuth()
    const { base } = useApi()
    const { refresh, reset } = useInbox()
    let stream: { stop: () => void } | undefined
    watch(() => user.value?.id, (id) => {
      stream?.stop(); stream = undefined
      if (!id) return reset()
      stream = startInboxStream({ url: `${base}/me/notifications/stream`, headers: nativeHeaders, onEvent: refresh })
    }, { immediate: true })
    document.addEventListener('visibilitychange', () => { if (user.value && document.visibilityState === 'visible') refresh() })
  })
})
