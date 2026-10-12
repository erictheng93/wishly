// 原生殼（Tauri）：webview 的 origin（tauri://localhost）與 API 跨站，SameSite=Lax 的 ws_session cookie 不會送出，
// 所以登入時帶 X-Native-Client 取回 session_token，之後以 Authorization: Bearer 驗證。
// __NATIVE__ 為建置時常數（NUXT_NATIVE=1），網頁版整段會被 tree-shake。
// ponytail: 權杖存 localStorage；上架前換成 Tauri 的 keychain / stronghold plugin。
const KEY = 'ws_session_token'
const safe = <T>(f: () => T): T | null => { try { return f() } catch { return null } }

export const isNative = () => __NATIVE__

export function setNativeToken(t: string | null) {
  if (!__NATIVE__) return
  safe(() => (t ? localStorage.setItem(KEY, t) : localStorage.removeItem(KEY)))
}

/** 原生殼才有內容；網頁版回空物件，走 cookie */
export function nativeHeaders(): Record<string, string> {
  if (!__NATIVE__) return {}
  const t = safe(() => localStorage.getItem(KEY))
  return { 'X-Native-Client': '1', ...(t ? { Authorization: `Bearer ${t}` } : {}) }
}

/** 分享連結的網站網域：網頁版就是目前網域，原生殼用設定值 */
export const siteOrigin = () => (__NATIVE__ ? useRuntimeConfig().public.siteUrl : location.origin)
