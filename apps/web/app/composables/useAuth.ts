// 「可能已登入」旗標（wishly.tw 的 cookie，不含身分、不是憑證）：ws_session 只在 API 網域，
// 首頁 SSR 讀不到，靠它先畫個人版骨架、免得閃一下行銷版。偽造也只會看到空骨架；效期同 session 閒置期限（30 天，fetchMe 成功即續期）
export const useAuthHint = () => useCookie<boolean | null>('ws_hint', { maxAge: 30 * 86400, sameSite: 'lax', path: '/' })

export function useAuth() {
  const user = useState<any>('auth-user', () => null)
  const loaded = useState<boolean>('auth-loaded', () => false)
  const { api, base } = useApi()
  async function fetchMe(force = false) {
    if (loaded.value && !force) return user.value
    const hint = useAuthHint()
    try {
      user.value = await api('/me', { noRedirect: true }); useState<string>('auth-error', () => '').value = ''
      hint.value = true
    } catch (e: any) {
      user.value = null
      if (e?.status === 401 || e?.code === 'ACCOUNT_BLOCKED') hint.value = null // 網路錯誤不清：只是暫時連不上
    }
    // 以目前介面語言為準，讓後端寄信（通知、安全通知）用同一種語言
    const cur = useNuxtApp().$i18n.locale.value
    const want = cur === 'zh-Hant' ? 'zh-TW' : cur
    if (user.value?.locale && user.value.locale !== want) api('/me', { method: 'PATCH', body: { locale: want }, noRedirect: true }).then((u) => { user.value = u }).catch(() => {})
    loaded.value = true
    return user.value
  }
  async function logout(redirect?: string) {
    const hint = useAuthHint() // await 之前取：useCookie 需要 Nuxt context
    await dropPush(api) // 共用裝置：先退訂，下一位使用者才不會收到前一位的通知（需趁 session 還在）
    try { await api('/auth/logout', { method: 'POST', noRedirect: true }) } catch {}
    user.value = null
    hint.value = null
    setNativeToken(null)
    await navigateTo(redirect ? { path: '/login', query: { redirect } } : '/login')
  }
  const post = (path: string, body: any) => api(path, { method: 'POST', body, noRedirect: true, credentials: 'include' })
  // 成功回 session cookie；重抓 /me 讓全域 user 就緒
  async function session(path: string, body: any) {
    const r = await post(path, body)
    if (r?.session_token) setNativeToken(r.session_token)
    return fetchMe(true)
  }
  const login = (email: string, password: string, turnstile_token?: string) => session('/auth/login', { email, password, turnstile_token })
  const register = (email: string, password: string, display_name: string, turnstile_token?: string) => post('/auth/register', { email, password, display_name, turnstile_token })
  const verifyRegister = (email: string, code: string, challenge_id?: string) => session('/auth/register/verify', { email, code, challenge_id })
  const requestReset = (email: string, turnstile_token?: string) => post('/auth/password/reset/request', { email, turnstile_token })
  const confirmReset = (email: string, code: string, new_password: string, challenge_id?: string) => post('/auth/password/reset/confirm', { email, code, new_password, challenge_id })
  async function oauth(provider: 'google' | 'line', redirect = '/dashboard', link = false) {
    // 原生殼：系統瀏覽器完成後以 deep link 回 App。登入憑 verifier 兌換權杖；
    // 綁定因瀏覽器沒有 session，先以 Bearer 換一張綁 app_challenge 的一次性 ticket
    if (isNative()) {
      const start = `${base}/auth/oauth/${provider}/start?native=true`
      const r = await nativeOAuth(async (c) => {
        if (!link) return `${start}&app_challenge=${c}`
        const { ticket } = await api(`/auth/oauth/${provider}/link-ticket`, { method: 'POST', body: { app_challenge: c }, noRedirect: true })
        return `${start}&link=true&app_challenge=${c}&ticket=${encodeURIComponent(ticket)}`
      })
      if (link) {
        await api('/auth/native/link-confirm', { method: 'POST', body: { code: r.linkCode, verifier: r.verifier }, noRedirect: true })
        return fetchMe(true)
      }
      return session('/auth/native/exchange', { code: r.code, verifier: r.verifier })
    }
    window.location.href = `${base}/auth/oauth/${provider}/start?redirect=${encodeURIComponent(redirect)}${link ? '&link=true' : ''}`
  }
  return { oauth, user, loaded, fetchMe, logout, login, register, verifyRegister, requestReset, confirmReset }
}
