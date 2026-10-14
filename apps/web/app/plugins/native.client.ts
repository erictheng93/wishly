// 原生殼：啟動前先從 keychain 載入權杖（enforce: pre，早於 middleware / fetchMe）；
// 並攔截 a[target=_blank]：站內路徑走 router，其餘（外站、sms:、line:）交給系統。網頁版整段不動作。
import { initNativeSession, openExternal } from '../utils/native'

export default defineNuxtPlugin({
  enforce: 'pre',
  async setup() {
    if (!__NATIVE__) return
    await initNativeSession()
    const router = useRouter()
    document.addEventListener('click', (e) => {
      const a = (e.target as Element | null)?.closest?.('a[target="_blank"]') as HTMLAnchorElement | null
      if (!a || !a.href || e.defaultPrevented) return
      e.preventDefault()
      const u = new URL(a.href)
      if (u.origin === location.origin) void router.push(u.pathname + u.search + u.hash)
      else void openExternal(a.href)
    })
  },
})
