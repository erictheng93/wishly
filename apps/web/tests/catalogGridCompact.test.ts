// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import CatalogGrid from '../app/components/guest/CatalogGrid.vue'
import { mount } from './vueHarness'

const Stub = defineComponent({ setup: (_, { slots }) => () => h('div', slots.default?.()) })
const NuxtLink = defineComponent({ props: { to: String }, setup: (p, { slots }) => () => h('a', { href: p.to }, slots.default?.()) })
const prod = { ref: 'r1', title: '咖啡杯', spec: '350ml', note: '店家備註', price_amount: 500, product_url: 'https://x.test', recipients: [], occasions: [], brand: { slug: 'b', name: '品牌' }, image_url: null }
let m: ReturnType<typeof mount> | undefined
beforeEach(() => {
  vi.stubGlobal('ref', ref)
  vi.stubGlobal('useApi', () => ({ api: vi.fn().mockResolvedValue({ data: [], next_cursor: null }) }))
  vi.stubGlobal('useRoute', () => ({ fullPath: '/' }))
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: '' } }))
  vi.stubGlobal('useAuth', () => ({ user: ref(null), fetchMe: vi.fn() }))
})
afterEach(() => { m?.unmount(); vi.unstubAllGlobals() })
const render = (compact: boolean) => {
  m = mount(CatalogGrid, { products: [prod], matched: { r1: ['coffee'] }, compact }, { NuxtLink, GuestDialog: Stub })
  return m.el
}

it('compact 保留符合興趣行與開啟動作，省略說明與按鈕', () => {
  const el = render(true)
  expect(el.querySelector('[data-matched]')).not.toBeNull()
  expect(el.querySelector('.x-hit')).not.toBeNull()
  expect(el.textContent).not.toContain('店家備註')
  expect(el.textContent).not.toContain('350ml')
  expect(el.querySelectorAll('.x-act, .x-act button').length).toBe(0)
})
it('非 compact 維持完整卡片', () => {
  const el = render(false)
  expect(el.textContent).toContain('店家備註')
  expect(el.querySelectorAll('.x-act button').length).toBe(2)
})
