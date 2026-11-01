// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { defineComponent, h, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import CodeLogin from '../app/components/creator/CodeLogin.vue'
import { mount, submit, flush } from './vueHarness'

const requestLoginCode = vi.fn(), verifyLoginCode = vi.fn(), done = vi.fn()
// 只留 v-model 與 complete：驗證完成時機是這裡要測的行為
const CreatorOtpInput = defineComponent({
  props: { modelValue: String }, emits: ['update:modelValue', 'complete'],
  setup(_, { emit, expose }) {
    expose({ focus: () => {} })
    return () => h('input', { class: 'otp', onInput: (e: Event) => {
      const v = (e.target as HTMLInputElement).value
      emit('update:modelValue', v)
      if (v.length === 6) emit('complete', v)
    } })
  },
})
const Turnstile = defineComponent({
  props: { modelValue: String }, emits: ['update:modelValue'],
  setup(_, { emit }) { return () => h('button', { class: 'ts', type: 'button', onClick: () => emit('update:modelValue', 'token') }) },
})
const SupportLink = defineComponent({ setup: () => () => null })
let mounted: ReturnType<typeof mount> | undefined

beforeEach(() => {
  for (const [name, value] of Object.entries({ ref, watch, nextTick, onBeforeUnmount,
    useI18n: () => ({ t: (k: string) => k }), errMsg: (e: any) => e.detail,
    useAuth: () => ({ requestLoginCode, verifyLoginCode }),
  })) vi.stubGlobal(name, value)
})
afterEach(() => { mounted?.unmount(); mounted = undefined; vi.unstubAllGlobals(); vi.resetAllMocks() })

async function open(email = ' me@example.test ') {
  mounted = mount(CodeLogin, { email, onDone: done }, { CreatorOtpInput, Turnstile, SupportLink })
  await nextTick()
  return mounted.el
}
async function typeCode(el: Element, code: string) {
  const f = el.querySelector<HTMLInputElement>('.otp')!
  f.value = code
  f.dispatchEvent(new Event('input', { bubbles: true }))
  await flush(); await flush()
}

it('carries the typed email, binds the challenge and signs in once the code is complete', async () => {
  requestLoginCode.mockResolvedValue({ challenge_id: 'c1', resend_after: 60 })
  verifyLoginCode.mockResolvedValue({ id: 'u' })
  const el = await open()
  await submit(el); await flush(); await flush()
  expect(requestLoginCode).toHaveBeenCalledWith('me@example.test', undefined)
  await typeCode(el, '123456')
  expect(verifyLoginCode).toHaveBeenCalledTimes(1)
  expect(verifyLoginCode).toHaveBeenCalledWith('me@example.test', '123456', 'c1')
  expect(done).toHaveBeenCalledTimes(1)
})

it('a wrong code stays on the code step, clears the input and does not finish', async () => {
  requestLoginCode.mockResolvedValue({ challenge_id: 'c1' })
  verifyLoginCode.mockRejectedValue({ code: 'OTP_INVALID', status: 400 })
  const el = await open()
  await submit(el); await flush(); await flush()
  await typeCode(el, '000000')
  expect(el.querySelector('[role="alert"]')?.textContent).toContain('auth.register.otpInvalid')
  expect(el.querySelector('.otp')).not.toBeNull()
  expect(el.querySelector<HTMLButtonElement>('button.primary')!.disabled).toBe(true) // 舊碼已清空，要重新輸入
  expect(done).not.toHaveBeenCalled()
})

it('asks for captcha when the backend requires it, then resends with the token', async () => {
  requestLoginCode.mockRejectedValueOnce({ code: 'CAPTCHA_REQUIRED', status: 403 })
    .mockResolvedValueOnce({ challenge_id: 'c2' })
  const el = await open()
  await submit(el); await flush(); await flush()
  expect(el.querySelector('[role="alert"]')?.textContent).toContain('auth.register.captcha')
  el.querySelector<HTMLButtonElement>('.ts')!.click()
  await flush(); await flush()
  expect(requestLoginCode).toHaveBeenLastCalledWith('me@example.test', 'token')
  expect(el.querySelector('.otp')).not.toBeNull()
})

