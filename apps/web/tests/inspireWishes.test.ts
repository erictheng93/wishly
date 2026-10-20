import { afterEach, expect, it, vi } from 'vitest'
import { fetchWishes, wishLeft, wishesUrl } from '../app/utils/inspireWishes'

afterEach(() => vi.unstubAllGlobals())
it('wishesUrl: scope 空 = 不帶參數，ref 會編碼', () => {
  expect(wishesUrl('http://a', 'p1', '')).toBe('http://a/api/v1/inspire/p1/wishes')
  expect(wishesUrl('http://a', 'p1', 'selected')).toBe('http://a/api/v1/inspire/p1/wishes?scope=selected')
  expect(wishesUrl('http://a', 'a/b', '')).toContain('a%2Fb')
})
it('wishLeft 不為負', () => {
  expect(wishLeft({ qty_needed: 3, qty_claimed: 1 })).toBe(2)
  expect(wishLeft({ qty_needed: 1, qty_claimed: 2 })).toBe(0)
})
it('fetchWishes 帶憑證並取 wishes', async () => {
  const f = vi.fn().mockResolvedValue({ wishes: [{ a: 1 }] })
  vi.stubGlobal('$fetch', f)
  expect(await fetchWishes('http://a', 'p1', 'friends')).toEqual([{ a: 1 }])
  expect(f.mock.calls[0]![1]).toMatchObject({ credentials: 'include' })
})
