// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, onMounted, ref } from 'vue'
import AccountNudge from '../app/components/creator/AccountNudge.vue'
import { mount, flush } from './vueHarness'

const prefs = new Map<string, string>()
const NuxtLink = defineComponent({ props: { to: String }, setup: (p, { slots }) => () => h('a', { href: p.to }, slots.default?.()) })
let mounted: ReturnType<typeof mount> | undefined
const user = ref<any>(null)

beforeEach(() => {
  prefs.clear()
  for (const [name, value] of Object.entries({ ref, computed, onMounted, useAuth: () => ({ user }),
    getPref: (k: string) => prefs.get(k) ?? null, setPref: (k: string, v: string) => prefs.set(k, v),
  })) vi.stubGlobal(name, value)
})
afterEach(() => { mounted?.unmount(); mounted = undefined; vi.unstubAllGlobals() })

async function open(u: any) {
  mounted?.unmount()
  user.value = u
  mounted = mount(AccountNudge, {}, { NuxtLink })
  await flush()
  return mounted.el
}
const label = (el: Element) => el.querySelector('section')?.getAttribute('aria-label')
const noEmail = { id: 'a', email: null, display_name: 'LINE 使用者' }
const prefixName = { id: 'a', email: 'chen.wei88@example.test', display_name: 'chen.wei88' }

it('asks accounts without an email to bind one, linking to settings', async () => {
  const el = await open(noEmail)
  expect(label(el)).toBe('綁定 Email，帳號更有保障')
  expect(el.querySelector('a')?.getAttribute('href')).toBe('/settings')
})

it('asks accounts still named after their email prefix to pick a name', async () => {
  expect(label(await open(prefixName))).toBe('設定你的顯示名稱')
})

it('matches the name the backend truncated to 50 characters', async () => {
  const local = 'a'.repeat(60)
  expect(label(await open({ id: 'a', email: `${local}@example.test`, display_name: 'a'.repeat(50) }))).toBe('設定你的顯示名稱')
})

it('stays hidden once the account has an email and its own name', async () => {
  expect((await open({ ...prefixName, display_name: '小魏' })).querySelector('section')).toBeNull()
})

it('closing is remembered per account and per kind', async () => {
  const el = await open(prefixName)
  el.querySelector('button')!.click()
  await flush()
  expect(el.querySelector('section')).toBeNull()
  expect((await open(prefixName)).querySelector('section')).toBeNull()
  expect((await open({ ...prefixName, id: 'b' })).querySelector('section')).not.toBeNull()
  expect((await open(noEmail)).querySelector('section')).not.toBeNull()
})
