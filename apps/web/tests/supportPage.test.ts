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
  NuxtLink: { props: ['to'], setup: (p: any, { slots, attrs }: any) => () => h('a', { ...attrs, href: typeof p.to === 'string' ? p.to : `${p.to.path}?${new URLSearchParams(p.to.query)}` }, slots.default?.()) },
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

it('後台：狀態篩選三個選項、預設待回覆，切到全部帶 status=all 並標示每列狀態', async () => {
  api.mockImplementation(async (_: string, opts: any) => opts.query.status === 'all'
    ? { data: [{ id: 'c', kind: 'guest', status: 'closed', category: 'other', body: 'x', user: null, contact_email: 'a@example.com', reply: '好了', handled_at: 't', handled_by: { id: 's', display_name: '客服甲' } },
      { id: 'o', kind: 'guest', status: 'open', category: 'other', body: 'y', user: null, contact_email: 'b@example.com' }] }
    : { data: [] })
  user.value = { staff_roles: ['support'] }
  const el = show(AdminSupport); await settle()
  const sel = el.querySelector<HTMLSelectElement>('[data-testid="status"]')!
  expect([...sel.options].map(o => o.value)).toEqual(['all', 'open', 'closed'])
  expect([...sel.options].map(o => o.textContent)).toEqual(['全部', '待回覆', '已結案'])
  expect(sel.value).toBe('open')
  expect(api).toHaveBeenLastCalledWith('/admin/support-requests', { query: { status: 'open', limit: 20, cursor: undefined } })
  sel.value = 'all'; sel.dispatchEvent(new Event('change')); await settle()
  expect(api).toHaveBeenLastCalledWith('/admin/support-requests', { query: { status: 'all', limit: 20, cursor: undefined } })
  expect([...el.querySelectorAll('[data-testid="row-status"]')].map(b => b.textContent)).toEqual(['已結案', '待回覆'])
  expect(el.querySelector('[data-status="closed"]')?.textContent).toContain('客服甲')
  expect([...el.querySelectorAll('[data-status="open"] button')].some(b => b.textContent?.includes('回覆並結案'))).toBe(true)
  expect([...el.querySelectorAll('[data-status="closed"] button')].some(b => b.textContent?.includes('回覆並結案'))).toBe(false)
})

it('後台：?status=all 直接開全部檢視', async () => {
  route.query = { status: 'all' }
  api.mockResolvedValue({ data: [] })
  user.value = { staff_roles: ['support'] }
  const el = show(AdminSupport); await settle()
  expect(el.querySelector<HTMLSelectElement>('[data-testid="status"]')!.value).toBe('all')
  expect(api).toHaveBeenCalledWith('/admin/support-requests', { query: { status: 'all', limit: 20, cursor: undefined } })
})

it('後台：顯示停權 / 凍結與清單、儲值脈絡，連結依角色顯示', async () => {
  const rows = [{
    id: 'm', kind: 'member', status: 'open', category: 'wishlist', body: '申訴', contact_email: null,
    user: { id: 'u1', handle: 'h', display_name: '會員甲', blocked: true, wallet: { id: 'w', status: 'frozen' } },
    wishlist_id: 'l1', wishlist: { id: 'l1', title: '生日清單', slug: 'abcdefghij', moderation_status: 'hidden' },
    topup_id: 't1', topup: { id: 't1', merchant_trade_no: 'WS123', status: 'disputed', user_id: 'u1' },
  }, { id: 'n', kind: 'member', status: 'open', category: 'other', body: '一般', contact_email: null,
    user: { id: 'u2', handle: null, display_name: '會員乙', blocked: false, wallet: null }, wishlist_id: null, wishlist: null, topup_id: null, topup: null }]
  api.mockResolvedValue({ data: rows })
  const q = (el: Element, s: string) => el.querySelector(`[data-testid="${s}"]`)

  // support：只看得到標記與文字，沒有進不去的連結
  user.value = { staff_roles: ['support'] }
  let el = show(AdminSupport); await settle()
  expect(q(el, 'blocked')).not.toBeNull()
  expect(q(el, 'frozen')).not.toBeNull()
  expect(q(el, 'wl-hidden')).not.toBeNull()
  expect(q(el, 'wishlist')?.textContent).toContain('生日清單')
  expect(q(el, 'topup')?.textContent).toContain('WS123')
  for (const s of ['user-reports', 'user-topups', 'wl-reports', 'topup-link']) expect(q(el, s)).toBeNull()
  // 沒停權、沒錢包的會員不顯示標記
  expect(el.querySelectorAll('[data-testid="blocked"], [data-testid="frozen"]').length).toBe(2)

  // moderator：檢舉頁對象檢視（已處置看 actioned）；沒有金流連結
  user.value = { staff_roles: ['moderator'] }
  el = show(AdminSupport); await settle()
  expect(q(el, 'user-reports')?.getAttribute('href')).toBe('/admin/reports?status=actioned&view=rows&target_kind=user&target_id=u1&target_label=%E6%9C%83%E5%93%A1%E7%94%B2')
  expect(q(el, 'wl-reports')?.getAttribute('href')).toContain('target_kind=wishlist&target_id=l1')
  expect(q(el, 'user-topups')).toBeNull()
  expect(q(el, 'topup-link')).toBeNull()

  // finance：金流交易明細依使用者篩選；沒有檢舉連結
  user.value = { staff_roles: ['finance'] }
  el = show(AdminSupport); await settle()
  expect(q(el, 'topup-link')?.getAttribute('href')).toBe('/admin/finance?tab=transactions&user_id=u1')
  expect(q(el, 'user-topups')?.getAttribute('href')).toBe('/admin/finance?tab=transactions&user_id=u1')
  expect(q(el, 'user-reports')).toBeNull()

  // admin：全部都有
  user.value = { staff_roles: ['admin'] }
  el = show(AdminSupport); await settle()
  for (const s of ['user-reports', 'user-topups', 'wl-reports', 'topup-link']) expect(q(el, s)).not.toBeNull()
})
