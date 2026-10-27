import { describe, expect, it } from 'vitest'
import { COUNTIES, mapsUrl, taipeiToday } from '../app/utils/relief'

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
