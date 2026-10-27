import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { CATEGORIES, COUNTIES, HAZARDS, URGENCIES, mapsUrl, taipeiToday } from '../app/utils/relief'

describe('relief utils', () => {
  it('縣市共 22 筆且用「臺」', () => {
    expect(COUNTIES.length).toBe(22)
    expect(COUNTIES.some(c => c.includes('台'))).toBe(false)
  })
  it('mapsUrl 編碼地址', () => {
    expect(mapsUrl('臺北市 中正路 1&2號')).toBe('https://www.google.com/maps/search/?api=1&query=' + encodeURIComponent('臺北市 中正路 1&2號'))
  })
  it('taipeiToday 為 YYYY-MM-DD', () => {
    expect(taipeiToday()).toMatch(/^\d{4}-\d{2}-\d{2}$/)
  })
})

describe('救災值的 i18n 標籤', () => {
  const groups = { hazard: HAZARDS, category: CATEGORIES, urgency: URGENCIES, siteStatus: ['active', 'paused', 'closed'] }
  for (const loc of ['zh-Hant', 'en', 'vi', 'id']) {
    it(`${loc} 每個值都有字串標籤`, () => {
      const r = JSON.parse(readFileSync(fileURLToPath(new URL(`../i18n/locales/${loc}/org.json`, import.meta.url)), 'utf8')).org.relief
      for (const [g, vals] of Object.entries(groups)) for (const v of vals) expect(typeof r[g][v], `${g}.${v}`).toBe('string')
      for (const k of ['categoryLabel', 'urgencyLabel', 'siteStatusLabel']) expect(typeof r[k]).toBe('string')
    })
  }
})