it('shows rate limits on both steps without leaving the flow', async () => {
  requestLoginCode.mockRejectedValueOnce({ status: 429 })
  const el = await open()
  await submit(el); await flush(); await flush()
  expect(el.querySelector('[role="alert"]')?.textContent).toContain('auth.register.rate')
  expect(el.querySelector('.otp')).toBeNull()

  requestLoginCode.mockResolvedValue({ challenge_id: 'c1' })
  verifyLoginCode.mockRejectedValue({ status: 429 })
  await submit(el); await flush(); await flush()
  await typeCode(el, '123456')
  expect(el.querySelector('[role="alert"]')?.textContent).toContain('auth.register.tooMany')
  expect(done).not.toHaveBeenCalled()
})

it('surfaces account errors such as a blocked account as the backend worded them', async () => {
  requestLoginCode.mockResolvedValue({ challenge_id: 'c1' })
  verifyLoginCode.mockRejectedValue({ code: 'ACCOUNT_BLOCKED', status: 403, detail: '帳號已停權' })
  const el = await open()
  await submit(el); await flush(); await flush()
  await typeCode(el, '123456')
  expect(el.querySelector('[role="alert"]')?.textContent).toContain('帳號已停權')
  expect(done).not.toHaveBeenCalled()
})

it('verifies once when completion fires twice in one tick', async () => {
  let finish!: (v: any) => void
  requestLoginCode.mockResolvedValue({ challenge_id: 'c1' })
  verifyLoginCode.mockReturnValue(new Promise(resolve => { finish = resolve }))
  const el = await open()
  await submit(el); await flush(); await flush()
  // 同一拍連續完成：Vue 還沒停用按鈕，只能靠 busy 擋
  const f = el.querySelector<HTMLInputElement>('.otp')!
  f.value = '123456'
  f.dispatchEvent(new Event('input', { bubbles: true }))
  f.dispatchEvent(new Event('input', { bubbles: true }))
  await flush(); await flush()
  expect(verifyLoginCode).toHaveBeenCalledTimes(1)
  finish({ id: 'u' }); await flush(); await flush()
  expect(done).toHaveBeenCalledTimes(1)
})

it('resend waits for the cooldown and uses the new challenge', async () => {
  vi.useFakeTimers()
  try {
    requestLoginCode.mockResolvedValueOnce({ challenge_id: 'old', resend_after: 2 }).mockResolvedValueOnce({ challenge_id: 'new', resend_after: 60 })
    verifyLoginCode.mockResolvedValue({ id: 'u' })
    const el = await open()
    await submit(el); await flush(); await flush()
    const resend = () => [...el.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === '重新寄送')
    expect(resend()).toBeUndefined()
    await vi.advanceTimersByTimeAsync(2000); await flush()
    resend()!.click()
    await flush(); await flush()
    await typeCode(el, '123456')
    expect(verifyLoginCode).toHaveBeenCalledWith('me@example.test', '123456', 'new')
  } finally { vi.useRealTimers() }
})

it('changing the email verifies against the new address', async () => {
  requestLoginCode.mockResolvedValueOnce({ challenge_id: 'old' }).mockResolvedValueOnce({ resend_after: 60 })
  verifyLoginCode.mockResolvedValue({ id: 'u' })
  const el = await open()
  await submit(el); await flush(); await flush()
  ;[...el.querySelectorAll('button')].find(b => b.textContent === '更換 Email')!.click()
  await flush()
  const f = el.querySelector<HTMLInputElement>('input[type="email"]')!
  f.value = 'other@example.test'; f.dispatchEvent(new Event('input', { bubbles: true }))
  await flush(); await submit(el); await flush(); await flush()
  await typeCode(el, '123456')
  expect(verifyLoginCode).toHaveBeenCalledWith('other@example.test', '123456', undefined)
})
