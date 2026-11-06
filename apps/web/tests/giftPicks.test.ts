import { afterEach, expect, it, vi } from 'vitest'
import { fetchGiftPicks, mergePicks } from '../app/utils/giftPicks'

afterEach(() => vi.unstubAllGlobals())
const p = (ref: string) => ({ ref, title: ref, price_amount: 1, product_url: 'u', recipients: [], occasions: [], brand: null })
it('mergePicks dedupes across pages', () => {
  const picks = [p('r2'), p('r5')].map(x => ({ ...x, matched: ['coffee'] }))
  expect(mergePicks(picks, ['r1', 'r2', 'r3', 'r5', 'r6'].map(p)).map(x => x.ref)).toEqual(['r2', 'r5', 'r1', 'r3', 'r6'])
})
it('mergePicks 拿掉 excluded，picks 仍在前', () => {
  const picks = [{ ...p('r2'), matched: ['coffee'] }]
  expect(mergePicks(picks, ['r1', 'r2', 'r3'].map(p), ['r1']).map(x => x.ref)).toEqual(['r2', 'r3'])
  expect(mergePicks([], ['r1', 'r3'].map(p), ['r1']).map(x => x.ref)).toEqual(['r3'])
})
it('fetchGiftPicks 帶 occasion / limit 與憑證', async () => {
  const f = vi.fn().mockResolvedValue({ basis: 'interests', products: [] })
  vi.stubGlobal('$fetch', f)
  await fetchGiftPicks('http://a', 'a b', { occasion: 'birthday', limit: 50 })
  expect(f.mock.calls[0]![0]).toBe('http://a/api/v1/users/a%20b/gift-picks')
  expect(f.mock.calls[0]![1]).toMatchObject({ query: { occasion: 'birthday', limit: 50 }, credentials: 'include', cache: 'no-store' })
})
