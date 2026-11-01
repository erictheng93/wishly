import { describe, expect, it } from 'vitest'
import { parseListSlug } from '../app/utils/supportTarget'

describe('parseListSlug', () => {
  it('解析連結與純代稱', () => {
    expect(parseListSlug('https://wishly.tw/s/abc123?x=1')).toBe('abc123')
    expect(parseListSlug('  abc123 ')).toBe('abc123')
    expect(parseListSlug('https://evil.test/x y')).toBeNull()
    expect(parseListSlug('')).toBeNull()
  })
})
