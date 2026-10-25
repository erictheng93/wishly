// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, h, nextTick, onBeforeUnmount, onMounted, reactive, ref, useId, watch } from 'vue'
import UserMenu from '../app/components/creator/UserMenu.vue'
import { mount } from './vueHarness'

let mounted: ReturnType<typeof mount> | undefined
const route = reactive({ fullPath: '/dashboard' })

beforeEach(() => {
  route.fullPath = '/dashboard'
  for (const [name, value] of Object.entries({
    computed, onBeforeUnmount, onMounted, ref, useId, watch,
    useRoute: () => route,
    useFriendRequests: () => ({ count: ref(0) }),
    useAuth: () => ({ user: ref({ display_name: '小華', handle: 'hua', is_staff: true }), logout: vi.fn() }),
  })) vi.stubGlobal(name, value)
  mounted = mount(UserMenu, {}, {
    NuxtLink: { props: ['to'], setup: (props, { slots }) => () => h('a', { href: props.to }, slots.default?.()) },
  })
})

afterEach(() => {
  mounted?.unmount()
  vi.unstubAllGlobals()
})

async function expand() {
  const trigger = mounted!.el.querySelector<HTMLButtonElement>('button')!
  trigger.focus()
  trigger.click()
  await nextTick()
  return trigger
}

it('exposes ordinary links and a logout button in an expanded account disclosure', async () => {
  const trigger = await expand()
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  expect(trigger.hasAttribute('aria-haspopup')).toBe(false)
  const panel = document.getElementById(trigger.getAttribute('aria-controls')!)!
  expect(panel).not.toBeNull()
  expect(panel.querySelectorAll('a[href]')).toHaveLength(5)
  expect(panel.querySelector('button')?.textContent).toBe('登出')
  expect(mounted!.el.querySelector('[role="menu"], [role="menuitem"]')).toBeNull()
  expect(document.activeElement).toBe(trigger)
})

it.each(['a', '.um-pop button'])('Escape from %s closes the disclosure and restores trigger focus', async selector => {
  const trigger = await expand()
  const child = mounted!.el.querySelector<HTMLElement>(selector)!
  child.focus()
  const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
  child.dispatchEvent(escape)
  await nextTick()
  expect(trigger.getAttribute('aria-expanded')).toBe('false')
  expect(document.activeElement).toBe(trigger)
  expect(escape.defaultPrevented).toBe(true)
})

it('closes when focus leaves the disclosure without moving focus back from the next control', async () => {
  const trigger = await expand()
  mounted!.el.querySelector<HTMLElement>('a')!.focus()
  await nextTick()
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
  const outside = document.createElement('button')
  document.body.append(outside)
  try {
    outside.focus()
    await nextTick()
    expect(trigger.getAttribute('aria-expanded')).toBe('false')
    expect(document.activeElement).toBe(outside)
  } finally {
    outside.remove()
  }
})

it('stays open when focus is lost without a target (WebKit click on a button)', async () => {
  const trigger = await expand()
  trigger.dispatchEvent(new FocusEvent('focusout', { bubbles: true, relatedTarget: null }))
  await nextTick()
  expect(trigger.getAttribute('aria-expanded')).toBe('true')
})
