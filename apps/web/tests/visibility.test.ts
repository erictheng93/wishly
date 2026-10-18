import { describe, expect, it } from 'vitest'
import { VIS_OPTIONS, isRestricted, visDesc } from '../app/utils/visibility'

describe('visibility', () => {
  it('四種可見性皆有說明', () => {
    expect(VIS_OPTIONS.map(o => o.v)).toEqual(['public', 'friends', 'selected', 'private'])
    expect(visDesc('friends')).toContain('好友'); expect(visDesc('selected')).toContain('摯友'); expect(visDesc('x')).toBe('')
  })
  it('只有 public 不需 client 重抓', () => {
    expect(isRestricted('public')).toBe(false)
    expect(['friends', 'selected', 'private'].every(isRestricted)).toBe(true)
  })
})
