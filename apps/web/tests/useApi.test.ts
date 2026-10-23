import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useApi } from '../app/composables/useApi'

const states = new Map<string, { value: any }>()
const navigate = vi.fn()
const token = vi.fn()
beforeEach(() => {
  states.clear()
  states.set('auth-user', { value: { id: 'blocked-user' } })
  vi.stubGlobal('useState', (key: string) => {
    if (!states.has(key)) states.set(key, { value: null })
    return states.get(key)
  })
  vi.stubGlobal('useRouter', () => ({ currentRoute: { value: { fullPath: '/settings' } } }))
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: '' } }))
  vi.stubGlobal('nativeHeaders', () => ({}))
  vi.stubGlobal('setNativeToken', token)
  vi.stubGlobal('navigateTo', navigate)
})
afterEach(() => { vi.unstubAllGlobals(); vi.clearAllMocks() })

it.each([false, true])('clears blocked auth even with noRedirect=%s', async (noRedirect) => {
  vi.stubGlobal('$fetch', vi.fn().mockRejectedValue({ statusCode: 403, data: { code: 'ACCOUNT_BLOCKED', detail: '帳號已停權，請聯絡客服' } }))
  await expect(useApi().api('/me', { noRedirect })).rejects.toMatchObject({ code: 'ACCOUNT_BLOCKED' })
  expect(states.get('auth-user')?.value).toBeNull()
  expect(states.get('auth-error')?.value).toBe('account_blocked')
  expect(token).toHaveBeenCalledWith(null)
  if (noRedirect) expect(navigate).not.toHaveBeenCalled()
  else expect(navigate).toHaveBeenCalledWith({ path: '/login', query: { redirect: '/settings', error: 'account_blocked' } })
})

it('keeps auth for unrelated 403 responses', async () => {
  vi.stubGlobal('$fetch', vi.fn().mockRejectedValue({ statusCode: 403, data: { code: 'STAFF_ONLY' } }))
  await expect(useApi().api('/admin')).rejects.toMatchObject({ code: 'STAFF_ONLY' })
  expect(states.get('auth-user')?.value).toEqual({ id: 'blocked-user' })
  expect(token).not.toHaveBeenCalled()
  expect(navigate).not.toHaveBeenCalled()
})

it('404 without detail says not found, not "connection failed"', async () => {
  vi.stubGlobal('$fetch', vi.fn().mockRejectedValue({ statusCode: 404, data: { code: 'NOT_FOUND', title: '找不到資源' } }))
  await expect(useApi().api('/wishlists/x')).rejects.toMatchObject({ code: 'NOT_FOUND', detail: '找不到這個項目，可能已被刪除或你沒有權限' })
})
