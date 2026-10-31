import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { detectPwaContext, installPrompt, isIos, sendPwaEvent, usePwaContext } from '../app/composables/usePwaContext'

const IOS_SAFARI = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1'
const IOS_CHROME = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/120.0 Mobile/15E148 Safari/604.1'
const IOS_LINE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari Line/13.0.0'
const IOS_WEBVIEW = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148'
const ANDROID = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36'

const store = new Map<string, string>()
let ua = IOS_SAFARI
let standalone = false
beforeEach(() => {
  store.clear(); ua = IOS_SAFARI; standalone = false; installPrompt.value = null
  vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) })
  vi.stubGlobal('navigator', { get userAgent() { return ua }, maxTouchPoints: 0, get standalone() { return standalone } })
  vi.stubGlobal('matchMedia', () => ({ matches: false }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks() })

it('依 UA 與狀態判斷環境', () => {
  expect(detectPwaContext(IOS_SAFARI, false, false, false)).toBe('ios')
  expect(detectPwaContext(ANDROID, false, true, false)).toBe('android')
  expect(detectPwaContext(ANDROID, false, false, false)).toBeNull() // 沒攔到 beforeinstallprompt 就不顯示
  expect(detectPwaContext(IOS_LINE, false, false, false)).toBe('line')
  expect(detectPwaContext(ANDROID + ' Line/13.0', false, true, false)).toBe('line')
  expect(detectPwaContext(IOS_CHROME, false, false, false)).toBeNull()
  expect(detectPwaContext(IOS_WEBVIEW, false, false, false)).toBeNull()
  expect(detectPwaContext(IOS_SAFARI, true, false, false)).toBe('standalone')
  expect(detectPwaContext(IOS_SAFARI, false, false, true)).toBeNull() // 原生殼
})

it('iPadOS 以 Macintosh + 觸控判為 iOS', () => {
  expect(isIos('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)')).toBe(false)
  vi.stubGlobal('navigator', { userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', maxTouchPoints: 5 })
  expect(isIos()).toBe(true)
})

it('usePwaContext：稍後只關本次、不再提醒寫入偏好', () => {
  const a = usePwaContext()
  expect(a.context.value).toBe('ios')
  a.dismiss(false)
  expect(a.context.value).toBeNull()
  expect(store.has('ws_hint_dismissed')).toBe(false)
  a.dismiss(true)
  expect(store.get('ws_hint_dismissed')).toBe('1')
})

it('standalone 不顯示；storage 不可用時 dismiss 不丟例外', () => {
  standalone = true
  expect(usePwaContext().context.value).toBeNull()
  vi.stubGlobal('localStorage', { getItem: () => { throw new Error('denied') }, setItem: () => { throw new Error('denied') } })
  expect(() => usePwaContext().dismiss(true)).not.toThrow()
})

it('install 呼叫 prompt() 一次並清掉暫存事件，prompt 失敗也不丟例外', async () => {
  const prompt = vi.fn().mockRejectedValue(new Error('x'))
  installPrompt.value = { prompt }
  const p = usePwaContext()
  await p.install()
  await p.install()
  expect(prompt).toHaveBeenCalledTimes(1)
  expect(installPrompt.value).toBeNull()
})

it('sendPwaEvent：以 keepalive POST 到 /pwa-events，失敗靜默', () => {
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: 'http://api.test' } }))
  const f = vi.fn().mockRejectedValue(new Error('offline'))
  vi.stubGlobal('fetch', f)
  const e = { event: 'pwa_install_prompted', platform: 'ios', context: 'claim' } as const
  expect(() => sendPwaEvent(e)).not.toThrow()
  expect(f).toHaveBeenCalledWith('http://api.test/api/v1/pwa-events', expect.objectContaining({ method: 'POST', keepalive: true, body: JSON.stringify(e) }))
  vi.stubGlobal('useRuntimeConfig', () => { throw new Error('no nuxt context') })
  expect(() => sendPwaEvent(e)).not.toThrow()
})
