// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { ref } from 'vue'
import { useOrgMode } from '../app/composables/useOrgMode'

const navigate = vi.fn()
const user = ref<any>(null)
const states = new Map<string, any>()
beforeEach(() => {
  states.clear()
  user.value = { orgs: [{ id: 'o1', name: 'A', role: 'owner', verification_status: 'approved' }] }
  vi.stubGlobal('useState', (k: string, init: () => any) => states.get(k) ?? states.set(k, ref(init())).get(k))
  vi.stubGlobal('useAuth', () => ({ user }))
  vi.stubGlobal('navigateTo', navigate)
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks(); localStorage.clear() })

it('存的 id 不在 orgs 裡 → current 回退 null', () => {
  localStorage.setItem('wishly-org-mode', 'gone')
  expect(useOrgMode().current.value).toBeNull()
})

it('存的 id 在 orgs 裡 → 沿用', () => {
  localStorage.setItem('wishly-org-mode', 'o1')
  expect(useOrgMode().current.value).toBe('o1')
})

it('switchTo 寫入 storage 並導頁；切回個人清除並回 dashboard', () => {
  const m = useOrgMode()
  m.switchTo('o1')
  expect(localStorage.getItem('wishly-org-mode')).toBe('o1')
  expect(m.current.value).toBe('o1')
  expect(navigate).toHaveBeenLastCalledWith('/orgs/o1')
  m.switchTo(null)
  expect(localStorage.getItem('wishly-org-mode')).toBeNull()
  expect(navigate).toHaveBeenLastCalledWith('/dashboard')
})

it('adopt 只對齊身分、不導頁', () => {
  const m = useOrgMode()
  m.adopt('o1')
  expect(m.current.value).toBe('o1')
  expect(localStorage.getItem('wishly-org-mode')).toBe('o1')
  expect(navigate).not.toHaveBeenCalled()
})

it('localStorage 丟例外也不壞', () => {
  vi.stubGlobal('localStorage', { getItem: () => { throw new Error('x') }, setItem: () => { throw new Error('x') }, removeItem: () => { throw new Error('x') } })
  const m = useOrgMode()
  expect(m.current.value).toBeNull()
  expect(() => m.switchTo('o1')).not.toThrow()
  expect(m.current.value).toBe('o1')
})
