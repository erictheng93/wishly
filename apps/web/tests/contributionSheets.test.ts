// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ref, computed, onMounted, onBeforeMount, onBeforeUnmount, h, defineComponent, nextTick } from 'vue'
import ClaimSheet from '../app/components/guest/ClaimSheet.vue'
import FundSheet from '../app/components/guest/FundSheet.vue'
import GuestDialog from '../app/components/guest/Dialog.vue'
import { contributionErr, fmtPts, fmtTime, quickAmounts } from '../app/utils/crowdfund'
import { mount, input, submit, flush } from './vueHarness'

const api = vi.fn(), setToken = vi.fn(), done = vi.fn(), stale = vi.fn()
let mounted: ReturnType<typeof mount> | undefined
let currentToken: string | null = null
beforeEach(() => {
  currentToken = null
  for (const [name, value] of Object.entries({ ref, computed, onMounted, onBeforeMount, onBeforeUnmount,
    useGuest: () => ({ api, newKey: () => 'claim-key' }), useApi: () => ({ api }),
    useAuth: () => ({ fetchMe: async () => ({ default_claim_visibility: 'private' }) }),
    getPref: () => null, setPref: vi.fn(), getGuestToken: () => currentToken,
    setGuestToken: (token: string) => { currentToken = token; setToken(token); return true },
    getActiveAccess: () => 'list-access', fmtPts, fmtTime, contributionErr, quickAmounts,
    errMsg: (error: any) => error.detail,
  })) vi.stubGlobal(name, value)
})
afterEach(() => { mounted?.unmount(); mounted = undefined; vi.unstubAllGlobals(); vi.resetAllMocks() })
const NuxtLink = defineComponent({ setup: (_, { slots }) => () => h('a', {}, slots.default?.()) })
function claim(mine: any = null, online = true, loggedInName?: string, extra: Record<string, any> = {}) {
  mounted = mount(ClaimSheet, { item: { id: 'item', title: '禮物', qty_remaining: 3 }, mine, online, loggedInName,
    onDone: done, onStale: stale, ...extra }, { GuestDialog, NuxtLink }, { fmtPts, fmtTime })
  return mounted.el
}
async function fund(online = true) {
  api.mockImplementation((path: string) => path === '/wallet' ? Promise.resolve({ wallet: { balance: 1000 } }) : Promise.resolve({ id: 'contribution' }))
  mounted = mount(FundSheet, { item: { id: 'item', title: '禮物', remaining_points: 1000 }, online, slug: 'list', onDone: done, onStale: stale }, { GuestDialog, NuxtLink }, { fmtPts, fmtTime })
  await flush()
  return mounted.el
}
function deferred() {
  let resolve!: (value: any) => void
  let reject!: (error: any) => void
  const promise = new Promise((ok, no) => { resolve = ok; reject = no })
  return { promise, resolve, reject }
}

it('signed-in claim has no nickname/contact/email fields and sends only qty, note and visibility', async () => {
  const request = deferred(); api.mockReturnValue(request.promise)
  const el = claim(null, true, '阿明')
  expect(el.querySelector('#cn, #cc, #ce')).toBeNull()
  expect(el.textContent).toContain('以 阿明 的身分捐助')
  const plus = el.querySelector<HTMLButtonElement>('button[aria-label="增加"]')!
  plus.click(); await nextTick(); plus.click(); await nextTick()
  expect(plus.disabled).toBe(true)
  await submit(el)
  expect(el.querySelector<HTMLButtonElement>('button.g-btn')!.disabled).toBe(true)
  expect(el.textContent).toContain('送出中')
  expect(api.mock.calls[0]).toEqual(['/items/item/claims', {
    method: 'POST', headers: { 'Idempotency-Key': 'claim-key' }, keepToken: true,
    body: { qty: 3, note: undefined, visibility: 'private' }, // 帶入帳號預設可見性
  }])
  request.resolve({ id: 'claim' }); await flush()
  expect(done).toHaveBeenCalledWith({ id: 'claim' })
  expect(setToken).not.toHaveBeenCalled()
})

it('claim 401 asks the page to send the user to login instead of showing an error', async () => {
  const login = vi.fn()
  api.mockRejectedValue({ status: 401, code: 'UNAUTHORIZED', detail: '請先登入' })
  const el = claim(null, true, '阿明', { onLogin: login })
  await submit(el); await flush()
  expect(login).toHaveBeenCalledOnce()
  expect(el.querySelector('[role="alert"]')).toBeNull()
})

it('legacy guest editing can lower but never raise quantity above the current claim', () => {
  const el = claim({ id: 'old', qty: 2 }, true)
  expect(el.querySelector<HTMLButtonElement>('button[aria-label="增加"]')!.disabled).toBe(true)
  expect(el.querySelector<HTMLButtonElement>('button[aria-label="減少"]')!.disabled).toBe(false)
})

it('claim server validation renders field error and unlocks submission', async () => {
  api.mockRejectedValue({ code: 'VALIDATION_FAILED', data: { errors: [{ pointer: '/note', detail: '留言太長' }] } })
  const el = claim(null, true, '阿明')
  await submit(el); await flush()
  expect(el.textContent).toContain('留言太長')
  expect(el.querySelector<HTMLButtonElement>('button.g-btn')!.disabled).toBe(false)
  expect(done).not.toHaveBeenCalled()
})

