// 全站唯一一條站內通知串流：登入且分頁在前景才開、登出即停（見 utils/inboxStream.ts）。
// 掛在 app:mounted 之後，避免在 hydration 期間改動畫面狀態。
import { startInboxStream } from '../utils/inboxStream'
import { nativeHeaders } from '../utils/native'

export default defineNuxtPlugin((nuxtApp) => {
  nuxtApp.hook('app:mounted', () => {
    const { user } = useAuth()
    const { base } = useApi()
    const { refresh, reset } = useInbox()
    let stream: { stop: () => void } | undefined, pause: ReturnType<typeof setTimeout> | undefined
    const visible = () => document.visibilityState === 'visible'
    const open = () => { stream ??= startInboxStream({ url: `${base}/me/notifications/stream`, headers: nativeHeaders, onEvent: refresh }) }
    const close = () => { stream?.stop(); stream = undefined }
    watch(() => user.value?.id, (id) => {
      clearTimeout(pause); close()
      if (!id) return reset()
      if (visible()) open()
    }, { immediate: true })
    // 背景分頁 1 分鐘後斷線，把名額讓出來（連線上限見 inbox.rs）；回前景重連時會補抓一次
    document.addEventListener('visibilitychange', () => {
      clearTimeout(pause)
      if (!user.value) return
      if (!visible()) pause = setTimeout(close, 60_000)
      else if (stream) refresh()
      else open()
    })
  })
})
