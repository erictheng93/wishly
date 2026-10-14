import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { useAuth } from '../app/composables/useAuth'

const states = new Map<string, ReturnType<typeof ref>>()
const api = vi.fn(), token = vi.fn(), navigate = vi.fn()
beforeEach(() => {
  states.clear()
  vi.stubGlobal('useState', (key: string, init: () => any) => {
    if (!states.has(key)) states.set(key, ref(init()))
    return states.get(key)
  })
  vi.stubGlobal('useApi', () => ({ api, base: '/api/v1' }))
  vi.stubGlobal('setNativeToken', token)
  vi.stubGlobal('navigateTo', navigate)
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks() })

it('shares cached identity, forced refresh replaces stale data and clears auth errors', async () => {
  api.mockResolvedValueOnce({ id: 'first' }).mockResolvedValueOnce({ id: 'second' })
  const auth = useAuth()
  expect(await auth.fetchMe()).toEqual({ id: 'first' })
  expect(await useAuth().fetchMe()).toEqual({ id: 'first' })
  expect(api).toHaveBeenCalledTimes(1)
  states.get('auth-error')!.value = 'account_blocked'
  expect(await auth.fetchMe(true)).toEqual({ id: 'second' })
  expect(auth.user.value).toEqual({ id: 'second' })
  expect(states.get('auth-error')!.value).toBe('')
})

it('failed refresh removes stale identity and remains cached until forced', async () => {
  const auth = useAuth()
  auth.user.value = { id: 'stale' }
  api.mockRejectedValue(new Error('401'))
  expect(await auth.fetchMe()).toBeNull()
  expect(auth.user.value).toBeNull()
  await auth.fetchMe()
  expect(api).toHaveBeenCalledTimes(1)
})

it.each(['login', 'verifyRegister'] as const)('%s saves native session before fetching fresh identity', async method => {
  api.mockResolvedValueOnce({ session_token: 'session' }).mockResolvedValueOnce({ id: 'signed-in' })
  const auth = useAuth()
  expect(await auth[method]('me@example.test', 'secret')).toEqual({ id: 'signed-in' })
  expect(api.mock.calls[0]).toEqual([method === 'login' ? '/auth/login' : '/auth/register/verify', {
    method: 'POST', body: method === 'login' ? { email: 'me@example.test', password: 'secret' } : { email: 'me@example.test', code: 'secret' },
    noRedirect: true, credentials: 'include',
  }])
  expect(token).toHaveBeenCalledWith('session')
  expect(api.mock.calls[1]).toEqual(['/me', { noRedirect: true }])
})

it('logout clears identity and native token even if server logout fails', async () => {
  const auth = useAuth()
  auth.user.value = { id: 'old' }
  api.mockRejectedValue(new Error('offline'))
  await auth.logout()
  expect(auth.user.value).toBeNull()
  expect(token).toHaveBeenCalledWith(null)
  expect(navigate).toHaveBeenCalledWith('/login')
})
