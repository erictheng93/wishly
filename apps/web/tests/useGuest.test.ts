// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ApiError, useGuest, setGuestToken, setActiveAccess } from '../app/composables/useGuest'

const fetch = vi.fn()
beforeEach(() => {
  localStorage.clear(); sessionStorage.clear(); setGuestToken(null); setActiveAccess(null)
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

it('explicit list access overrides active access', async () => {
  setActiveAccess('active')
  await useGuest().api('/public/wishlists/list', { access: 'explicit' })
  expect(fetch.mock.calls[0]![1].headers).toEqual({ 'X-List-Access': 'explicit' })
})

it('expired token retries only once as new guest and preserves idempotency key', async () => {
  setGuestToken('expired')
  fetch.mockRejectedValueOnce({ response: { status: 401 }, data: { code: 'UNAUTHORIZED' } }).mockResolvedValueOnce({ id: 'created' })
  expect(await useGuest().api('/items/item/claims', { method: 'POST', headers: { 'Idempotency-Key': 'same' }, body: { qty: 2 }, retryAsNewGuest: true })).toEqual({ id: 'created' })
  expect(fetch.mock.calls[0]![1].headers['X-Guest-Token']).toBe('expired')
  expect(fetch.mock.calls[1]![1]).toMatchObject({ headers: { 'Idempotency-Key': 'same' }, body: { qty: 2 }, credentials: 'include' })
  expect(fetch.mock.calls[1]![1].headers).not.toHaveProperty('X-Guest-Token')
})

it('does not silently retry management operations or loop on repeated unauthorized responses', async () => {
  setGuestToken('expired')
  fetch.mockRejectedValue({ response: { status: 401 }, data: { code: 'UNAUTHORIZED' } })
  await expect(useGuest().api('/claims/old', { method: 'DELETE' })).rejects.toMatchObject({ status: 401 })
  expect(fetch).toHaveBeenCalledTimes(1)
  setGuestToken('expired')
  await expect(useGuest().api('/items/item/claims', { retryAsNewGuest: true })).rejects.toMatchObject({ status: 401 })
  expect(fetch).toHaveBeenCalledTimes(3)
})

it('turns disconnected transport into a typed NETWORK error', async () => {
  fetch.mockRejectedValue(new TypeError('offline'))
  await expect(useGuest().api('/me/claims')).rejects.toBeInstanceOf(ApiError)
  await expect(useGuest().api('/me/claims')).rejects.toMatchObject({ status: 0, code: 'NETWORK' })
})
