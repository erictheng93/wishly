// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ApiError, useGuest, setGuestToken, getGuestToken } from '../app/composables/useGuest'

const fetch = vi.fn()
const sys = { value: null as any }
beforeEach(() => {
  sys.value = null
  vi.stubGlobal('useState', () => sys)
  localStorage.clear(); sessionStorage.clear(); setGuestToken(null)
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: 'https://api.example.test' } }))
  vi.stubGlobal('$fetch', fetch)
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks() })

it('local token wins over cookie and caller header; requests retain HttpOnly cookie fallback', async () => {
  localStorage.setItem('ws_guest_token', 'local')
  document.cookie = 'ws_guest_js=cookie; Path=/'
  fetch.mockResolvedValue({ id: 'claim' })
  await useGuest().api('/items/item/claims', { method: 'POST', headers: { 'X-Guest-Token': 'caller', 'Idempotency-Key': 'key' }, body: { qty: 1 } })
  expect(fetch.mock.calls[0]).toEqual(['https://api.example.test/api/v1/items/item/claims', {
    method: 'POST', body: { qty: 1 }, headers: { 'X-Guest-Token': 'local', 'Idempotency-Key': 'key' }, credentials: 'include',
  }])
})

it('cookie token beats sessionStorage and absence of JS token still sends cookies', async () => {
  document.cookie = 'ws_guest_js=cookie; Path=/'
  sessionStorage.setItem('ws_guest_token', 'session')
  await useGuest().api('/me/claims')
  expect(fetch.mock.calls[0]![1].headers).toEqual({ 'X-Guest-Token': 'cookie' })
  setGuestToken(null)
  await useGuest().api('/me/claims')
  expect(fetch.mock.calls[1]![1]).toMatchObject({ headers: {}, credentials: 'include' })
})

it('401 clears a stale guest token, but keepToken preserves it (claim POST 401 means not signed in)', async () => {
  fetch.mockRejectedValue({ response: { status: 401 }, data: { code: 'UNAUTHORIZED' } })
  setGuestToken('legacy')
  await expect(useGuest().api('/items/item/claims', { method: 'POST', keepToken: true })).rejects.toMatchObject({ status: 401, code: 'UNAUTHORIZED' })
  expect(getGuestToken()).toBe('legacy')
  await expect(useGuest().api('/claims/old', { method: 'DELETE' })).rejects.toMatchObject({ status: 401 })
  expect(getGuestToken()).toBeNull()
  expect(fetch).toHaveBeenCalledTimes(2) // 從不自動重送
})

it('turns disconnected transport into a typed NETWORK error', async () => {
  fetch.mockRejectedValue(new TypeError('offline'))
  await expect(useGuest().api('/me/claims')).rejects.toBeInstanceOf(ApiError)
  expect(sys.value?.kind).toBe('busy') // happy-dom 的 navigator.onLine 為 true：連不到 = 伺服器端問題
  await expect(useGuest().api('/me/claims')).rejects.toMatchObject({ status: 0, code: 'NETWORK' })
})
