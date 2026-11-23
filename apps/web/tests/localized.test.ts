import { expect, it } from 'vitest'
import { apiLocale, isLocalizable, localizedPath, stripLocale } from '../app/utils/localized'

it('只有可收錄的公開頁加語系前綴', () => {
  for (const p of ['/', '/explore', '/inspire', '/inspire/birthday', '/brands', '/brands/x', '/s/abc', '/e', '/e/x', '/org', '/o/x', '/terms', '/privacy']) expect(isLocalizable(p), p).toBe(true)
  for (const p of ['/dashboard', '/lists/1/edit', '/u/me', '/org/apply', '/s', '/s/a/b', '/explorer', '/me/claims', '/login']) expect(isLocalizable(p), p).toBe(false)
})

it('localizedPath / stripLocale', () => {
  expect(localizedPath('/', 'vi')).toBe('/vi')
  expect(localizedPath('/s/abc', 'id')).toBe('/id/s/abc')
  expect(localizedPath('/vi/s/abc', 'en')).toBe('/en/s/abc')
  expect(localizedPath('/vi/s/abc', 'zh-Hant')).toBe('/s/abc')
  expect(localizedPath('/dashboard', 'vi')).toBe('/dashboard')
  expect(stripLocale('/vi')).toBe('/')
  expect(stripLocale('/videos')).toBe('/videos') // 前綴要整段相符
  expect(stripLocale('/id/e')).toBe('/e')
})

it('zh-Hans 前綴一律小寫，大小寫混用的網址也能去前綴', () => {
  expect(localizedPath('/s/abc', 'zh-Hans')).toBe('/zh-hans/s/abc')
  expect(localizedPath('/', 'zh-Hans')).toBe('/zh-hans')
  expect(localizedPath('/zh-hans/s/abc', 'zh-Hant')).toBe('/s/abc')
  expect(stripLocale('/zh-Hans/explore')).toBe('/explore')
  expect(stripLocale('/zh-hans')).toBe('/')
  expect(stripLocale('/zh-hansx')).toBe('/zh-hansx')
})

it('apiLocale：中文送地區碼', () => {
  expect(apiLocale('zh-Hant')).toBe('zh-TW')
  expect(apiLocale('zh-Hans')).toBe('zh-CN')
  expect(apiLocale('vi')).toBe('vi')
})
