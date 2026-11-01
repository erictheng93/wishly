// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, h, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch, type Component } from 'vue'
import Channels from '../app/components/finance/Channels.vue'
import Overview from '../app/components/finance/Overview.vue'
import Payers from '../app/components/finance/Payers.vue'
import Risk from '../app/components/finance/Risk.vue'
import Transactions from '../app/components/finance/Transactions.vue'
import { fmtCount } from '../app/utils/analytics'
import { maskEmail } from '../app/utils/mask'
import { fmtPts, fmtTime } from '../app/utils/crowdfund'
import { delta, fmtDay, fmtRate, methodLabel, perPayer } from '../app/utils/finance'
import { flush, input, mount, submit } from './vueHarness'

const api = vi.fn()
const route = reactive<{ query: Record<string, string | undefined> }>({ query: {} })
const replace = vi.fn(({ query }: { query: Record<string, string | undefined> }) => { route.query = query })
const range = { from: '2026-10-02', to: '2026-10-31', days: 30 }
const helpers = { maskEmail, fmtCount, fmtDay, fmtPts, fmtRate, fmtTime, methodLabel }
const stubs: Record<string, Component> = {
  NuxtLink: { props: ['to'], setup: (props: any, { slots }: any) => () => h('a', { href: props.to }, slots.default?.()) },
  CreatorConfirm: {
    props: ['open', 'error'], emits: ['ok', 'close'],
    setup: (props: any, { slots, emit }: any) => () => props.open ? h('div', { class: 'confirm' }, [props.error, slots.default?.(), h('button', { class: 'ok', onClick: () => emit('ok') }, 'ok')]) : null,
  },
}
const mounted: { unmount: () => void }[] = []
function show(component: Component, props: Record<string, any> = {}) {
  const m = mount(component, props, stubs, helpers)
  mounted.push(m)
  return m.el
}
const deferred = () => { let resolve!: (v: any) => void; const p = new Promise(r => { resolve = r }); return { p, resolve } }
const settle = async () => { await flush(); await flush(); await nextTick() }
const click = async (el: Element, label: string) => {
  [...el.querySelectorAll('button')].find(b => b.textContent?.includes(label))!.click()
  await settle()
}
const lastQuery = () => api.mock.calls.at(-1)![1].query

beforeEach(() => {
  route.query = {}
  for (const [name, value] of Object.entries({
    computed, delta, errMsg: (e: any) => e.message, fmtCount, fmtDay, fmtPts, fmtRate, fmtTime, methodLabel, onBeforeUnmount, onMounted, perPayer, ref, watch,
    intlLocale: () => 'en-US',
    useApi: () => ({ api }),
    useRoute: () => route,
    useRouter: () => ({ replace }),
  })) vi.stubGlobal(name, value)
})

afterEach(() => {
  mounted.splice(0).forEach(m => m.unmount())
  vi.unstubAllGlobals()
  vi.useRealTimers()
  api.mockReset()
  replace.mockClear()
})

const overview = (daily = [{ date: '2026-10-30', gross: 100 }, { date: '2026-10-31', gross: 300 }]) => ({
  range,
  todo: { disputed: { count: 2, amount: 2000 }, cash_refunds: { count: 1, amount: 500 }, debts: { count: 0, amount: 0 }, pending: { count: 3, amount: 900 } },
  kpis: { cur: { gross: 1200, payers: 4, first_time: 1 }, prev: { gross: 1000, payers: 0, first_time: 0 } },
  daily,
})

it('總覽：待處理連到處理處，實收比較前期；前期為 0 的人數不顯示百分比', async () => {
  api.mockResolvedValue(overview())
  const el = show(Overview, { days: 30 })
  await settle()
  expect(api).toHaveBeenCalledWith('/admin/finance/overview', { query: { days: 30 } })
  const links = [...el.querySelectorAll('a')].map(a => a.getAttribute('href'))
  expect(links).toEqual(['/admin/finance?tab=transactions&status=disputed', '/admin/cash-refunds?status=all', '/admin/wallet-debts', '/admin/finance?tab=transactions&status=pending'])
  expect(el.textContent).toContain('1,200')
  expect(el.textContent).toContain('+20%')
  expect(el.querySelector('polyline')).not.toBeNull()
})

