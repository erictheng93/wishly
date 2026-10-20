import { expect, it } from 'vitest'
import { sanitizeItemPrefill } from '../app/utils/itemPrefill'

it('sanitizeItemPrefill trims, clamps and keeps valid fields', () => {
  expect(sanitizeItemPrefill({ title: ' 馬克杯 ', brand: 'B', spec: 'S', product_url: 'https://a.com/x?y=1', unit_price_amount: '350' }))
    .toEqual({ title: '馬克杯', brand: 'B', spec: 'S', product_url: 'https://a.com/x?y=1', unit_price_amount: 350 })
  expect(sanitizeItemPrefill({ title: 'x'.repeat(500) }).title).toHaveLength(120)
})

it('sanitizeItemPrefill drops arrays, non-http urls and bad prices', () => {
  expect(sanitizeItemPrefill({ title: ['a', 'b'], brand: '  ' })).toEqual({})
  for (const u of ['javascript:alert(1)', 'ftp://a.com', 'not a url', '//a.com']) expect(sanitizeItemPrefill({ product_url: u })).toEqual({})
  for (const p of ['-5', '3.5', '1e3', 'abc', '', ['1']]) expect(sanitizeItemPrefill({ unit_price_amount: p })).toEqual({})
  expect(sanitizeItemPrefill({ unit_price_amount: '0' })).toEqual({ unit_price_amount: 0 })
})

it('sanitizeItemPrefill keeps ref as catalog_ref only when it matches the pattern', () => {
  expect(sanitizeItemPrefill({ ref: 'p_1-A' })).toEqual({ catalog_ref: 'p_1-A' })
  for (const r of ['', 'a b', 'x'.repeat(65), '中文', ['a']]) expect(sanitizeItemPrefill({ ref: r })).toEqual({})
})
