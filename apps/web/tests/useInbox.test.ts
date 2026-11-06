import { afterEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { pickToast } from '../app/utils/inboxStream'
import { useInbox } from '../app/composables/useInbox'

afterEach(() => vi.unstubAllGlobals())

it('reset 之後才回來的進行中 refresh 會被丟棄', async () => {
  const store = new Map<string, any>()
  let resolve!: (v: any) => void
  const api = vi.fn(() => new Promise(r => { resolve = r }))
  vi.stubGlobal('useState', (k: string, init: () => any) => { if (!store.has(k)) store.set(k, ref(init())); return store.get(k) })
  vi.stubGlobal('useApi', () => ({ api }))
  vi.stubGlobal('pickToast', pickToast)
  const inbox = useInbox()
  const p = inbox.refresh()
  inbox.reset()
  resolve({ items: [{ id: 'a' }], unread: 3 })
  await p
  expect(inbox.unread.value).toBe(0)
  expect(inbox.latest.value).toEqual([])
  expect(inbox.toast.value).toBeNull()
  expect(api).toHaveBeenCalledTimes(1)
})
