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
    // 原生殼：系統瀏覽器登入後以 deep link 回 App，再憑 verifier 兌換權杖（不支援 link）
    if (isNative() && !link) {
      const { code, verifier } = await nativeOAuth(c => `${base}/auth/oauth/${provider}/start?native=true&app_challenge=${c}`)
      return session('/auth/native/exchange', { code, verifier })
    }
    window.location.href = `${base}/auth/oauth/${provider}/start?redirect=${encodeURIComponent(redirect)}${link ? '&link=true' : ''}`
  }
  return { oauth, user, fetchMe, logout, login, register, verifyRegister, requestReset, confirmReset }
}
