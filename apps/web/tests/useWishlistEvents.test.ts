// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { defineComponent, ref, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useWishlistEvents } from '../app/composables/useWishlistEvents'
import { mount } from './vueHarness'

class FakeES {
  static all: FakeES[] = []
  readyState = 0
  onopen = () => {}; onerror = () => {}
  listeners = new Map<string, () => void>()
  closed = false
  constructor(public url: string, public options: any) { FakeES.all.push(this) }
  addEventListener(type: string, callback: () => void) { this.listeners.set(type, callback) }
  close() { this.closed = true }
}
let mounted: ReturnType<typeof mount> | undefined
beforeEach(() => {
  FakeES.all = []
  vi.useFakeTimers()
  for (const [name, value] of Object.entries({ ref, watch, onMounted, onBeforeUnmount, EventSource: FakeES,
    useApi: () => ({ base: '/api/v1' }), getListAccess: () => 'a+b' })) vi.stubGlobal(name, value)
})
afterEach(() => { mounted?.unmount(); mounted = undefined; vi.useRealTimers(); vi.unstubAllGlobals() })
function setup() {
  const slug = ref<string | null>('list'), events: string[] = []
  let state!: ReturnType<typeof useWishlistEvents>
  mounted = mount(defineComponent({ setup() { state = useWishlistEvents(slug, t => events.push(t), 1000); return () => null } }))
  return { slug, events, state }
}

it('starts polling after exactly two consecutive reconnect errors and stops on recovery', () => {
  const { events, state } = setup(), es = FakeES.all[0]!
  expect(es.url).toBe('/api/v1/public/wishlists/list/events?access=a%2Bb')
  expect(es.options).toEqual({ withCredentials: true })
  es.onerror()
  vi.advanceTimersByTime(2000)
  expect(events).toEqual([])
  expect(state.status.value).toBe('idle')
  es.onerror()
  expect(state.status.value).toBe('polling')
  vi.advanceTimersByTime(1000)
  expect(events).toEqual(['poll'])
  es.onopen()
  expect(state.status.value).toBe('live')
  vi.advanceTimersByTime(2000)
  expect(events).toEqual(['poll'])
  // Successful connection resets the consecutive error counter.
  es.onerror()
  vi.advanceTimersByTime(1000)
  expect(events).toEqual(['poll'])
  es.listeners.get('item.updated')!()
  es.listeners.get('wishlist.updated')!()
  expect(events).toEqual(['poll', 'item.updated', 'wishlist.updated'])
})

it('CLOSED connection immediately falls back and unmount closes connection and cancels polling', () => {
  const { state, events } = setup(), es = FakeES.all[0]!
  es.readyState = 2; es.onerror()
  expect(state.status.value).toBe('polling')
  mounted!.unmount(); mounted = undefined
  expect(es.closed).toBe(true)
  vi.advanceTimersByTime(3000)
  expect(events).toEqual([])
})

it('polls without EventSource and cleans up the timer', () => {
  vi.stubGlobal('EventSource', undefined)
  const { state, events } = setup()
  expect(state.status.value).toBe('polling')
  vi.advanceTimersByTime(1000)
  expect(events).toEqual(['poll'])
})

it('changing slug closes previous stream and opens the new URL', async () => {
  const { slug } = setup(), first = FakeES.all[0]!
  slug.value = 'next'; await nextTick()
  expect(first.closed).toBe(true)
  expect(FakeES.all[1]!.url).toContain('/wishlists/next/events')
})
