export default defineNuxtRouteMiddleware(async (to) => {
  if (import.meta.server) return
  const u = await useAuth().fetchMe(true) // 要拿到目前 session 的 mfa_verified
  if (!u) return navigateTo({ path: '/login', query: { redirect: to.fullPath, ...(useState<string>('auth-error', () => '').value === 'account_blocked' ? { error: 'account_blocked' } : {}) } })
  if (!u.is_staff) return abortNavigation(createError({ statusCode: 403, statusMessage: '這個頁面僅限營運人員' }))
  // 已啟用兩步驟驗證：此 session 須先驗證；未啟用者由後台首頁提示（軟性引導）
  if (u.mfa_enabled && !u.mfa_verified && to.path !== '/admin/mfa') return navigateTo({ path: '/admin/mfa', query: { redirect: to.fullPath } })
})
