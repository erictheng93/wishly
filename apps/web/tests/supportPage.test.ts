// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, h, nextTick, onMounted, ref, watch } from 'vue'
import Support from '../app/pages/support.vue'
import AdminSupport from '../app/pages/admin/support.vue'
import { flush, input, mount, submit } from './vueHarness'

const api = vi.fn()
const fetchMe = vi.fn()
const user = ref<any>(null)
const hint = ref<any>(null)
const runtime = { public: { turnstileSiteKey: '' } }
const route = { query: {} as Record<string, any> }
const mounted: ReturnType<typeof mount>[] = []
const stubs = {
  CreatorHeader: { template: '<header />' },
  SupportLink: { template: '<a data-testid="support-link" />' },
  Turnstile: { props: ['modelValue'], emits: ['update:modelValue'], setup: (_: any, { emit }: any) => () => h('button', { type: 'button', class: 'ts', onClick: () => emit('update:modelValue', 'tok') }) },
  UiIcon: { template: '<i />' },
  CreatorConfirm: {
    props: ['open', 'title', 'text', 'error', 'disabled', 'busy'], emits: ['ok', 'close'],
    setup: (p: any, { slots, emit }: any) => () => p.open ? h('div', { class: 'confirm' }, [p.text, p.error, slots.default?.(), h('button', { class: 'ok', disabled: p.disabled || p.busy, onClick: () => emit('ok') }, 'ok')]) : null,
  },
}
const settle = async () => { await flush(); await flush(); await nextTick() }
function show(c: any) { const m = mount(c, {}, stubs, { fmtTime: (v: any) => v }); mounted.push(m); return m.el }
class ApiErr extends Error { constructor(public code: string, msg: string) { super(msg) } }
beforeEach(() => {
  user.value = null; hint.value = null; route.query = {}; runtime.public.turnstileSiteKey = ''
  fetchMe.mockImplementation(async () => user.value)
  for (const [k, v] of Object.entries({ computed, nextTick, onMounted, ref, watch, definePageMeta: () => {}, useHead: () => {}, useSeoMeta: () => {}, useApi: () => ({ api }), useAuth: () => ({ user, fetchMe }), useAuthHint: () => hint, useRoute: () => route, useRuntimeConfig: () => runtime, errMsg: (e: any) => e.message, fmtTime: (v: any) => v, hasStaffRole: (u: any, r: string) => !!u?.staff_roles?.some((x: string) => x === r || x === 'admin') })) vi.stubGlobal(k, v)
})
afterEach(() => { mounted.splice(0).forEach(m => m.unmount()); vi.unstubAllGlobals(); api.mockReset(); fetchMe.mockReset() })

it('未登入：不抓 /me、顯示 Email 與 Turnstile，送到公開端點且不列出我的請求', async () => {
  api.mockResolvedValue({ id: 'x', status: 'open' })
  const el = show(Support); await settle()
  expect(fetchMe).not.toHaveBeenCalled()
  expect(el.querySelector('[data-testid="email"]')).not.toBeNull()
  expect(el.textContent).not.toContain('我的客服請求')
  await submit(el)
  expect(el.querySelector('[data-testid="err"]')?.textContent).toBe('請填寫問題描述')
  await input(el, '[data-testid="body"]', '我被停權了')
  await submit(el)
  expect(el.querySelector('[data-testid="err"]')?.textContent).toBe('請填寫聯絡 Email')
  await input(el, '[data-testid="email"]', ' a@example.com ')
  el.querySelector<HTMLButtonElement>('.ts')!.click(); await settle()
  await submit(el); await settle()
  expect(api).toHaveBeenCalledWith('/public/support-requests', { method: 'POST', noRedirect: true, body: { email: 'a@example.com', category: 'other', body: '我被停權了', turnstile_token: 'tok' } })
  expect(el.querySelector('[data-testid="sent"]')?.textContent).toContain('以 Email 回覆')
})

it('未登入有 site key 但尚無 Turnstile token：不呼叫 api、顯示請先驗證', async () => {
  runtime.public.turnstileSiteKey = 'site-key'
  const el = show(Support); await settle()
  await input(el, '[data-testid="email"]', 'a@example.com')
  await input(el, '[data-testid="body"]', '求助')
  await submit(el); await settle()
  expect(api).not.toHaveBeenCalled()
  expect(el.querySelector('[data-testid="err"]')?.textContent).toBe('請先完成人機驗證後再試')
})

