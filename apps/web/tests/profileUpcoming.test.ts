import { describe, expect, it } from 'vitest'
import { daysUntil, localDay, sortLists, upcoming } from '../app/utils/profile'

const w = (slug: string, event_date: string | null, status = 'active') => ({ slug, event_date, status })
const today = '2026-11-03'

describe('profile upcoming', () => {
  const ls = [w('a', null), w('past', '2026-10-01'), w('far', '2026-12-25'), w('near', '2026-11-10'), w('draft', '2026-11-04', 'draft'), w('today', '2026-11-03')]
  it('upcoming picks nearest active list on or after today', () => {
    expect(upcoming(ls, today)?.slug).toBe('today')
    expect(upcoming([w('a', null), w('p', '2026-01-01')], today)).toBeNull()
  })
  it('sortLists puts upcoming first ascending, rest keep order', () => {
    expect(sortLists(ls, today).map(x => x.slug)).toEqual(['today', 'near', 'far', 'a', 'past', 'draft'])
  })
  it('daysUntil and localDay', () => {
    expect(daysUntil('2026-11-10', today)).toBe(7)
    expect(daysUntil('2026-11-03', today)).toBe(0)
    expect(localDay(new Date(2026, 0, 5))).toBe('2026-01-05')
  })
})
