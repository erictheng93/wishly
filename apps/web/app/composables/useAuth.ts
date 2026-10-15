export function useAuth() {
  const user = useState<any>('auth-user', () => null)
  const loaded = useState<boolean>('auth-loaded', () => false)
  const { api, base } = useApi()
  async function fetchMe(force = false) {
    if (loaded.value && !force) return user.value
    try { user.value = await api('/me', { noRedirect: true }); useState<string>('auth-error', () => '').value = '' } catch { user.value = null }
    loaded.value = true
    return user.value
  }
  async function logout() {
    try { await api('/auth/logout', { method: 'POST', noRedirect: true }) } catch {}
    user.value = null
    setNativeToken(null)
    await navigateTo('/login')
  }
  const post = (path: string, body: any) => api(path, { method: 'POST', body, noRedirect: true, credentials: 'include' })
  // 成功回 session cookie；重抓 /me 讓全域 user 就緒
  async function session(path: string, body: any) {
    const r = await post(path, body)
    if (r?.session_token) setNativeToken(r.session_token)
    return fetchMe(true)
  }
  const login = (email: string, password: string) => session('/auth/login', { email, password })
  const register = (email: string, password: string, display_name: string) => post('/auth/register', { email, password, display_name })
  const verifyRegister = (email: string, code: string) => session('/auth/register/verify', { email, code })
  const requestReset = (email: string) => post('/auth/password/reset/request', { email })
  const confirmReset = (email: string, code: string, new_password: string) => post('/auth/password/reset/confirm', { email, code, new_password })
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
      return link ? fetchMe(true) : session('/auth/native/exchange', { code: r.code, verifier: r.verifier })
    }
    window.location.href = `${base}/auth/oauth/${provider}/start?redirect=${encodeURIComponent(redirect)}${link ? '&link=true' : ''}`
  }
  return { oauth, user, fetchMe, logout, login, register, verifyRegister, requestReset, confirmReset }
}