it('總覽：只有一天時不畫折線，改顯示數字', async () => {
  api.mockResolvedValue(overview([{ date: '2026-10-31', gross: 300 }]))
  const el = show(Overview, { days: 1 })
  await settle()
  expect(el.querySelector('polyline')).toBeNull()
  expect(el.textContent).toContain('300')
})

it('總覽：快速切換區間時，較慢的舊回應不會蓋掉新區間', async () => {
  const slow = deferred(), fast = deferred()
  api.mockReturnValueOnce(slow.p).mockReturnValueOnce(fast.p)
  const props = reactive({ days: 30 })
  const el = show(Overview, props)
  props.days = 7
  await settle()
  fast.resolve(overview([]))
  await settle()
  slow.resolve({ ...overview([]), kpis: { cur: { gross: 999, payers: 1, first_time: 0 }, prev: { gross: 0, payers: 0, first_time: 0 } } })
  await settle()
  expect(lastQuery()).toEqual({ days: 7 })
  expect(el.textContent).not.toContain('999')
})

it('區間重查失敗不留舊數字，重試後恢復', async () => {
  api.mockResolvedValueOnce(overview()).mockRejectedValueOnce(new Error('壞掉了')).mockResolvedValueOnce(overview())
  const props = reactive({ days: 30 })
  const el = show(Overview, props)
  await settle()
  props.days = 90
  await settle()
  expect(el.querySelector('[role=alert]')?.textContent).toContain('壞掉了')
  expect(el.textContent).not.toContain('1,200')
  el.querySelector<HTMLButtonElement>('[role=alert] button')!.click()
  await settle()
  expect(el.querySelector('[role=alert]')).toBeNull()
  expect(el.textContent).toContain('1,200')
})

it('付費用戶：前 20 名連到交易明細；已刪除用戶沒有 email 也能顯示', async () => {
  api.mockResolvedValue({
    range, gross: 1000, payers: 2, first_time: 1, repeat_rate: 0.5,
    buckets: [{ key: 'lt500', count: 0, amount: 0 }, { key: '500_999', count: 1, amount: 500 }, { key: '1000_2999', count: 1, amount: 500 }, { key: '3000_9999', count: 0, amount: 0 }, { key: 'gte10000', count: 0, amount: 0 }],
    top: [
      { user: { id: 'u1', display_name: '小明', email: 'a@example.com' }, amount: 700, count: 2, first_paid_at: '2026-10-01T00:00:00Z', last_paid_at: '2026-10-30T16:30:00Z' },
      { user: { id: 'u2', display_name: '已刪除的使用者', email: null }, amount: 300, count: 1, first_paid_at: '2026-10-02T00:00:00Z', last_paid_at: '2026-10-02T00:00:00Z' },
    ],
  })
  const el = show(Payers, { days: 30 })
  await settle()
  expect(api).toHaveBeenCalledWith('/admin/finance/payers', { query: { days: 30 } })
  expect([...el.querySelectorAll('a')].map(a => a.getAttribute('href'))).toEqual(['/admin/finance?tab=transactions&user_id=u1', '/admin/finance?tab=transactions&user_id=u2'])
  expect(el.textContent).toContain('a@example.com')
  expect(el.textContent).toContain('已刪除的使用者')
  expect(el.textContent).toContain('50')
})

it('付費用戶：沒有付費者時比率顯示 –，不列表格', async () => {
  api.mockResolvedValue({ range, gross: 0, payers: 0, first_time: 0, repeat_rate: null, buckets: [], top: [] })
  const el = show(Payers, { days: 7 })
  await settle()
  expect(el.querySelector('table')).toBeNull()
})

it('渠道：比率為 null 顯示 –，LINE Pay 顯示名稱而不是代碼', async () => {
  api.mockResolvedValue({ range, channels: [
    { method: 'card', gross: 900, count: 3, avg: 300, success_rate: 0.75, payers: 2 },
    { method: 'line_pay', gross: 0, count: 0, avg: null, success_rate: null, payers: 0 },
  ] })
  const el = show(Channels, { days: 30 })
  await settle()
  expect(el.textContent).toContain('LINE Pay')
  expect(el.textContent).not.toContain('line_pay')
  expect(el.textContent).toContain('75')
  expect(el.textContent).toContain('–')
})

