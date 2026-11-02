import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { useAuth } from '../app/composables/useAuth'

const states = new Map<string, ReturnType<typeof ref>>()
const api = vi.fn(), token = vi.fn(), navigate = vi.fn(), dropPush = vi.fn()
beforeEach(() => {
  states.clear()
  vi.stubGlobal('useState', (key: string, init: () => any) => {
    if (!states.has(key)) states.set(key, ref(init()))
    return states.get(key)
  })
  vi.stubGlobal('useApi', () => ({ api, base: '/api/v1' }))
  vi.stubGlobal('setNativeToken', token)
  vi.stubGlobal('navigateTo', navigate)
  vi.stubGlobal('dropPush', dropPush)
  vi.stubGlobal('useCookie', (key: string) => {
    if (!states.has(`cookie:${key}`)) states.set(`cookie:${key}`, ref(null))
    return states.get(`cookie:${key}`)
  })
})
const hint = () => states.get('cookie:ws_hint')?.value
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

it('requestLoginCode passes the captcha token without starting a session', async () => {
  api.mockResolvedValueOnce({ challenge_id: 'c' })
  expect(await useAuth().requestLoginCode('me@example.test', 'ts')).toEqual({ challenge_id: 'c' })
  expect(api).toHaveBeenCalledTimes(1)
  expect(api).toHaveBeenCalledWith('/auth/otp/request', { method: 'POST', body: { email: 'me@example.test', turnstile_token: 'ts' }, noRedirect: true, credentials: 'include' })
})

it('verifyLoginCode saves native session before fetching fresh identity', async () => {
  api.mockResolvedValueOnce({ session_token: 'session' }).mockResolvedValueOnce({ id: 'signed-in' })
  expect(await useAuth().verifyLoginCode('me@example.test', '123456', 'challenge')).toEqual({ id: 'signed-in' })
  expect(api.mock.calls[0]).toEqual(['/auth/otp/verify', {
    method: 'POST', body: { email: 'me@example.test', code: '123456', challenge_id: 'challenge' },
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

it('otp challenge id and turnstile token are sent to request / verify / confirm endpoints', async () => {
  api.mockResolvedValue({ challenge_id: 'c1' })
  const auth = useAuth()
  await auth.register('a@b.c', 'password1', 'n', 'ts')
  await auth.requestReset('a@b.c', 'ts')
  await auth.confirmReset('a@b.c', '123456', 'newpassword', 'c1')
  const bodies = api.mock.calls.map(c => c[1].body)
  expect(bodies[0]).toMatchObject({ turnstile_token: 'ts' })
  expect(bodies[1]).toEqual({ email: 'a@b.c', turnstile_token: 'ts' })
  expect(bodies[2]).toMatchObject({ challenge_id: 'c1' })
  await auth.verifyRegister('a@b.c', '123456', 'c1')
  expect(api.mock.calls[3][1].body).toEqual({ email: 'a@b.c', code: '123456', challenge_id: 'c1' })
})

it('ws_hint is set on sign-in, cleared on 401 or block, kept on network errors', async () => {
  const auth = useAuth()
  api.mockResolvedValueOnce({ id: 'me' })
  await auth.fetchMe(true)
  expect(hint()).toBe(true)
  api.mockRejectedValueOnce(new TypeError('Failed to fetch'))
  await auth.fetchMe(true)
  expect(hint()).toBe(true)
  api.mockRejectedValueOnce({ status: 401, code: 'UNAUTHORIZED' })
  await auth.fetchMe(true)
  expect(hint()).toBeNull()
  states.get('cookie:ws_hint')!.value = true
  api.mockRejectedValueOnce({ status: 403, code: 'ACCOUNT_BLOCKED' })
  await auth.fetchMe(true)
  expect(hint()).toBeNull()
})

it('logout clears ws_hint', async () => {
  const auth = useAuth()
  api.mockResolvedValueOnce({ id: 'me' })
  await auth.fetchMe(true)
  api.mockRejectedValue(new Error('offline'))
  await auth.logout()
  expect(hint()).toBeNull()
})
