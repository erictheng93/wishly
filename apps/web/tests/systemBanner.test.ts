// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import SystemBanner from '../app/components/SystemBanner.vue'
import { noteUnavailable, useSysStatus } from '../app/composables/useSysStatus'
import { flush, mount } from './vueHarness'

const state = ref<any>(null)
const fetch = vi.fn()
let mounted: ReturnType<typeof mount> | undefined

beforeEach(() => {
  vi.useFakeTimers()
  state.value = { kind: 'readOnly', until: Date.now() + 60_000 } // 已在維護中，掛載時不先查
  for (const [name, value] of Object.entries({
    onBeforeUnmount, onMounted, watch, noteUnavailable, useSysStatus,
    useState: () => state,
    useRuntimeConfig: () => ({ public: { apiBase: 'http://api' } }),
    $fetch: fetch,
  })) vi.stubGlobal(name, value)
  mounted = mount(SystemBanner)
})

afterEach(() => {
  mounted?.unmount()
  vi.unstubAllGlobals()
  vi.useRealTimers()
  fetch.mockReset()
})

it('維護橫幅到期時剛好離線：查詢失敗後仍會再排程，恢復連線後橫幅消失', async () => {
  vi.stubGlobal('navigator', { onLine: false })
  fetch.mockRejectedValueOnce(Object.assign(new Error('offline'), { statusCode: 0 }))
  await vi.advanceTimersByTimeAsync(60_000)
  await flush()
  expect(fetch).toHaveBeenCalledTimes(1)
  expect(useSysStatus().value?.kind).toBe('readOnly')
  expect(mounted!.el.querySelector('.sys-banner')).not.toBeNull()

  vi.stubGlobal('navigator', { onLine: true })
  fetch.mockResolvedValueOnce({ read_only: false })
  await vi.advanceTimersByTimeAsync(30_000)
  await flush()
  expect(fetch).toHaveBeenCalledTimes(2)
  expect(useSysStatus().value).toBeNull()
  expect(mounted!.el.querySelector('.sys-banner')).toBeNull()
})
