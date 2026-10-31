// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { nextTick, onMounted, ref } from 'vue'
import PushToggle from '../app/components/creator/PushToggle.vue'
import { usePush } from '../app/composables/usePush'
import * as push from '../app/utils/push'
import { flush, mount } from './vueHarness'

class ApiErr extends Error {}
const api = vi.fn()
const sub = { endpoint: 'https://fcm.googleapis.com/x', toJSON: () => ({ endpoint: 'https://fcm.googleapis.com/x', keys: { p256dh: 'p', auth: 'a' } }), unsubscribe: vi.fn(async () => true) }
let existing: typeof sub | null
const subscribe = vi.fn(async () => sub)
let permission: string
const requestPermission = vi.fn(async () => permission)

function env({ ios = false, standalone = false, key = 'AQID' as string | null, withPush = true } = {}) {
  existing = null
  vi.stubGlobal('useApi', () => ({ api }))
  vi.stubGlobal('ref', ref)
  vi.stubGlobal('onMounted', onMounted)
  vi.stubGlobal('CreatorApiError', ApiErr)
  vi.stubGlobal('errMsg', () => 'api-error')
  vi.stubGlobal('useNuxtApp', () => ({ $i18n: { t: (k: string) => k } }))
  for (const k of ['isIos', 'isStandalone', 'currentPushSub', 'dropPush', 'b64urlToBytes'] as const) vi.stubGlobal(k, k === 'isIos' ? () => ios : k === 'isStandalone' ? () => standalone : k === 'currentPushSub' ? async () => existing : push[k])
  vi.stubGlobal('__NATIVE__', false)
  vi.stubGlobal('Notification', { get permission() { return permission }, requestPermission })
  if (withPush) vi.stubGlobal('PushManager', class {})
  else Reflect.deleteProperty(window, 'PushManager')
  Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: { getRegistration: async () => ({ pushManager: { subscribe, getSubscription: async () => existing } }) } })
  api.mockImplementation(async (path: string) => (path === '/push/config' ? { public_key: key } : undefined))
}

beforeEach(() => { permission = 'default'; api.mockReset(); subscribe.mockClear(); requestPermission.mockClear(); sub.unsubscribe.mockClear() })
afterEach(() => { vi.unstubAllGlobals(); Reflect.deleteProperty(navigator, 'serviceWorker') })

it('is unavailable when the server has no public key, and never asks for permission on load', async () => {
  env({ key: null })
  const p = usePush(); await p.init()
  expect(p.status.value).toBe('unavailable')
  expect(requestPermission).not.toHaveBeenCalled()
})

it('is unavailable without PushManager, and in the native shell', async () => {
  env({ withPush: false })
  let p = usePush(); await p.init()
  expect(p.status.value).toBe('unavailable')
  env(); vi.stubGlobal('__NATIVE__', true); api.mockClear()
  p = usePush(); await p.init()
  expect(p.status.value).toBe('unavailable')
  expect(api).not.toHaveBeenCalledWith('/push/config', expect.anything())
})

it('asks iOS users outside the installed app to add to home screen', async () => {
  env({ ios: true, standalone: false })
  const p = usePush(); await p.init()
  expect(p.status.value).toBe('needs-install')
  env({ ios: true, standalone: true })
  const q = usePush(); await q.init()
  expect(q.status.value).toBe('ready')
})

it('turning on requests permission, subscribes and registers with the server', async () => {
  env(); permission = 'granted'
  const p = usePush(); await p.init()
  expect(p.enabled.value).toBe(false)
  await p.set(true)
  expect(requestPermission).toHaveBeenCalledOnce()
  expect(subscribe).toHaveBeenCalledWith({ userVisibleOnly: true, applicationServerKey: new Uint8Array([1, 2, 3]) })
  expect(api).toHaveBeenCalledWith('/me/push-subscriptions', { method: 'POST', body: sub.toJSON(), noRedirect: true })
  expect(p.enabled.value).toBe(true)
})

it('denied permission shows the hint and does not subscribe', async () => {
  env(); permission = 'denied'
  const p = usePush(); await p.init()
  expect(p.denied.value).toBe(true)
  await p.set(true)
  expect(subscribe).not.toHaveBeenCalled()
  expect(p.enabled.value).toBe(false)
})

it('server failure rolls the browser subscription back and reports an error', async () => {
  env(); permission = 'granted'
  const p = usePush(); await p.init()
  api.mockRejectedValueOnce(new ApiErr('x'))
  await p.set(true)
  expect(sub.unsubscribe).toHaveBeenCalled()
  expect([p.enabled.value, p.error.value]).toEqual([false, 'api-error'])
})

it('turning off unsubscribes and deletes on the server', async () => {
  env(); permission = 'granted'; existing = sub
  const p = usePush(); await p.init()
  expect(p.enabled.value).toBe(true)
  await p.set(false)
  expect(api).toHaveBeenCalledWith('/me/push-subscriptions', { method: 'DELETE', body: { endpoint: sub.endpoint }, noRedirect: true })
  expect(sub.unsubscribe).toHaveBeenCalled()
  expect(p.enabled.value).toBe(false)
})

it('dropPush never throws, even when the server call or unsubscribe fails', async () => {
  env(); existing = sub
  const failing = vi.fn().mockRejectedValue(new Error('offline'))
  await expect(push.dropPush(failing)).resolves.toBeUndefined()
  expect(sub.unsubscribe).toHaveBeenCalled()
  Object.defineProperty(navigator, 'serviceWorker', { configurable: true, value: { getRegistration: async () => { throw new Error('boom') } } })
  await expect(push.dropPush(failing)).resolves.toBeUndefined()
  existing = null
  expect(await push.b64urlToBytes('AQID')).toEqual(new Uint8Array([1, 2, 3]))
})

it('PushToggle renders nothing when unavailable and a switch when ready', async () => {
  env({ key: null })
  vi.stubGlobal('usePush', usePush)
  let m = mount(PushToggle); await flush(); await flush()
  expect(m.el.querySelector('section')).toBeNull()
  m.unmount()
  env(); permission = 'granted'
  vi.stubGlobal('usePush', usePush)
  m = mount(PushToggle); await flush(); await flush(); await nextTick()
  const box = m.el.querySelector<HTMLInputElement>('input[type=checkbox]')!
  expect(box.checked).toBe(false)
  box.checked = true; box.dispatchEvent(new Event('change'))
  await flush(); await flush()
  expect(subscribe).toHaveBeenCalled()
  m.unmount()
  env({ ios: true })
  vi.stubGlobal('usePush', usePush)
  m = mount(PushToggle); await flush(); await flush()
  expect(m.el.querySelector('input')).toBeNull()
  expect(m.el.textContent).toContain('加入主畫面')
  m.unmount()
})
