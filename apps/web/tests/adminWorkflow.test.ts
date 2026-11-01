// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, h, nextTick, onMounted, reactive, ref, watch } from 'vue'
import Orgs from '../app/pages/admin/orgs.vue'
import Index from '../app/pages/admin/index.vue'
import Orders from '../app/pages/admin/orders.vue'
import { ORDER_STATUS, orderActions, fmtPts, fmtTime } from '../app/utils/crowdfund'
import { flush, input, mount } from './vueHarness'

const api = vi.fn()
const mounted: ReturnType<typeof mount>[] = []
const stubs = {
  CreatorHeader: { template: '<header><slot /></header>' },
  CreatorStatusBadge: { template: '<span />' },
  NuxtLink: { props: ['to'], setup: (p: any, { slots }: any) => () => h('a', { href: p.to }, slots.default?.()) },
  CreatorConfirm: {
    props: ['open', 'title', 'error', 'disabled', 'busy'], emits: ['ok', 'close'],
    setup: (p: any, { slots, emit }: any) => () => p.open ? h('div', { class: 'confirm' }, [p.title, p.error, slots.default?.(), h('button', { class: 'ok', disabled: p.disabled || p.busy, onClick: () => emit('ok') }, 'ok')]) : null,
  },
}
const settle = async () => { await flush(); await flush(); await nextTick() }
function show(c: any) { const m = mount(c, {}, stubs, { fmtTime, fmtPts, ORDER_STATUS, orderActions, label: (m: any, k: string) => m[k] }); mounted.push(m); return m.el }
beforeEach(() => {
  for (const [k, v] of Object.entries({ ORDER_STATUS, orderActions, fmtPts, computed, onMounted, reactive, ref, watch, definePageMeta: () => {}, useHead: () => {}, useApi: () => ({ api }), useAuth: () => ({ user: null }), fmtTime: (v: any) => v, errMsg: (e: any) => e.message })) vi.stubGlobal(k, v)
})
afterEach(() => { mounted.splice(0).forEach(m => m.unmount()); vi.unstubAllGlobals(); api.mockReset() })

it('組織撤銷先填理由，第二次確認保留理由；許可直接下載對應文件', async () => {
  const org = { id: 'org', display_name: 'Org', verification_status: 'verified' }
  api.mockImplementation(async (url: string) => url === '/admin/orgs' ? { data: [org] } : { org, documents: [{ id: 'doc', kind: 'permit', content_type: 'application/pdf' }], permits: [{ id: 'permit', permit_no: 'P-123', status: 'approved', document_id: 'doc' }], members: [] })
  const el = show(Orgs); await settle()
  el.querySelector<HTMLButtonElement>('article button')!.click(); await settle()
  expect(el.querySelector('[data-permit="permit"]')?.textContent).toContain('下載')
  const buttons = [...el.querySelectorAll<HTMLButtonElement>('article button')]
  buttons.at(-1)!.click(); await settle()
  expect(el.querySelector<HTMLButtonElement>('.confirm .ok')!.disabled).toBe(true)
  await input(el, '.confirm textarea', '資格已失效')
  el.querySelector<HTMLButtonElement>('.confirm .ok')!.click(); await settle()
  expect(el.querySelector('.confirm')?.textContent).toContain('資格已失效')
  expect(el.querySelector('.confirm textarea')).toBeNull()
  el.querySelector<HTMLButtonElement>('.confirm .ok')!.click(); await settle()
  expect(api).toHaveBeenCalledWith('/admin/orgs/org/review', { method: 'POST', body: { decision: 'reject', note: '資格已失效' } })
})

it('首頁導覽顯示六種待辦數字，包括零', async () => {
  api.mockImplementation(async (url: string) => url === '/admin/stats' ? { pending_orgs: 2, pending_permits: 3, pending_orders: 4, requested_refunds: 5, disputed_topups: 6, open_debts: 0, staff_mfa: 2 } : { data: [] })
  const el = show(Index); await settle()
  for (const [key, n] of Object.entries({ pending_orgs: 2, pending_permits: 3, pending_orders: 4, requested_refunds: 5, disputed_topups: 6, open_debts: 0 })) {
    expect(el.querySelector(`[data-testid="todo-${key}"]`)?.textContent).toContain(String(n))
  }
})

