import { expect, it } from 'vitest'
import { maskEmail } from '../app/utils/mask'

it('maskEmail 只留第一個字與網域', () => {
  expect(maskEmail('alice@example.com')).toBe('a***@example.com')
  expect(maskEmail('x@y.tw')).toBe('x***@y.tw')
  expect(maskEmail('')).toBe('')
})
