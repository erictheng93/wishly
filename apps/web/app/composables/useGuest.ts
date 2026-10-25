import { apiErrMsg, problemDetail } from '../utils/errors'
import { nativeHeaders } from '../utils/native'
import { noteUnavailable } from './useSysStatus'
// 訪客 token 儲存 + API 呼叫。
// LINE in-app browser 的 localStorage / cookie 可能被清掉或停用，所以依序寫入多處、讀取時取第一個有值的：
//   localStorage -> JS cookie(ws_guest_js) -> sessionStorage -> 記憶體。
// 伺服器另外會 Set-Cookie ws_guest（HttpOnly），請求一律 credentials:'include'，header(X-Guest-Token) 優先、cookie 為備援。
// ponytail: 不做 IndexedDB / window.name 備援；若全部失敗只在成功頁警告「請截圖保存」，換裝置靠 Email 恢復連結。
const KEY = 'ws_guest_token'
let mem: string | null = null

const safe = <T>(f: () => T): T | null => { try { return f() } catch { return null } }
const readCookie = () => safe(() => document.cookie.split('; ').find(c => c.startsWith('ws_guest_js='))?.split('=')[1] ?? null)

export class ApiError extends Error {
  constructor(public status: number, public code: string, public detail: string, public data: any = {}) { super(detail) }
}

export function getGuestToken(): string | null {
  if (!import.meta.client) return null
  return safe(() => localStorage.getItem(KEY)) || readCookie() || safe(() => sessionStorage.getItem(KEY)) || mem
}

/** 回傳是否至少有一個「持久」儲存成功（localStorage 或 cookie） */
export function setGuestToken(t: string | null): boolean {
  mem = t
  let persisted = false
  if (t) {
    persisted = safe(() => { localStorage.setItem(KEY, t); return localStorage.getItem(KEY) === t }) === true
    safe(() => { document.cookie = `ws_guest_js=${t}; Path=/; Max-Age=31536000; SameSite=Lax`; persisted ||= readCookie() === t })
    safe(() => sessionStorage.setItem(KEY, t))
  } else {
    safe(() => localStorage.removeItem(KEY)); safe(() => sessionStorage.removeItem(KEY))
    safe(() => { document.cookie = 'ws_guest_js=; Path=/; Max-Age=0' })
  }
  return persisted
}

export const getPref = (k: string) => (import.meta.client ? safe(() => localStorage.getItem(k)) : null)
export const setPref = (k: string, v: string) => safe(() => localStorage.setItem(k, v))

export function useGuest() {
  const { public: { apiBase } } = useRuntimeConfig()
  // keepToken：401 時不清本機訪客 token。新增捐助需登入會員，其 401 只代表「沒登入」，不能讓舊訪客因此失去既有捐助的管理權
  async function api<T = any>(path: string, opt: { method?: string, body?: any, headers?: Record<string, string>, keepToken?: boolean } = {}): Promise<T> {
    const headers: Record<string, string> = { ...nativeHeaders(), ...opt.headers } // 原生殼：登入會員以 Bearer 驗證
    const tk = getGuestToken()
    if (tk) headers['X-Guest-Token'] = tk
    try {
      return await $fetch(`${apiBase}/api/v1${path}`, { method: (opt.method ?? 'GET') as any, body: opt.body, headers, credentials: 'include' }) as T
    } catch (e: any) {
      const d = e?.data ?? {}
      if (!e?.response) { noteUnavailable(0); throw new ApiError(0, 'NETWORK', apiErrMsg({ code: 'NETWORK' })) }
      const status = e.response.status
      if (status === 401 && tk && !opt.keepToken) setGuestToken(null)
      const code = d.code ?? 'UNKNOWN'
      noteUnavailable(status, code, Number(e.response.headers?.get?.('retry-after')) || undefined)
      const errors = (d.errors ?? []).map((x: any) => ({ ...x, detail: problemDetail(x) }))
      throw new ApiError(status, code, apiErrMsg({ ...d, status, code }), { ...d, errors })
    }
  }
  const newKey = () => crypto.randomUUID()
  return { api, newKey, apiBase }
}

export const isLineBrowser = () => import.meta.client && /Line\//i.test(navigator.userAgent)
