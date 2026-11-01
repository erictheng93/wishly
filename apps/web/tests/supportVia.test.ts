import { beforeEach, expect, it, vi } from 'vitest'
import { currentVia, rememberVia } from '../app/utils/supportVia'

const A = '11111111-1111-4111-8111-111111111111', B = '22222222-2222-4222-8222-222222222222'
const DAY = 86400000
const store = new Map<string, string>()
beforeEach(() => {
  store.clear()
  vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) })
})

it('存入後可取回，第二次覆蓋第一次', () => {
  rememberVia('s', A, 0)
  expect(currentVia('s', 1)).toBe(A)
  rememberVia('s', B, 2)
  expect(currentVia('s', 3)).toBe(B)
})
it('7 天整有效，多 1 毫秒失效', () => {
  rememberVia('s', A, 0)
  expect(currentVia('s', 7 * DAY)).toBe(A)
  expect(currentVia('s', 7 * DAY + 1)).toBeUndefined()
})
it('不同 slug 互不影響', () => {
  rememberVia('s', A, 0)
  expect(currentVia('t', 0)).toBeUndefined()
})
it('非 UUID 不寫入', () => {
  for (const raw of ['x', '', undefined, null, 5, ['a'], A + 'x', `${A}\n`]) rememberVia('s', raw, 0)
  expect(store.size).toBe(0)
})
it('內容損毀或 localStorage 拋例外時回 undefined', () => {
  store.set('ws_via:s', '{bad')
  expect(currentVia('s', 0)).toBeUndefined()
  store.set('ws_via:s', JSON.stringify({ id: 'x', at: 0 }))
  expect(currentVia('s', 0)).toBeUndefined()
  vi.stubGlobal('localStorage', { getItem: () => { throw new Error('denied') }, setItem: () => { throw new Error('denied') } })
  expect(() => rememberVia('s', A, 0)).not.toThrow()
  expect(currentVia('s', 0)).toBeUndefined()
})