it('渠道：載入失敗顯示錯誤與重試', async () => {
  api.mockRejectedValueOnce(new Error('失敗')).mockResolvedValueOnce({ range, channels: [] })
  const el = show(Channels, { days: 30 })
  await settle()
  expect(el.querySelector('[role=alert]')?.textContent).toContain('失敗')
  el.querySelector<HTMLButtonElement>('[role=alert] button')!.click()
  await settle()
  expect(el.querySelector('[role=alert]')).toBeNull()
})

const risk = (extra = {}) => ({
  range,
  liability: { total: 180, unused: 150, refundable: 100, non_refundable: 50, pledged: 30 },
  refunds: { count: 1, amount: 40 }, chargebacks: { count: 1, amount: 60 }, loss_rate: 0.1,
  disputed: [{ id: 'd1', amount: 1000, method: 'card', paid_at: '2026-10-30T16:30:00Z', display_name: '小明' }],
  cash_refunds_open: { count: 2, amount: 70 }, debts_open: { count: 1, amount: 80 }, frozen_wallets: 3, invoices_pending: { count: 4, amount: 90 },
  ...extra,
})

it('風險：列出負債與爭議，並連到退款、欠款處理頁', async () => {
  api.mockResolvedValue(risk())
  const el = show(Risk, { days: 30 })
  await settle()
  expect(api).toHaveBeenCalledWith('/admin/finance/risk', { query: { days: 30 } })
  const links = [...el.querySelectorAll('a')].map(a => a.getAttribute('href'))
  expect(links).toContain('/admin/cash-refunds?status=all')
  expect(links).toContain('/admin/wallet-debts')
  expect(el.textContent).toContain('180')
  expect(el.textContent).toContain('小明')
  expect(el.textContent).toContain('10/31') // 台北時間的付款日
})

it('風險：無爭議且無實收時，損失率顯示 –', async () => {
  api.mockResolvedValue(risk({ disputed: [], loss_rate: null }))
  const el = show(Risk, { days: 30 })
  await settle()
  expect(el.querySelector('table')).toBeNull()
  expect(el.textContent).toContain('–')
})

const row = (extra = {}) => ({ id: 't1', amount: 1000, method: 'card', status: 'paid', provider: 'ecpay', provider_txn_id: 'X1', created_at: '2026-10-30T00:00:00Z', paid_at: '2026-10-30T00:00:00Z', user: { id: 'u1', display_name: '小明', email: 'a@example.com' }, wallet: { id: 'w1', status: 'active' }, ...extra })
const list = (data: any[], next_cursor: string | null = null) => ({ data, next_cursor })

it('交易明細：篩選初值取自網址；不認得的狀態當作全部', async () => {
  route.query = { status: 'bogus', method: 'atm', user_id: 'u1' }
  api.mockResolvedValue(list([row()]))
  show(Transactions)
  await settle()
  expect(lastQuery()).toMatchObject({ status: undefined, method: 'atm', user_id: 'u1' })
})

it('交易明細：清除用戶篩選時保留已選的狀態', async () => {
  route.query = { tab: 'transactions', user_id: 'u1' }
  api.mockResolvedValue(list([row()]))
  const el = show(Transactions)
  await settle()
  const status = el.querySelector('select')!
  status.value = 'disputed'
  status.dispatchEvent(new Event('change', { bubbles: true }))
  await settle()
  expect(lastQuery()).toMatchObject({ status: 'disputed', user_id: 'u1' })
  el.querySelector<HTMLButtonElement>('[role=status] button')!.click()
  await settle()
  expect(route.query).toMatchObject({ tab: 'transactions', status: 'disputed' })
  expect(lastQuery()).toMatchObject({ status: 'disputed', user_id: undefined })
})

it('交易明細：搜尋送出才查詢，去除前後空白', async () => {
  api.mockResolvedValue(list([]))
  const el = show(Transactions)
  await settle()
  await input(el, 'input[type=search]', '  小明 ')
  await submit(el)
  await settle()
  expect(lastQuery().q).toBe('小明')
})

