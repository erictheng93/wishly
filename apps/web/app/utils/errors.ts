const t = (k: string) => useNuxtApp().$i18n.t(k)

// 訪客端錯誤碼 → 使用者文案（純函式，可單元測試）
export function reportErrMsg(e: { code?: string, status?: number, detail?: string, data?: any }): { msg: string, resetCaptcha: boolean } {
  if (e.code === 'RATE_LIMITED' || e.status === 429) return { msg: t('errors.reportRate'), resetCaptcha: false }
  const turnstile = e.code === 'TURNSTILE_FAILED' || (e.data?.errors ?? []).some((x: any) => x.pointer === '/turnstile_token')
  if (turnstile) return { msg: t('errors.turnstile'), resetCaptcha: true }
  if (e.code === 'NETWORK') return { msg: t('errors.network'), resetCaptcha: true }
  return { msg: apiErrMsg(e), resetCaptcha: true }
}

// 一般 API 錯誤 → 使用者文案：先看 code / HTTP 狀態（不依賴後端是否有補 detail），其餘才用 detail
export function apiErrMsg(e: { code?: string, status?: number, detail?: string }): string {
  const s = e.status ?? 0
  if (e.code === 'NETWORK' || e.status === 0) return t('errors.network')
  if (e.code === 'WISHLIST_REMOVED' || s === 410) return t('errors.removed')
  if (e.code === 'UNAUTHORIZED' || s === 401) return t('errors.unauthorized')
  if (e.code === 'HANDLE_TAKEN') return t('errors.handleTaken')
  if (e.code === 'SELF_INVITE') return t('errors.selfInvite')
  if (e.code === 'LOGIN_REQUIRED') return t('errors.loginRequired')
  if (e.code === 'FRIENDS_ONLY') return t('errors.friendsOnly')
  if (e.code === 'NOT_ALLOWED') return t('errors.notAllowed')
  if (e.code === 'NOT_FRIEND') return t('errors.notFriend')
  if (e.code === 'RATE_LIMITED' || s === 429) return t('errors.rate')
  if (s === 503) return t('errors.maintenance')
  if (s === 404) return e.code === 'CLAIM_NOT_FOUND' ? t('errors.claimGone') : t('errors.itemGone')
  if (e.detail && s < 500) return e.detail
  if (s >= 500) return t('errors.server')
  return e.detail || t('errors.generic')
}

// 個人頁代號：小寫英數與底線，3–30 字
export const isValidHandle = (h: string) => /^[a-z0-9_]{3,30}$/.test(h)
