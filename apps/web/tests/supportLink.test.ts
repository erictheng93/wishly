// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest'
import { computed, h, ref } from 'vue'
import SupportLink from '../app/components/SupportLink.vue'
import { mount } from './vueHarness'

afterEach(() => vi.unstubAllGlobals())

const NuxtLink = { props: ['to'], setup: (p: any, { slots }: any) => () => h('a', { href: p.to }, slots.default?.()) }

function render(user: any, props: Record<string, any> = {}) {
  for (const [name, value] of Object.entries({
    computed,
    useRuntimeConfig: () => ({ public: { supportEmail: 'support@wishly.tw' } }),
    useState: () => ref(user),
  })) vi.stubGlobal(name, value)
  const m = mount(SupportLink, props, { NuxtLink })
  const a = m.el.querySelector('[data-testid="support-link"]')!
  const form = m.el.querySelector('[data-testid="support-form-link"]')
  const r = { href: a.getAttribute('href')!, text: a.textContent, form: form && { href: form.getAttribute('href'), text: form.textContent } }
  m.unmount()
  return r
}

it('未登入：只帶主旨，顯示信箱，也顯示站內客服表單', () => {
  const r = render(null)
  expect(r.text).toBe('support@wishly.tw')
  expect(r.href).toBe(`mailto:support@wishly.tw?subject=${encodeURIComponent('Wishly 客服')}`)
  expect(r.form).toEqual({ href: '/support', text: '或填寫客服表單' })
})

it('已登入：主旨帶情境、內文帶帳號 ID，並顯示站內客服表單', () => {
  const r = render({ id: 'u-123' }, { topic: '生日清單' })
  const u = new URL(r.href)
  expect(u.searchParams.get('subject')).toBe('Wishly 客服：生日清單')
  expect(u.searchParams.get('body')).toContain('帳號 ID：u-123')
  expect(r.form?.href).toBe('/support')
})

it('form=false 時不顯示站內表單連結（錯誤頁）', () => {
  expect(render(null, { form: false }).form).toBeNull()
})
