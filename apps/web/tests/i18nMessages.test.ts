import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { baseCompile } from '@intlify/message-compiler'
import { describe, expect, it } from 'vitest'
import { createI18n } from 'vue-i18n'

// vue-i18n 訊息語法：@ 是連結訊息、| 是複數分隔、{} 是插值。字面字元要寫成 {'@'}、{'|'}：
// @ 未跳脫會編譯失敗（整頁渲染崩潰），| 未跳脫會被當成複數，t() 只回傳 | 前那段。
const root = fileURLToPath(new URL('../i18n/locales', import.meta.url))

function* messages(o: unknown, key = ''): Generator<[string, string]> {
  if (typeof o === 'string') yield [key, o]
  else if (o && typeof o === 'object') for (const [k, v] of Object.entries(o)) yield* messages(v, key ? `${key}.${k}` : k)
}

const all = readdirSync(root).flatMap(locale =>
  readdirSync(join(root, locale)).flatMap(file =>
    [...messages(JSON.parse(readFileSync(join(root, locale, file), 'utf8')))].map(([k, v]) => [`${locale}/${file} ${k}`, v] as const)))

describe('i18n 語系訊息', () => {
  it('每一則都能被 vue-i18n 編譯', () => {
    const broken = all.filter(([, v]) => {
      let failed = false
      baseCompile(v, { onError: () => { failed = true } })
      return failed
    }).map(([k]) => k)
    expect(broken).toEqual([])
  })

  // 刻意的複數「{n} item | {n} items」：每段都有插值才放行
  it('沒有未跳脫的 |（字面請寫 {\'|\'}）', () => {
    const stray = (v: string) => { const s = v.replaceAll("{'|'}", ''); return s.includes('|') && !s.split('|').every(p => p.includes('{')) }
    expect(all.filter(([, v]) => stray(v)).map(([k]) => k)).toEqual([])
  })
})

describe('英文單複數', () => {
  const load = (l: string) => JSON.parse(readFileSync(join(root, l, 'guest.json'), 'utf8'))
  const { global: { t } } = createI18n({ legacy: false, locale: 'en', messages: { en: load('en'), vi: load('vi') } })
  it('1 用單數、其他用複數；沒有複數的語系照原句', () => {
    expect(t('guest.explore.itemCount', { n: 1 }, 1)).toBe('1 item')
    expect(t('guest.explore.itemCount', { n: 3 }, 3)).toBe('3 items')
    expect(t('guest.explore.itemCount', { n: 1 }, { plural: 1, locale: 'vi' })).toBe('1 món')
  })
})
