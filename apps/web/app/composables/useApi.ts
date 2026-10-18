// 創建者端 API 客戶端：cookie（ws_session）驗證；原生殼改用 Bearer（見 utils/native.ts）、RFC 9457 錯誤正規化
export class CreatorApiError extends Error {
  constructor(public status: number, public code: string, public detail: string, public errors: any[] = [], public body: any = {}) { super(detail) }
}

export function errMsg(e: any): string {
  const t = useNuxtApp().$i18n.t
  if (!(e instanceof CreatorApiError)) return t('auth.api.unknown')
  if (e.code === 'READ_ONLY_MODE') return t('auth.api.readOnly')
  if (e.code === 'NETWORK') return t('auth.api.network')
  if (e.errors.length) return e.errors.map((x: any) => x.detail).join('；')
  return e.detail
}

export function useApi() {
  const router = useRouter()
  const base = useRuntimeConfig().public.apiBase.replace(/\/$/, '') + '/api/v1'
  async function api<T = any>(path: string, opts: any = {}): Promise<T> {
    const t = useNuxtApp().$i18n.t
    const { noRedirect, ...rest } = opts
    try {
      return await $fetch(path, { baseURL: base, credentials: 'include', ...rest, headers: { 'Accept-Language': useNuxtApp().$i18n.locale.value === 'zh-Hant' ? 'zh-TW' : useNuxtApp().$i18n.locale.value, ...nativeHeaders(), ...rest.headers } }) as T
    } catch (e: any) {
      const status = e.statusCode || e.status || 0
      const d = e.data && typeof e.data === 'object' ? e.data : {}
      const err = new CreatorApiError(status, d.code || (status ? 'HTTP_' + status : 'NETWORK'), d.detail || t('auth.api.connFail'), d.errors || [], d)
      const blocked = err.code === 'ACCOUNT_BLOCKED'
      if (import.meta.client && (blocked || (err.code === 'UNAUTHORIZED' && !noRedirect))) {
        useState('auth-user').value = null
        setNativeToken(null)
        if (blocked) useState<string>('auth-error', () => '').value = 'account_blocked'
        if (!noRedirect) await navigateTo({ path: '/login', query: { redirect: router.currentRoute.value.fullPath, ...(blocked ? { error: 'account_blocked' } : {}) } })
      }
      throw err
    }
  }
  return { api, base }
}
