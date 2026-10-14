// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ref, onMounted, onBeforeMount, onBeforeUnmount, nextTick } from 'vue'
import ReportSheet from '../app/components/guest/ReportSheet.vue'
import GuestDialog from '../app/components/guest/Dialog.vue'
import { ApiError } from '../app/composables/useGuest'
import { reportErrMsg } from '../app/utils/errors'
import { mount, input, submit, flush } from './vueHarness'

const api = vi.fn(), sent = vi.fn(), close = vi.fn()
let mounted: ReturnType<typeof mount> | undefined
let callbacks: Record<string, (...args: any[]) => void>
const turnstile = { render: vi.fn(), reset: vi.fn(), remove: vi.fn() }
const scriptSelector = 'script[src="https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit"]'

beforeEach(() => {
  callbacks = {}
  turnstile.render.mockImplementation((_box, options) => { callbacks = options; return 'widget' })
  vi.stubGlobal('turnstile', turnstile)
  for (const [name, value] of Object.entries({ ref, onMounted, onBeforeMount, onBeforeUnmount,
    useGuest: () => ({ api }), useRuntimeConfig: () => ({ public: { turnstileSiteKey: 'test-site-key' } }), reportErrMsg,
  })) vi.stubGlobal(name, value)
})
afterEach(() => {
  mounted?.unmount(); mounted = undefined
  document.querySelector(scriptSelector)?.remove()
  vi.unstubAllGlobals(); vi.restoreAllMocks(); vi.resetAllMocks()
})
async function report(siteKey = 'test-site-key') {
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { turnstileSiteKey: siteKey } }))
  mounted = mount(ReportSheet, { slug: 'public-list', onSent: sent, onClose: close }, { GuestDialog })
  await flush()
  return mounted.el
}
const button = (el: Element) => el.querySelector<HTMLButtonElement>('button.g-btn')!

it('report refuses an unverified form even when submission bypasses its disabled button', async () => {
  const el = await report()
  expect(button(el).disabled).toBe(true)
  await input(el, '#rd', '疑似詐騙')
  await submit(el)
  expect(api).not.toHaveBeenCalled()
  expect(sent).not.toHaveBeenCalled()
  expect(el.querySelector('[role="alert"]')?.textContent).toContain('請先完成驗證')
  callbacks.callback!('verified-token'); await nextTick()
  expect(button(el).disabled).toBe(false)
  expect(el.querySelector('[role="alert"]')).toBeNull()
})

it('report keeps one request pending across rapid submissions and emits sent only after success', async () => {
  let complete!: () => void
  api.mockReturnValue(new Promise<void>(resolve => { complete = resolve }))
  const el = await report()
  callbacks.callback!('verified-token'); await nextTick()
  el.querySelector<HTMLInputElement>('input[value="personal_info"]')!.click(); await nextTick()
  await input(el, '#rd', '頁面公開了私人電話')
  // Both clicks happen before Vue patches disabled: the handler must guard busy too.
  button(el).click(); button(el).click(); await nextTick()
  expect(api).toHaveBeenCalledTimes(1)
  expect(api).toHaveBeenCalledWith('/public/wishlists/public-list/reports', { method: 'POST', body: {
    reason: 'personal_info', detail: '頁面公開了私人電話', item_id: null, turnstile_token: 'verified-token',
  } })
  expect(button(el).disabled).toBe(true)
  expect(button(el).textContent).toContain('送出中')
  expect(sent).not.toHaveBeenCalled()
  await submit(el)
  expect(api).toHaveBeenCalledTimes(1)
  complete(); await flush()
  expect(sent).toHaveBeenCalledOnce()
  expect(button(el).disabled).toBe(false)
})

it.each([
  ['expired-callback', '驗證已過期'], ['timeout-callback', '驗證逾時'], ['error-callback', '人機驗證發生錯誤'],
])('%s revokes a previously valid token and requires fresh verification', async (event, message) => {
  const el = await report()
  callbacks.callback!('old-token'); await nextTick()
  callbacks[event]!(); await nextTick()
  expect(button(el).disabled).toBe(true)
  expect(el.querySelector('[role="alert"]')!.textContent).toContain(message)
  await submit(el)
  expect(api).not.toHaveBeenCalled()
  callbacks.callback!('fresh-token'); await nextTick()
  expect(button(el).disabled).toBe(false)
  expect(el.querySelector('[role="alert"]')).toBeNull()
})

it('captcha rejection preserves entered detail, resets verification, and allows a verified retry', async () => {
  api.mockRejectedValueOnce(new ApiError(403, 'FORBIDDEN', '人機驗證失敗，請重新整理後再試'))
    .mockResolvedValueOnce({ id: 'report-id', status: 'open' })
  const el = await report()
  await input(el, '#rd', '保留這段說明')
  callbacks.callback!('old-token'); await nextTick()
  button(el).click(); await flush()
  expect(el.querySelector('[role="alert"]')!.textContent).toBe('人機驗證失敗，請重新整理後再試')
  expect(el.querySelector<HTMLTextAreaElement>('#rd')!.value).toBe('保留這段說明')
  expect(button(el).disabled).toBe(true)
  expect(turnstile.reset).toHaveBeenCalledWith('widget')
  expect(sent).not.toHaveBeenCalled()
  callbacks.callback!('new-token'); await nextTick()
  button(el).click(); await flush()
  expect(api.mock.calls[1]![1].body.turnstile_token).toBe('new-token')
  expect(sent).toHaveBeenCalledOnce()
  mounted!.unmount(); mounted = undefined
  expect(turnstile.remove).toHaveBeenCalledWith('widget')
})

it('rate limit error displays recovery without consuming the captcha token', async () => {
  api.mockRejectedValue({ code: 'RATE_LIMITED', status: 429 })
  const el = await report()
  callbacks.callback!('verified-token'); await nextTick()
  button(el).click(); await flush()
  expect(el.querySelector('[role="alert"]')!.textContent).toContain('檢舉太頻繁')
  expect(button(el).disabled).toBe(false)
  expect(turnstile.reset).not.toHaveBeenCalled()
  expect(sent).not.toHaveBeenCalled()
})

it('local bypass submits an empty optional detail as null without loading a widget', async () => {
  api.mockResolvedValue(undefined)
  const el = await report('')
  button(el).click(); await flush()
  expect(api).toHaveBeenCalledWith('/public/wishlists/public-list/reports', { method: 'POST', body: {
    reason: 'scam', detail: null, item_id: null, turnstile_token: 'dev-bypass',
  } })
  expect(turnstile.render).not.toHaveBeenCalled()
  expect(sent).toHaveBeenCalledOnce()
})

it('widget loading failure leaves submission disabled and reports how to recover', async () => {
  vi.stubGlobal('turnstile', undefined)
  // Replace only the external script load; keep the component's load/error listeners real.
  let script!: HTMLScriptElement
  vi.spyOn(document.head, 'append').mockImplementation(node => { script = node as HTMLScriptElement })
  const el = await report()
  script.dispatchEvent(new Event('error')); await flush()
  expect(el.querySelector('[role="alert"]')!.textContent).toContain('無法載入人機驗證')
  expect(button(el).disabled).toBe(true)
  expect(api).not.toHaveBeenCalled()
})

it('cancel closes the report without sending anything', async () => {
  const el = await report()
  el.querySelector<HTMLButtonElement>('button[type="button"]')!.click()
  expect(close).toHaveBeenCalledOnce()
  expect(api).not.toHaveBeenCalled()
  expect(sent).not.toHaveBeenCalled()
})
