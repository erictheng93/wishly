import { beforeEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { useFriendRequests } from '../app/composables/useFriendRequests'

const states = new Map<string, any>()
const api = vi.fn()
beforeEach(() => {
  states.clear(); api.mockReset(); api.mockResolvedValue({ incoming: 3 }); vi.useFakeTimers()
  vi.stubGlobal('useState', (k: string, init: () => any) => { if (!states.has(k)) states.set(k, ref(init())); return states.get(k) })
  vi.stubGlobal('useAuth', () => ({ user: ref({ id: 'u1' }) }))
  vi.stubGlobal('useApi', () => ({ api }))
})

it('60 秒內不重打；force 或逾時才打；同時進行共用一個請求', async () => {
  const { count, refresh } = useFriendRequests()
  await Promise.all([refresh(), refresh()])
  expect(api).toHaveBeenCalledTimes(1); expect(count.value).toBe(3)
  await refresh(); expect(api).toHaveBeenCalledTimes(1)
  await refresh(true); expect(api).toHaveBeenCalledTimes(2)
  vi.advanceTimersByTime(61_000)
  await refresh(); expect(api).toHaveBeenCalledTimes(3)
  vi.useRealTimers()
})

it('失敗保留前值', async () => {
  const { count, refresh } = useFriendRequests()
  await refresh(true)
  api.mockRejectedValueOnce(new Error('x'))
  await refresh(true); expect(count.value).toBe(3)
  vi.useRealTimers()
})
