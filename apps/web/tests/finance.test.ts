import { describe, expect, it } from 'vitest'
import { delta, financeTab, fmtDay, perPayer } from '../app/utils/finance'

describe('finance utils', () => {
  it('financeTab 未知值回 overview', () => {
    expect(financeTab('x')).toBe('overview')
    expect(financeTab(undefined)).toBe('overview')
    expect(financeTab('risk')).toBe('risk')
  })
  it('delta 為百分比，前期為 0 回 null', () => {
    expect(delta(120, 100)).toBe(20)
    expect(delta(5, 0)).toBeNull()
    expect(delta(0, 100)).toBe(-100)
  })
  it('perPayer 無付費用戶回 null', () => {
    expect(perPayer(1000, 0)).toBeNull()
    expect(perPayer(1000, 4)).toBe(250)
  })
  it('fmtDay 固定用台北時間', () => {
    expect(fmtDay('2026-10-30T16:30:00Z', 'en-US')).toBe('10/31/2026')
  })
})
