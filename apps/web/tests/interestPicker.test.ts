// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import InterestPicker from '../app/components/InterestPicker.vue'
import CatalogGrid from '../app/components/guest/CatalogGrid.vue'
import { mount, flush } from './vueHarness'

let m: ReturnType<typeof mount> | undefined
afterEach(() => { m?.unmount(); m = undefined; vi.unstubAllGlobals() })
const chips = () => [...m!.el.querySelectorAll<HTMLButtonElement>('[data-chip]')]

function host(initial: string[], max?: number) {
  const v = ref(initial)
  m = mount(defineComponent({ render: () => h(InterestPicker, { modelValue: v.value, max, 'onUpdate:modelValue': (x: string[]) => { v.value = x } }) }))
  return v
}

it('點 chip 加入與移除', async () => {
  const v = host([])
  chips()[0]!.click(); await flush()
  expect(v.value.length).toBe(1)
  chips()[0]!.click(); await flush()
  expect(v.value).toEqual([])
})
it('max 選滿後其餘停用，不設 max 全可選', async () => {
  host(['coffee'], 1) // 以實際 id 為準：先取第一顆
  await flush()
  expect(chips().filter(c => c.disabled).length).toBe(chips().length - chips().filter(c => c.getAttribute('aria-pressed') === 'true').length)
  m!.unmount()
  host([])
  expect(chips().every(c => !c.disabled)).toBe(true)
})

const prods = ['r1', 'r2'].map(ref => ({ ref, title: ref, price_amount: 1, product_url: 'https://x', recipients: [], occasions: [], brand: null }))
function grid(matched?: Record<string, string[]>) {
  vi.stubGlobal('useI18n', () => ({ t: (k: string) => k }))
  vi.stubGlobal('useApi', () => ({ api: vi.fn() }))
  vi.stubGlobal('useRoute', () => ({ fullPath: '/' }))
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: '' } }))
  vi.stubGlobal('useAuth', () => ({ user: ref(null), fetchMe: vi.fn() }))
  vi.stubGlobal('ref', ref)
  m = mount(CatalogGrid, { products: prods, matched }, { NuxtLink: defineComponent({ render: () => h('a') }), GuestDialog: defineComponent({ render: () => h('div') }) })
  return [...m.el.querySelectorAll('article')].map(a => a.querySelector('[data-matched]'))
}
it('matched 只標示有值的商品', () => {
  const [a, b] = grid({ r1: ['coffee'] })
  expect(a).not.toBeNull(); expect(b).toBeNull()
})
it('不傳 matched 無標記', () => {
  expect(grid().every(x => x === null)).toBe(true)
})
