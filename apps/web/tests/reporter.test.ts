import { expect, it } from 'vitest'
import { reporterSuspicious } from '../app/utils/reporter'

it('駁回 ≥3 且駁回率 ≥50% 才標記', () => {
  expect(reporterSuspicious(null)).toBe(false)
  expect(reporterSuspicious({ total: 0, dismissed: 0 })).toBe(false)
  expect(reporterSuspicious({ total: 2, dismissed: 2 })).toBe(false)
  expect(reporterSuspicious({ total: 10, dismissed: 4 })).toBe(false)
  expect(reporterSuspicious({ total: 6, dismissed: 3 })).toBe(true)
  expect(reporterSuspicious({ total: 3, dismissed: 3 })).toBe(true)
})
