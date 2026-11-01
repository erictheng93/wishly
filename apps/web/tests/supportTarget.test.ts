import { describe, expect, it } from 'vitest'
import { isCurrentSlug, parseListSlug } from '../app/utils/supportTarget'

describe('parseListSlug', () => {
  it('解析連結與純代稱', () => {
    expect(parseListSlug('https://wishly.tw/s/abc123?x=1')).toBe('abc123')
    expect(parseListSlug('  abc123 ')).toBe('abc123')
    expect(parseListSlug('https://evil.test/x y')).toBeNull()
    expect(parseListSlug('')).toBeNull()
  })
})

describe('isCurrentSlug', () => {
  it('回應只在仍對應目前輸入時有效', () => {
    expect(isCurrentSlug('https://wishly.tw/s/a1', 'a1')).toBe(true)
    expect(isCurrentSlug('https://wishly.tw/s/b2', 'a1')).toBe(false)
    expect(isCurrentSlug('', 'a1')).toBe(false)
  })
})