it('claim inventory conflict lowers quantity to remaining and emits stale inventory', async () => {
  api.mockRejectedValue({ code: 'ITEM_FULLY_CLAIMED', data: { remaining: 1 } })
  const el = claim(null, true, '阿明')
  el.querySelector<HTMLButtonElement>('button[aria-label="增加"]')!.click(); await nextTick()
  await submit(el); await flush()
  expect(stale).toHaveBeenCalledWith(1)
  expect(el.querySelector('.g-step b')!.textContent).toBe('1')
  expect(el.querySelector<HTMLButtonElement>('button[aria-label="增加"]')!.disabled).toBe(true)
})

it('editing claim sends only quantity and note to the existing claim', async () => {
  api.mockResolvedValue({ id: 'existing' })
  const el = claim({ id: 'existing', qty: 2, note: '原留言' })
  await input(el, '#cm', '新留言'); await submit(el); await flush()
  expect(api.mock.calls[0]).toEqual(['/claims/existing', { method: 'PATCH', body: { qty: 2, note: '新留言' } }])
  expect(done).toHaveBeenCalledWith({ id: 'existing' })
})

it.each(['0', '-1', '1.5', '1001', ''])('fund rejects invalid amount %s before sending', async amount => {
  const el = await fund()
  api.mockClear()
  await input(el, '#fp', amount); await submit(el)
  expect(api).not.toHaveBeenCalled()
  expect(el.querySelector('[role="alert"]')!.textContent).toContain('整數點數')
  expect(done).not.toHaveBeenCalled()
})

it('fund submits trimmed message and list access; busy state prevents another click and resolves to done', async () => {
  const el = await fund(), request = deferred()
  api.mockClear(); api.mockReturnValue(request.promise)
  await input(el, '#fp', '300'); await input(el, '#fm', '  生日快樂  ')
  el.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click(); await nextTick()
  await submit(el)
  const button = el.querySelector<HTMLButtonElement>('button.g-btn:not(.ghost)')!
  expect(button.disabled).toBe(true)
  expect(el.querySelector<HTMLInputElement>('#fp')!.readOnly).toBe(true)
  button.click()
  expect(api).toHaveBeenCalledTimes(1)
  expect(api.mock.calls[0]).toEqual(['/items/item/contributions', {
    method: 'POST', headers: { 'Idempotency-Key': expect.any(String), 'X-List-Access': 'list-access' }, noRedirect: true,
    body: { points: 300, message: '生日快樂', is_anonymous: true, visibility: 'private' },
  }])
  request.resolve({ id: 'funded', points: 300 }); await flush()
  expect(done).toHaveBeenCalledWith({ id: 'funded', points: 300 })
  expect(button.disabled).toBe(false)
})

it('fund network retry keeps idempotency key; deterministic inventory error renews key and adjusts points', async () => {
  const el = await fund()
  api.mockClear(); api.mockRejectedValueOnce({ code: 'NETWORK' }).mockRejectedValueOnce({ code: 'CROWDFUND_TARGET_EXCEEDED', body: { remaining_points: 100 } }).mockResolvedValueOnce({ id: 'funded' })
  await submit(el); await flush()
  const firstKey = api.mock.calls[0]![1].headers['Idempotency-Key']
  await submit(el); await flush()
  expect(api.mock.calls[1]![1].headers['Idempotency-Key']).toBe(firstKey)
  expect(stale).toHaveBeenCalledOnce()
  expect(el.querySelector<HTMLInputElement>('#fp')!.value).toBe('100')
  await submit(el); await flush()
  expect(api.mock.calls[2]![1].headers['Idempotency-Key']).not.toBe(firstKey)
  expect(api.mock.calls[2]![1].body.points).toBe(100)
  expect(done).toHaveBeenCalledWith({ id: 'funded' })
})

it('fund unauthorized error shows login recovery without emitting success', async () => {
  const el = await fund()
  api.mockRejectedValue({ code: 'UNAUTHORIZED' })
  await submit(el); await flush()
  expect(el.querySelector('[role="alert"]')!.textContent).toContain('重新登入')
  expect(done).not.toHaveBeenCalled()
  expect(el.querySelector<HTMLButtonElement>('button.g-btn:not(.ghost)')!.disabled).toBe(false)
})

it('offline sheets disable their submit buttons', async () => {
  const el = claim(null, false)
  expect(el.querySelector<HTMLButtonElement>('button.g-btn')!.disabled).toBe(true)
  mounted!.unmount()
  const fundEl = await fund(false)
  expect(fundEl.querySelector<HTMLButtonElement>('button.g-btn:not(.ghost)')!.disabled).toBe(true)
})

it('fund visibility defaults to the account setting and the user can change it before submitting', async () => {
  const el = await fund()
  const select = el.querySelector<HTMLSelectElement>('#fv')!
  expect(select.value).toBe('private')
  select.value = 'public'; select.dispatchEvent(new Event('change', { bubbles: true })); await nextTick()
  api.mockClear(); api.mockResolvedValue({ id: 'funded' })
  await input(el, '#fp', '100'); await submit(el); await flush()
  expect(api.mock.calls[0][1].body).toMatchObject({ points: 100, visibility: 'public' })
})

it('fund without a loaded account default hides the picker and lets the server apply the default', async () => {
  vi.stubGlobal('useAuth', () => ({ fetchMe: () => Promise.reject(new Error('offline')) }))
  const el = await fund()
  expect(el.querySelector('#fv')).toBeNull()
  api.mockClear(); api.mockResolvedValue({ id: 'funded' })
  await input(el, '#fp', '100'); await submit(el); await flush()
  expect(api.mock.calls[0][1].body.visibility).toBeUndefined()
})
