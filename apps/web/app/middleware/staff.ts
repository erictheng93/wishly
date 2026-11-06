export default defineNuxtRouteMiddleware(async (to, from) => {
  if (import.meta.server) return
  const u = await useAuth().fetchMe(true) // 要拿到目前 session 的 mfa_verified
  if (!u) return navigateTo({ path: '/login', query: { redirect: to.fullPath, ...(useState<string>('auth-error', () => '').value === 'account_blocked' ? { error: 'account_blocked' } : {}) } })
  if (!u.is_staff) {
    // 剛從登入 / 註冊被帶回 /admin（redirect 參數）的一般使用者：改去 dashboard，不讓登入流程報錯；直接打網址則照舊 403（錯誤頁呈現為找不到頁面）
    if (from.path === '/login' || from.path === '/register') return navigateTo('/dashboard', { replace: true })
    return abortNavigation(createError({ statusCode: 403, statusMessage: useNuxtApp().$i18n.t('auth.api.staffOnly') }))
  }
  // 角色限定頁（definePageMeta 的 staffRole）：缺角色回後台首頁；staff_roles 未提供（舊後端）時不擋，交給後端 403
  const need = to.meta.staffRole as StaffRole | undefined
  if (need && u.staff_roles && !hasStaffRole(u, need)) return navigateTo('/admin', { replace: true })
  // 已啟用者此 session 須先驗證；開了強制旗標（#99）時未設定者也要先到 /admin/mfa 設定，否則由後台首頁提示
  if (!u.mfa_verified && (u.mfa_enabled || u.mfa_required) && to.path !== '/admin/mfa') return navigateTo({ path: '/admin/mfa', query: { redirect: to.fullPath } })
})