it('未登入沒有 site key（本機）：無 token 仍會送出', async () => {
  api.mockResolvedValue({ id: 'x', status: 'open' })
  const el = show(Support); await settle()
  await input(el, '[data-testid="email"]', 'a@example.com')
  await input(el, '[data-testid="body"]', '求助')
  await submit(el); await settle()
  expect(api).toHaveBeenCalledTimes(1)
})

it('未登入送出失敗顯示錯誤（含上限）', async () => {
  api.mockRejectedValue(new ApiErr('SUPPORT_OPEN_LIMIT', '尚未回覆的客服請求太多'))
  const el = show(Support); await settle()
  await input(el, '[data-testid="email"]', 'a@example.com')
  await input(el, '[data-testid="body"]', '求助')
  await submit(el); await settle()
  expect(el.querySelector('[data-testid="err"]')?.textContent).toContain('太多')
})

it('已登入：預填類別與清單、列出我的請求與回覆，送出後加到最前面', async () => {
  hint.value = true
  fetchMe.mockImplementation(async () => (user.value = { id: 'u' }))
  route.query = { category: 'wishlist', wishlist: '0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b' }
  api.mockImplementation(async (url: string, opts: any) => opts?.method === 'POST'
    ? { id: 'n', category: 'wishlist', body: '清單被下架', status: 'open', created_at: 't2' }
    : { data: [{ id: 'o', category: 'account', body: '舊問題', status: 'closed', reply: '已處理好了', handled_at: 't1', created_at: 't0' }] })
  const el = show(Support); await settle()
  expect(el.querySelector('[data-testid="email"]')).toBeNull()
  expect(el.querySelector<HTMLSelectElement>('[data-testid="category"]')!.value).toBe('wishlist')
  expect(el.querySelector('[data-testid="wishlist-ref"]')).not.toBeNull()
  expect(el.textContent).toContain('已處理好了')
  expect(el.textContent).toContain('已回覆')
  await input(el, '[data-testid="body"]', '清單被下架')
  await submit(el); await settle()
  expect(api).toHaveBeenCalledWith('/me/support-requests', { method: 'POST', body: { category: 'wishlist', body: '清單被下架', wishlist_id: '0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b' } })
  expect(el.querySelectorAll('article')[0].textContent).toContain('清單被下架')
  expect(el.querySelector('[data-testid="sent"]')).not.toBeNull()
})

it('已登入且沒有請求時顯示空狀態', async () => {
  hint.value = true
  fetchMe.mockImplementation(async () => (user.value = { id: 'u' }))
  api.mockResolvedValue({ data: [] })
  const el = show(Support); await settle()
  expect(el.querySelector('[data-testid="empty"]')).not.toBeNull()
})

it('後台：匿名工單顯示 mailto，只有 support / admin 看得到回覆按鈕，回覆後結案', async () => {
  const rows = [
    { id: 'm', kind: 'member', status: 'open', category: 'account', body: '會員', user: { id: 'u1', handle: 'h', display_name: '會員甲' }, contact_email: null },
    { id: 'g', kind: 'guest', status: 'open', category: 'other', body: '訪客', user: null, contact_email: 'g@example.com' },
  ]
  api.mockImplementation(async (url: string, opts: any) => opts?.method === 'PATCH' ? { ...rows[1], status: 'closed', reply: '已回信' } : { data: rows.map(r => ({ ...r })) })
  user.value = { staff_roles: ['finance'] }
  let el = show(AdminSupport); await settle()
  expect(el.querySelector('[data-kind="guest"] [data-testid="mailto"]')?.getAttribute('href')).toContain('mailto:g@example.com')
  expect(el.querySelector('[data-kind="member"] [data-testid="mailto"]')).toBeNull()
  expect([...el.querySelectorAll('button')].some(b => b.textContent?.includes('回覆並結案'))).toBe(false)

  user.value = { staff_roles: ['support'] }
  el = show(AdminSupport); await settle()
  ;[...el.querySelectorAll<HTMLButtonElement>('[data-kind="guest"] button')].find(b => b.textContent?.includes('回覆並結案'))!.click(); await settle()
  expect(el.querySelector('.confirm')?.textContent).toContain('g@example.com')
  expect(el.querySelector<HTMLButtonElement>('.confirm .ok')!.disabled).toBe(true)
  await input(el, '.confirm textarea', '已回信')
  el.querySelector<HTMLButtonElement>('.confirm .ok')!.click(); await settle()
  expect(api).toHaveBeenCalledWith('/admin/support-requests/g', { method: 'PATCH', body: { reply: '已回信' } })
  expect(el.querySelector('[data-kind="guest"]')?.getAttribute('data-status')).toBe('closed')
})
