import { afterEach, beforeAll, expect, it, vi } from 'vitest'

const fetchMe = vi.fn(), navigate = vi.fn((x: any) => ({ nav: x })), abort = vi.fn((e: any) => ({ abort: e }))
let mw: (to: any, from: any) => Promise<any>
beforeAll(async () => {
  vi.stubGlobal('defineNuxtRouteMiddleware', (f: any) => f)
  vi.stubGlobal('useAuth', () => ({ fetchMe }))
  vi.stubGlobal('useState', () => ({ value: '' }))
  vi.stubGlobal('navigateTo', navigate)
  vi.stubGlobal('abortNavigation', abort)
  vi.stubGlobal('createError', (e: any) => e)
  mw = (await import('../app/middleware/staff')).default as any
})
afterEach(() => vi.clearAllMocks())
const to = { path: '/admin/analytics', fullPath: '/admin/analytics' }

it('non-staff arriving from login or register goes to dashboard instead of erroring', async () => {
  fetchMe.mockResolvedValue({ is_staff: false })
  for (const path of ['/login', '/register']) {
    expect(await mw(to, { path })).toEqual({ nav: '/dashboard' })
  }
  expect(navigate).toHaveBeenCalledWith('/dashboard', { replace: true })
  expect(abort).not.toHaveBeenCalled()
})

it('non-staff opening /admin directly still gets 403', async () => {
  fetchMe.mockResolvedValue({ is_staff: false })
  expect(await mw(to, { path: '/dashboard' })).toEqual({ abort: { statusCode: 403, statusMessage: '這個頁面僅限營運人員' } })
  expect(await mw(to, to)).toEqual({ abort: { statusCode: 403, statusMessage: '這個頁面僅限營運人員' } })
  expect(navigate).not.toHaveBeenCalled()
})

it('signed-out user is sent to login with redirect back', async () => {
  fetchMe.mockResolvedValue(null)
  expect(await mw(to, { path: '/' })).toEqual({ nav: { path: '/login', query: { redirect: '/admin/analytics' } } })
})

it('staff passes; unverified MFA, or not set up while enforcement is on, goes to /admin/mfa', async () => {
  const mfa = { nav: { path: '/admin/mfa', query: { redirect: '/admin/analytics' } } }
  fetchMe.mockResolvedValue({ is_staff: true, mfa_enabled: false, mfa_verified: false, mfa_required: false })
  expect(await mw(to, { path: '/login' })).toBeUndefined()
  fetchMe.mockResolvedValue({ is_staff: true, mfa_enabled: true, mfa_verified: true })
  expect(await mw(to, { path: '/login' })).toBeUndefined()
  fetchMe.mockResolvedValue({ is_staff: true, mfa_enabled: false, mfa_verified: false, mfa_required: true })
  expect(await mw(to, { path: '/login' })).toEqual(mfa)
  fetchMe.mockResolvedValue({ is_staff: true, mfa_enabled: true, mfa_verified: false })
  expect(await mw(to, { path: '/login' })).toEqual(mfa)
  expect(await mw({ path: '/admin/mfa', fullPath: '/admin/mfa' }, { path: '/login' })).toBeUndefined()
})
