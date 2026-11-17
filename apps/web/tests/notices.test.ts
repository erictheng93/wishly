import { describe, expect, it } from 'vitest'
import { visibleNotices, type SysNotice } from '../app/utils/notices'

const n = (id: string, level: SysNotice['level'], locale: string | null = null): SysNotice => ({ id, title: id, body: '', level, locale, starts_at: '', ends_at: null })

describe('visibleNotices', () => {
  it('依語系過濾，null 代表全部語系', () => {
    const r = visibleNotices([n('a', 'info'), n('b', 'info', 'en'), n('c', 'info', 'vi')], 'en', [])
    expect(r.map(x => x.id)).toEqual(['a', 'b'])
  })
  it('已關閉的 info / warning 隱藏，critical 不可關閉', () => {
    const r = visibleNotices([n('a', 'info'), n('b', 'warning'), n('c', 'critical')], 'en', ['a', 'b', 'c'])
    expect(r.map(x => x.id)).toEqual(['c'])
  })
  it('undefined 視為空', () => expect(visibleNotices(undefined, 'en', [])).toEqual([]))
})
