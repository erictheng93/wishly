import { describe, expect, it } from 'vitest'
import { daysInMonth, formatBirthday, toBirthday } from '../app/utils/birthday'

describe('birthday', () => {
  it('daysInMonth uses a leap year so 02-29 is selectable', () => {
    expect([daysInMonth('02'), daysInMonth('04'), daysInMonth('12'), daysInMonth('')]).toEqual([29, 30, 31, 31])
  })
  it('toBirthday: both empty clears, one side is invalid', () => {
    expect(toBirthday('', '')).toBeNull()
    expect(toBirthday('05', '')).toBeUndefined()
    expect(toBirthday('', '01')).toBeUndefined()
  })
  it('toBirthday joins MM-DD and clamps day to month length', () => {
    expect(toBirthday('05', '01')).toBe('05-01')
    expect(toBirthday('02', '31')).toBe('02-29')
    expect(toBirthday('04', '31')).toBe('04-30')
  })
  it('formatBirthday drops leading zeros', () => {
    expect(formatBirthday('05-01')).toBe('5 月 1 日')
    expect(formatBirthday('12-25')).toBe('12 月 25 日')
  })
})
