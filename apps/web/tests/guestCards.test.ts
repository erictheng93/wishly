// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, nextTick, onMounted, ref } from 'vue'
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
  expect(el.textContent).toContain('送禮者 要送')
  expect(el.querySelector('[role="progressbar"]')!.getAttribute('aria-valuenow')).toBe('40')
  const product = el.querySelector<HTMLAnchorElement>('a')!
  expect(product.href).toBe('https://shop.example/bottle')
  expect(product.rel).toBe('noopener nofollow')
  expect(product.target).toBe('_blank')
  el.querySelector<HTMLButtonElement>('button')!.click()
  expect(claim).toHaveBeenCalledOnce()
  expect(fund).not.toHaveBeenCalled()
})

it('signed-out quantity card shows a login link instead of the claim button', () => {
  const NuxtLink = defineComponent({ props: ['to'], setup: (props, { slots }) => () => h('a', { href: JSON.stringify(props.to) }, slots.default?.()) })
  mounted = mount(ItemCard, { item: quantity, mine: null, masked: false, closed: false, online: true, loginTo: { path: '/login', query: { redirect: '/s/a' } } }, { NuxtLink },
    { deriveDisplayStatus, displayStatusLabel, fmtPts, fmtTime })
  const el = mounted.el
  expect(el.querySelector('button.js-claim')).toBeNull()
  const a = [...el.querySelectorAll('a')].find(x => x.textContent === '登入後來送')!
  expect(JSON.parse(a.getAttribute('href')!)).toEqual({ path: '/login', query: { redirect: '/s/a' } })
})

it('my existing claim can be edited even when the quantity is fully claimed', () => {
  const el = card({ ...quantity, is_fully_claimed: true, qty_claimed: 5 }, { mine: { qty: 2 } })
  expect(el.textContent).toContain('你要送 2 件')
  expect(el.querySelector('button')!.textContent).toContain('修改我要送的')
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

it.each([quantity, crowdfunding])('masked card shows progress but hides contributors, product link and actions', item => {
  const el = card(item, { masked: true })
  expect(el.textContent).toContain('建立者不能送自己清單上的禮物')
  expect(el.textContent).not.toContain('送禮者')
  expect(el.textContent).not.toContain('贊助者')
  expect(el.querySelector('[role="progressbar"]')).not.toBeNull()
  expect(el.textContent).toContain(item === quantity ? '2 / 5' : '200')
  expect(el.querySelector('button')).toBeNull()
  expect(el.querySelector('a')).toBeNull()
  expect(claim).not.toHaveBeenCalled()
  expect(fund).not.toHaveBeenCalled()
})

const NuxtLink = defineComponent({ props: ['to'], setup: (props, { slots }) => () => h('a', { href: props.to }, slots.default?.()) })
const fetchMe = vi.fn()
function top(hint: boolean | null) {
  fetchMe.mockResolvedValue(null)
  for (const [name, value] of Object.entries({ onMounted, useAuth: () => ({ fetchMe }), useAuthHint: () => ref(hint) })) vi.stubGlobal(name, value)
  mounted = mount(GuestTop, {}, { NuxtLink })
}
it('guest header routes home and to claim management without requiring sign in', () => {
  top(null)
  const links = [...mounted.el.querySelectorAll('header a')]
  expect(links.map(a => [a.textContent, a.getAttribute('href')])).toEqual([
    ['Wishly', '/'], ['探索', '/explore'], ['靈感', '/inspire'], ['品牌館', '/brands'], ['我的送禮', '/me/claims'],
  ])
})

it('guest header only asks who is signed in when the sign-in hint cookie exists', async () => {
  top(null)
  await nextTick()
  expect(fetchMe).not.toHaveBeenCalled()
  mounted!.unmount()
  top(true)
  await nextTick()
  expect(fetchMe).toHaveBeenCalledTimes(1)
})

// --- 支持品項卡片 ---
import SupportCard from '../app/components/guest/SupportCard.vue'
const support = (over: Record<string, any> = {}) => ({ id: 'it1', title: '幫基金會', funding_mode: 'support',
  support: { ended: false, target: { slug: 'org-1', title: '震災物資', cover_image_url: null, cover_preset: 'gift', owner_name: '家園基金會' }, points: 12000, qty: 85, ...over } })
function supportCard(item: any, closed = false) {
  const NuxtLink = defineComponent({ props: ['to'], setup: (p, { slots }) => () => h('a', { href: p.to }, slots.default?.()) })
  mounted = mount(SupportCard, { item, closed, index: 1 }, { NuxtLink }, { fmtPts, coverSrc: (w: any) => w.cover_image_url ?? '/covers/gift.svg' })
  return mounted.el
}
it('support card shows both numbers with labels and links with via', () => {
  const el = supportCard(support())
  expect(el.textContent).toContain('12,000')
  expect(el.textContent).toContain('85')
  expect(el.textContent).toContain('透過這裡籌到的點數')
  expect(el.textContent).toContain('透過這裡認捐的物資')
  expect(el.querySelector<HTMLAnchorElement>('a')!.getAttribute('href')).toBe('/s/org-1?via=it1')
})
it('support card zero is shown, null hides the numbers row', () => {
  const zero = supportCard(support({ points: 0, qty: 0 }))
  expect(zero.querySelector('.sp-nums')).not.toBeNull()
  expect([...zero.querySelectorAll('.sp-nums dd')].map(d => d.textContent)).toEqual(['0', '0'])
  mounted?.unmount()
  const el = supportCard(support({ points: null, qty: null }))
  expect(el.querySelector('.sp-nums')).toBeNull()
})
it('ended support card has no link', () => {
  const el = supportCard(support({ ended: true }))
  expect(el.querySelector('a')).toBeNull()
  expect(el.textContent).toContain('已結束')
  expect(el.textContent).toContain('12,000')
})
it('closed list support card has no link', () => {
  expect(supportCard(support(), true).querySelector('a')).toBeNull()
})
