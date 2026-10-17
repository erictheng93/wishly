import { expect, it } from 'vitest'
import { loginUrl, safeRedirect } from '../app/utils/redirect'

it('safeRedirect keeps same-site paths with query and rejects open redirects', () => {
  expect(safeRedirect('/s/abc?x=1&y=2')).toBe('/s/abc?x=1&y=2')
  for (const bad of ['//evil.com', '/\\evil.com', 'https://evil.com', 'evil.com', '', undefined, null]) expect(safeRedirect(bad)).toBe('/dashboard')
  expect(safeRedirect('//evil.com', '/')).toBe('/')
})

it('loginUrl returns to the current share page, preserving query; unsafe paths fall back to /', () => {
  expect(loginUrl('/s/abc')).toEqual({ path: '/login', query: { redirect: '/s/abc' } })
  expect(loginUrl('/s/abc?pw=1')).toEqual({ path: '/login', query: { redirect: '/s/abc?pw=1' } })
  expect(loginUrl('//evil.com')).toEqual({ path: '/login', query: { redirect: '/' } })
})
