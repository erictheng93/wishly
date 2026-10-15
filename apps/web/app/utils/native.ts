// 原生殼（Tauri）：webview 的 origin（tauri://localhost）與 API 跨站，SameSite=Lax 的 ws_session cookie 不會送出，
// 所以登入時帶 X-Native-Client 取回 session_token，之後以 Authorization: Bearer 驗證。
// __NATIVE__ 為建置時常數（NUXT_NATIVE=1），網頁版整段會被 tree-shake。
// 權杖存 OS keychain（Rust 端 session_token_*），記憶體持有以保持 nativeHeaders() 同步；啟動時由 initNativeSession() 載入。
const LEGACY_KEY = 'ws_session_token'
let token: string | null = null

export const isNative = () => __NATIVE__

// 不記錄權杖本身
const warn = (what: string) => console.warn(`[native] keychain ${what} failed`)

async function persist(t: string | null) {
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    if (t) await invoke('session_token_set', { token: t })
    else await invoke('session_token_clear')
  } catch { warn(t ? 'set' : 'clear') }
}

/** 啟動時呼叫一次：從 keychain 載入權杖；並把舊版 localStorage 權杖搬進 keychain 後刪除 */
export async function initNativeSession() {
  if (!__NATIVE__) return
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    token = (await invoke<string | null>('session_token_get')) || null
  } catch { warn('get') }
  let legacy: string | null = null
  try { legacy = localStorage.getItem(LEGACY_KEY); localStorage.removeItem(LEGACY_KEY) } catch { /* ignore */ }
  if (!token && legacy) { token = legacy; await persist(token) }
}

export function setNativeToken(t: string | null) {
  if (!__NATIVE__) return
  token = t
  void persist(t)
}

/** 原生殼才有內容；網頁版回空物件，走 cookie */
export function nativeHeaders(): Record<string, string> {
  if (!__NATIVE__) return {}
  return { 'X-Native-Client': '1', ...(token ? { Authorization: `Bearer ${token}` } : {}) }
}

/** 外部網址／sms:／line: 等：原生殼交給系統處理，網頁版開新分頁 */
export async function openExternal(url: string) {
  if (__NATIVE__) { const { openUrl } = await import('@tauri-apps/plugin-opener'); return openUrl(url) }
  window.open(url, '_blank', 'noopener')
}

/** 複製文字；失敗會丟出例外，由呼叫端 catch */
export async function copyText(text: string) {
  if (__NATIVE__) { const { writeText } = await import('@tauri-apps/plugin-clipboard-manager'); return writeText(text) }
  await navigator.clipboard.writeText(text)
}

/** 分享連結的網站網域：網頁版就是目前網域，原生殼用設定值 */
export const siteOrigin = () => (__NATIVE__ ? useRuntimeConfig().public.siteUrl : location.origin)

const CALLBACK = 'wishsync://auth'
const b64url = (b: ArrayBuffer | Uint8Array) =>
  btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
let cancelPending: (() => void) | null = null

/**
 * 原生殼 OAuth（系統瀏覽器 + deep link + PKCE）：先監聽 wishsync://auth，再開瀏覽器，等到 ?code=（或 ?error=）。
 * 回傳一次性碼與 verifier，由呼叫端 POST /auth/native/exchange 兌換。
 * 同時只保留一次進行中的嘗試（舊的以 'cancelled' 拒絕），否則過期嘗試會拿新的 deep link 去兌換並燒掉碼。
 * ponytail: verifier 只放記憶體；若 OS 在使用者登入期間回收 App，需重新登入。
 */
export async function nativeOAuth(startUrl: (challenge: string) => string): Promise<{ code: string; verifier: string }> {
  cancelPending?.()
  const [{ onOpenUrl }, { openUrl }] = await Promise.all([import('@tauri-apps/plugin-deep-link'), import('@tauri-apps/plugin-opener')])
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)))
  const challenge = b64url(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(verifier)))
  let settle!: (r: { code?: string; error?: string }) => void
  const result = new Promise<{ code?: string; error?: string }>((r) => { settle = r })
  const timer = setTimeout(() => settle({ error: 'timeout' }), 5 * 60_000)
  const off = await onOpenUrl((urls) => {
    for (const u of urls) {
      if (!u.startsWith(CALLBACK)) continue
      const p = new URL(u).searchParams
      settle({ code: p.get('code') || undefined, error: p.get('error') || 'oauth_failed' })
    }
  })
  const cancel = () => settle({ error: 'cancelled' })
  cancelPending = cancel
  try {
    await openUrl(startUrl(challenge))
    const r = await result
    if (!r.code) throw new Error(r.error)
    return { code: r.code, verifier }
  } finally {
    clearTimeout(timer)
    off()
    if (cancelPending === cancel) cancelPending = null
  }
}
