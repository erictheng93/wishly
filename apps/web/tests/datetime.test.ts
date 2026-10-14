import { describe, expect, it } from 'vitest'
import { closesAtChange, fromLocalInput, toLocalInput } from '../app/utils/datetime'

describe('closes_at datetime-local 轉換', () => {
  it('空值 ⇄ 空字串 / null', () => {
    expect(toLocalInput(null)).toBe('')
    expect(toLocalInput(undefined)).toBe('')
    expect(fromLocalInput('')).toBeNull()
  })
  it('來回轉換保留到分鐘', () => {
    const iso = '2999-05-06T07:08:00.000Z'
    expect(toLocalInput(iso)).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/)
    expect(fromLocalInput(toLocalInput(iso))).toBe(iso)
  })
  it('未變動不送（含帶秒數的原值）；清空送 null；改動送 ISO', () => {
    const orig = '2999-05-06T07:08:09.000Z'
    expect(closesAtChange(toLocalInput(orig), orig)).toBeUndefined()
    expect(closesAtChange('', orig)).toBeNull()
    expect(closesAtChange('', null)).toBeUndefined()
    expect(closesAtChange('2999-06-01T10:00', orig)).toBe(new Date('2999-06-01T10:00').toISOString())
  })
})