it('採購單只有點擊才取得收件資訊，隱藏後可重新查看', async () => {
  api.mockImplementation(async (url: string) => url.endsWith('/shipping-address') ? { recipient_name: '收件人', phone: '0912345678', address: '完整地址' } : { data: [{ id: 'order', status: 'pending', shipping_address: { recipient_name: '***', phone: '***', address: '***' } }] })
  const el = show(Orders); await settle()
  expect(api.mock.calls.some(([url]) => url.endsWith('/shipping-address'))).toBe(false)
  expect(el.textContent).not.toContain('完整地址')
  const clickText = async (text: string) => { [...el.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.includes(text))!.click(); await settle() }
  await clickText('顯示收件資訊')
  expect(el.textContent).toContain('完整地址')
  expect(api).toHaveBeenCalledWith('/admin/orders/order/shipping-address')
  await clickText('隱藏收件資訊')
  expect(el.textContent).not.toContain('完整地址')
  await clickText('顯示收件資訊')
  expect(api.mock.calls.filter(([url]) => url.endsWith('/shipping-address'))).toHaveLength(2)
})

it('檢舉結案對話框送出備註，已結案列表顯示原備註', async () => {
  api.mockImplementation(async (url: string, opts: any) => {
    if (url === '/admin/stats') return { staff_mfa: 2 }
    if (opts?.method === 'PATCH') return {}
    return { data: opts?.query?.status === 'open' ? [{ id: 'report', status: 'open', reason: 'other', wishlist: { id: 'w', slug: 'slug', title: 'List' } }] : [{ id: 'report', status: 'dismissed', reason: 'other', resolution_note: '查證後駁回', wishlist: { id: 'w', slug: 'slug', title: 'List' } }] }
  })
  const el = show(Index); await settle()
  ;[...el.querySelectorAll<HTMLButtonElement>('article button')].find(b => b.textContent?.includes('駁回'))!.click(); await settle()
  expect(el.querySelector('.confirm textarea')).not.toBeNull()
  await input(el, '.confirm textarea', '查證後駁回')
  el.querySelector<HTMLButtonElement>('.confirm .ok')!.click(); await settle()
  expect(api).toHaveBeenCalledWith('/admin/reports/report', { method: 'PATCH', body: { status: 'dismissed', note: '查證後駁回' } })
  const select = el.querySelector('select')!; select.value = 'dismissed'; select.dispatchEvent(new Event('change')); await settle()
  expect(el.textContent).toContain('查證後駁回')
})

it('收件資訊失敗維持遮罩；切換篩選後忽略舊的明文回應', async () => {
  let resolve!: (v: any) => void
  let attempts = 0
  api.mockImplementation((url: string) => {
    if (url.endsWith('/shipping-address')) {
      if (++attempts === 1) return Promise.reject(new Error('暫時無法讀取'))
      return new Promise(r => { resolve = r })
    }
    return Promise.resolve({ data: [{ id: 'order', status: 'pending', shipping_address: { recipient_name: '***', phone: '***', address: '***' } }] })
  })
  const el = show(Orders); await settle()
  const reveal = () => [...el.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent?.includes('顯示收件資訊'))!
  reveal().click(); await settle()
  expect(el.textContent).toContain('暫時無法讀取')
  expect(reveal().disabled).toBe(false)
  reveal().click(); await settle()
  expect(reveal().disabled).toBe(true)
  const select = el.querySelector('select')!; select.value = 'placed'; select.dispatchEvent(new Event('change')); await settle()
  resolve({ recipient_name: '舊收件人', phone: '0912345678', address: '舊地址' }); await settle()
  expect(el.textContent).not.toContain('舊地址')
  expect(reveal().disabled).toBe(false)
})
