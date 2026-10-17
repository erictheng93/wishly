import { describe, expect, it } from 'vitest'
import { barRows, fmtCount, funnelSteps, mean, pct, retentionRows } from '../app/utils/analytics'

describe('analytics helpers', () => {
  it('pct guards null and zero', () => {
    expect(pct(1, 4)).toBe('25%')
    expect(pct(1, 3, 1)).toBe('33.3%')
    expect(pct(null, 4)).toBe('–')
    expect(pct(1, null)).toBe('–')
    expect(pct(0, 0)).toBe('–')
    expect(pct(0, 5)).toBe('0%')
  })
  it('fmtCount shows <5 for suppressed', () => {
    expect(fmtCount(null)).toBe('<5')
    expect(fmtCount(0)).toBe('0')
    expect(fmtCount(1234)).toBe('1,234')
  })
  it('barRows excludes null from share, sorts null last, maps labels', () => {
    const r = barRows({ a: null, b: 30, c: 10 }, { b: '乙' })
    expect(r.map(x => x.key)).toEqual(['b', 'c', 'a'])
    expect(r[0]).toMatchObject({ label: '乙', share: '75%', w: 100 })
    expect(r[2]).toMatchObject({ v: null, share: '–', w: 0 })
  })
  it('barRows keeps order when sort=false and handles undefined', () => {
    expect(barRows({ '01': 1, '02': 5 }, {}, false).map(x => x.key)).toEqual(['01', '02'])
    expect(barRows(undefined)).toEqual([])
  })
  it('funnelSteps conversion and zero guard', () => {
    const s = funnelSteps({ signed_up: 100, created_list: 50, published_list: 10, received_claim: 0 })
    expect(s.map(x => x.step)).toEqual(['–', '50%', '20%', '0%'])
    expect(s[1]!.w).toBe(50)
    expect(funnelSteps({ signed_up: 0, created_list: 0, published_list: 0, received_claim: 0 }).map(x => x.step)).toEqual(['–', '–', '–', '–'])
  })
  it('mean to 1 decimal, empty guard', () => {
    expect(mean([1, 2, 4])).toBe('2.3')
    expect(mean([0, 0])).toBe('0.0')
    expect(mean([])).toBe('–')
  })
  it('retentionRows rates, zero cohort, missing keys', () => {
    const r = retentionRows({ d1: { cohort: 8, retained: 2 }, d7: { cohort: 3, retained: 1 }, d30: { cohort: 0, retained: 0 } })
    expect(r.map(x => x.label)).toEqual(['次日留存（第 1 天）', '第 7–13 天', '第 30–59 天'])
    expect(r.map(x => x.pct)).toEqual(['25%', '33%', '–'])
    expect(r[0]).toMatchObject({ cohort: 8, retained: 2, w: 25 })
    expect(r[2]!.w).toBe(0)
    const none = retentionRows(undefined)
    expect(none.map(x => [x.cohort, x.retained, x.pct, x.w])).toEqual([[0, 0, '–', 0], [0, 0, '–', 0], [0, 0, '–', 0]])
    expect(retentionRows({ d1: { cohort: 4, retained: 4 } })[1]).toMatchObject({ cohort: 0, pct: '–' })
  })
})
