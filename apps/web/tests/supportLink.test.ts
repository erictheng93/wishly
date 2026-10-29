// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from 'vitest'
import { computed, ref } from 'vue'
import SupportLink from '../app/components/SupportLink.vue'
import { mount } from './vueHarness'

afterEach(() => vi.unstubAllGlobals())

function href(user: any, topic?: string) {
  for (const [name, value] of Object.entries({
    computed,
    useRuntimeConfig: () => ({ public: { supportEmail: 'support@wishly.tw' } }),
    useState: () => ref(user),
  })) vi.stubGlobal(name, value)
  const m = mount(SupportLink, topic ? { topic } : {})
  const a = m.el.querySelector('a')!
  const r = { href: a.getAttribute('href')!, text: a.textContent }
  m.unmount()
  return r
}

it('未登入：只帶主旨，顯示信箱', () => {
  const r = href(null)
  expect(r.text).toBe('support@wishly.tw')
  expect(r.href).toBe(`mailto:support@wishly.tw?subject=${encodeURIComponent('Wishly 客服')}`)
})

it('已登入：主旨帶情境、內文帶帳號 ID', () => {
  const u = new URL(href({ id: 'u-123' }, '生日清單').href)
  expect(u.searchParams.get('subject')).toBe('Wishly 客服：生日清單')
  expect(u.searchParams.get('body')).toContain('帳號 ID：u-123')
})
