import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { INTEREST_GROUPS } from '../app/utils/interests'

const flat = INTEREST_GROUPS.flatMap(g => g.tags)
const rs = new URL('../../api/src/interests.rs', import.meta.url)
const locales = readdirSync('i18n/locales')

describe('interests', () => {
  it('has 6 groups and 40 unique tags', () => {
    expect(INTEREST_GROUPS.map(g => g.id)).toEqual(['life', 'food', 'sports', 'arts', 'tech', 'family'])
    expect(flat).toHaveLength(40)
    expect(new Set(flat).size).toBe(40)
  })
  it.each(locales)('%s translates every tag and group', (l) => {
    const m = JSON.parse(readFileSync(`i18n/locales/${l}/interests.json`, 'utf8')).interests
    expect(flat.filter(t => !m.tag[t])).toEqual([])
    expect(INTEREST_GROUPS.filter(g => !m.group[g.id])).toEqual([])
  })
  // 後端 Task 3 合併後才有此檔
  it.skipIf(!existsSync(rs))('matches the backend INTERESTS constant', () => {
    const src = readFileSync(rs, 'utf8')
    const body = /INTERESTS[^=]*=\s*&\[([\s\S]*?)\];/.exec(src)![1]!
    expect([...body.matchAll(/"([a-z0-9-]+)"/g)].map(m => m[1])).toEqual(flat)
  })
})