it('交易明細：換篩選失敗時清掉舊列與 cursor，避免載入更多混到舊資料', async () => {
  api.mockResolvedValueOnce(list([row()], 'c1')).mockRejectedValueOnce(new Error('失敗'))
  const el = show(Transactions)
  await settle()
  route.query = { status: 'paid' }
  await settle()
  expect(el.querySelector('article')).toBeNull()
  expect(el.querySelector('[role=alert]')?.textContent).toContain('失敗')
  expect([...el.querySelectorAll('button')].some(b => b.textContent === '載入更多')).toBe(false)
})

it('交易明細：載入更多會帶 cursor 並接在原列表後', async () => {
  api.mockResolvedValueOnce(list([row()], 'c1')).mockResolvedValueOnce(list([row({ id: 't2' })]))
  const el = show(Transactions)
  await settle()
  el.querySelector<HTMLButtonElement>('button.block')!.click()
  await settle()
  expect(lastQuery().cursor).toBe('c1')
  expect(el.querySelectorAll('article')).toHaveLength(2)
})

it('爭議：網路中斷重試沿用同一把 Idempotency-Key，後端已回應才換新', async () => {
  const network = Object.assign(new Error('離線'), { code: 'NETWORK' })
  const rejected = Object.assign(new Error('內部錯誤'), { code: 'INTERNAL' })
  api.mockResolvedValueOnce(list([row()]))
    .mockRejectedValueOnce(network).mockRejectedValueOnce(rejected).mockResolvedValueOnce({ applied: true, status: 'disputed' })
  const el = show(Transactions)
  await settle()
  ;[...el.querySelectorAll<HTMLButtonElement>('article button')].find(b => b.textContent?.includes('標記爭議'))!.click()
  await settle()
  const ok = () => el.querySelector<HTMLButtonElement>('.confirm .ok')!
  ok().click(); await settle()
  ok().click(); await settle()
  ok().click(); await settle()
  const keys = api.mock.calls.slice(1).map(c => c[1].headers['Idempotency-Key'])
  expect(keys[0]).toBe(keys[1])
  expect(keys[2]).not.toBe(keys[1])
  expect(el.querySelector('.confirm')).toBeNull()
  expect(el.querySelector('article')?.getAttribute('data-status')).toBe('disputed')
  expect(el.querySelector('.c-ok')).not.toBeNull()
})

it('爭議：狀態已被改動（409）時關閉對話框並重讀列表', async () => {
  api.mockResolvedValueOnce(list([row()]))
    .mockRejectedValueOnce(Object.assign(new Error('已變更'), { code: 'INVALID_STATE_TRANSITION' }))
    .mockResolvedValueOnce(list([row({ status: 'disputed' })]))
  const el = show(Transactions)
  await settle()
  ;[...el.querySelectorAll<HTMLButtonElement>('article button')].find(b => b.textContent?.includes('標記爭議'))!.click()
  await settle()
  el.querySelector<HTMLButtonElement>('.confirm .ok')!.click()
  await settle()
  expect(el.querySelector('.confirm')).toBeNull()
  expect(el.querySelector('article')?.getAttribute('data-status')).toBe('disputed')
  expect(el.querySelector('[role=alert]')).not.toBeNull()
})

it('交易 Email 預設遮罩，明確點擊才顯示，重新載入恢復遮罩', async () => {
  api.mockResolvedValue({ data: [{ id: 'a', status: 'paid', amount: 100, method: 'card', user: { display_name: 'Alice', email: 'alice@example.com' } }], next_cursor: null })
  const el = show(Transactions)
  await settle()
  expect(el.textContent).toContain('a***@example.com')
  expect(el.textContent).not.toContain('alice@example.com')
  await click(el, '顯示')
  expect(el.textContent).toContain('alice@example.com')
  route.query = { status: 'paid' }
  await settle()
  expect(el.textContent).not.toContain('alice@example.com')
})

it('交易明細：有商店交易編號才顯示', async () => {
  api.mockResolvedValue(list([row({ id: 'a', merchant_trade_no: 'WT2610310001' }), row({ id: 'b', merchant_trade_no: null })]))
  const el = show(Transactions)
  await settle()
  expect(el.textContent).toContain('WT2610310001')
  expect(el.textContent!.split('WT2610310001').length).toBe(2)
})
