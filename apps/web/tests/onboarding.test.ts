import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { computed, ref } from 'vue'
import { onTourPage, tourTarget, useOnboarding } from '../app/composables/useOnboarding'

const user = ref<any>(null)
const api = vi.fn()
const states = new Map<string, any>()
beforeEach(() => {
  states.clear()
  user.value = { id: 'u1', onboarding_done: false }
  vi.stubGlobal('useState', (k: string, init: () => any) => states.get(k) ?? states.set(k, ref(init())).get(k))
  vi.stubGlobal('computed', computed)
  vi.stubGlobal('useAuth', () => ({ user }))
  vi.stubGlobal('useApi', () => ({ api }))
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks() })

const state = (current: string | null, extra: any = {}) => ({ done: false, current, wishlist_id: null, steps: [], ...extra })

it('每一步帶到對的頁面；加品項 / 發佈沒有清單時回清單列表', () => {
  expect(tourTarget('profile')).toBe('/welcome')
  expect(tourTarget('wishlist')).toBe('/lists/new')
  expect(tourTarget('item', 'w1')).toBe('/lists/w1/edit')
  expect(tourTarget('publish', null)).toBe('/dashboard')
  expect(tourTarget('give')).toBe('/explore')
})

it('在該步驟的頁面才顯示「怎麼做」', () => {
  expect(onTourPage('wishlist', '/lists/new')).toBe(true)
  expect(onTourPage('wishlist', '/dashboard')).toBe(false)
  expect(onTourPage('item', '/lists/w1/edit', 'w1')).toBe(true)
  expect(onTourPage('item', '/lists/w2/edit', 'w1')).toBe(false)
  expect(onTourPage('publish', '/lists/w2/edit', null)).toBe(true)
  expect(onTourPage('give', '/s/abc')).toBe(true)
  expect(onTourPage('give', '/explore')).toBe(true)
  expect(onTourPage('give', '/dashboard')).toBe(false)
})

it('只有後端明確回 onboarding_done=false 才抓導覽狀態', async () => {
  user.value = { id: 'u1' } // 舊版 API 沒有欄位
  expect(await useOnboarding().refresh()).toBeNull()
  user.value = { id: 'u1', onboarding_done: true }
  expect(await useOnboarding().refresh()).toBeNull()
  expect(api).not.toHaveBeenCalled()
  user.value = { id: 'u1', onboarding_done: false }
  api.mockResolvedValueOnce(state('profile'))
  const o = useOnboarding()
  await o.refresh()
  expect(api).toHaveBeenCalledWith('/me/onboarding', { noRedirect: true })
  expect(o.active.value).toBe(true)
})

it('抓取失敗保留前值', async () => {
  const o = useOnboarding()
  api.mockResolvedValueOnce(state('item'))
  await o.refresh()
  api.mockRejectedValueOnce(new Error('offline'))
  await o.refresh()
  expect(o.state.value?.current).toBe('item')
})

it('跳過 / 重新開始同步 user.onboarding_done', async () => {
  const o = useOnboarding()
  api.mockResolvedValueOnce(state('profile', { done: true }))
  await o.finish(true)
  expect(api).toHaveBeenLastCalledWith('/me/onboarding', { method: 'PATCH', body: { done: true }, noRedirect: true })
  expect(user.value.onboarding_done).toBe(true)
  expect(o.active.value).toBe(false)
  api.mockResolvedValueOnce(state('profile'))
  await o.finish(false)
  expect(user.value.onboarding_done).toBe(false)
  expect(o.active.value).toBe(true)
})
