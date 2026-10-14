// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, defineComponent, h } from 'vue'
import ItemCard from '../app/components/guest/ItemCard.vue'
import GuestTop from '../app/components/guest/GuestTop.vue'
import { deriveDisplayStatus, displayStatusLabel, fmtPts, fmtTime } from '../app/utils/crowdfund'
import { mount } from './vueHarness'

let mounted: ReturnType<typeof mount> | undefined
const claim = vi.fn(), fund = vi.fn()
beforeEach(() => {
  for (const [name, value] of Object.entries({ computed, deriveDisplayStatus, displayStatusLabel, fmtPts, fmtTime })) vi.stubGlobal(name, value)
})
afterEach(() => { mounted?.unmount(); mounted = undefined; vi.unstubAllGlobals(); vi.resetAllMocks() })
const quantity = { title: '保溫瓶', priority: 'high', brand: '品牌', spec: '500ml', qty_claimed: 2, qty_needed: 5,
  progress_percent: 40, is_fully_claimed: false, claimers: [{ display_name: '送禮者' }], product_url: 'https://shop.example/bottle' }
const crowdfunding = { title: '推車', funding_mode: 'crowdfund', funding_status: 'open', pledged_points: 200,
  target_points: 500, remaining_points: 300, progress_percent: 40, funding_deadline: '2026-10-20T15:59:00Z',
  contributors: [{ display_name: '贊助者', points: 200 }], product_url: 'https://shop.example/stroller' }
function card(item: any = quantity, options: Record<string, any> = {}) {
  mounted = mount(ItemCard, { item, mine: null, masked: false, closed: false, online: true, onClaim: claim, onFund: fund, ...options }, {},
    { deriveDisplayStatus, displayStatusLabel, fmtPts, fmtTime })
  return mounted.el
}

it('quantity card exposes inventory, priority and a safe product link and opens the claim action', () => {
  const el = card()
  expect(el.textContent).toContain('2 / 5')
  expect(el.textContent).toContain('優先度 高')
  expect(el.textContent).toContain('品牌・500ml')
  expect(el.textContent).toContain('送禮者 已認領')
  expect(el.querySelector('[role="progressbar"]')!.getAttribute('aria-valuenow')).toBe('40')
  const product = el.querySelector<HTMLAnchorElement>('a')!
  expect(product.href).toBe('https://shop.example/bottle')
  expect(product.rel).toBe('noopener nofollow')
  expect(product.target).toBe('_blank')
  el.querySelector<HTMLButtonElement>('button')!.click()
  expect(claim).toHaveBeenCalledOnce()
  expect(fund).not.toHaveBeenCalled()
})

it('my existing claim can be edited even when the quantity is fully claimed', () => {
  const el = card({ ...quantity, is_fully_claimed: true, qty_claimed: 5 }, { mine: { qty: 2 } })
  expect(el.textContent).toContain('你已認領 2 件')
  expect(el.querySelector('button')!.textContent).toContain('修改我的認領')
  el.querySelector<HTMLButtonElement>('button')!.click()
  expect(claim).toHaveBeenCalledOnce()
})

it.each(['offline', 'closed', 'full'])('%s quantity card prevents a new claim', state => {
  const el = card({ ...quantity, is_fully_claimed: state === 'full' }, { online: state !== 'offline', closed: state === 'closed' })
  const button = el.querySelector<HTMLButtonElement>('button')
  if (state === 'offline') { expect(button!.disabled).toBe(true); button!.click() }
  else expect(button).toBeNull()
  expect(claim).not.toHaveBeenCalled()
})

it('open crowdfunding displays funding progress and opens fund rather than claim', () => {
  const el = card(crowdfunding)
  expect(el.textContent).toContain('募集中')
  expect(el.textContent).toContain('200 / 500 點')
  expect(el.textContent).toContain('還差 300 點')
  expect(el.textContent).toContain('贊助者 200 點 已贊助')
  expect(el.textContent).toContain('台北時間')
  el.querySelector<HTMLButtonElement>('button')!.click()
  expect(fund).toHaveBeenCalledOnce()
  expect(claim).not.toHaveBeenCalled()
})

it.each([
  ['offline', 'open'], ['closed', 'open'], ['funded', 'funded'], ['expired', 'expired'],
])('%s crowdfunding prevents funding', (state, funding_status) => {
  const el = card({ ...crowdfunding, funding_status }, { online: state !== 'offline', closed: state === 'closed' })
  const button = el.querySelector<HTMLButtonElement>('button')
  if (state === 'offline') { expect(button!.disabled).toBe(true); button!.click() }
  else expect(button).toBeNull()
  if (state === 'expired') expect(el.textContent).toContain('募集已截止')
  if (state === 'funded') expect(el.textContent).toContain('已募集完成')
  expect(fund).not.toHaveBeenCalled()
})

it.each([quantity, crowdfunding])('masked card hides contribution details, progress, product link and actions', item => {
  const el = card(item, { masked: true })
  expect(el.textContent).toContain('驚喜模式中')
  expect(el.textContent).toContain('建立者不可認領自己的清單')
  expect(el.textContent).not.toContain('送禮者')
  expect(el.textContent).not.toContain('贊助者')
  expect(el.querySelector('[role="progressbar"]')).toBeNull()
  expect(el.querySelector('button')).toBeNull()
  expect(el.querySelector('a')).toBeNull()
  expect(claim).not.toHaveBeenCalled()
  expect(fund).not.toHaveBeenCalled()
})

it('guest header routes home and to claim management without requiring sign in', () => {
  const NuxtLink = defineComponent({ props: ['to'], setup: (props, { slots }) => () => h('a', { href: props.to }, slots.default?.()) })
  mounted = mount(GuestTop, {}, { NuxtLink })
  const links = [...mounted.el.querySelectorAll('header a')]
  expect(links.map(a => [a.textContent, a.getAttribute('href')])).toEqual([
    ['WishSync', '/'], ['我的認領', '/me/claims'],
  ])
})
