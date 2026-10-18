import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { baseCompile } from '@intlify/message-compiler'
import { describe, expect, it } from 'vitest'

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

  it('沒有未跳脫的 |（字面請寫 {\'|\'}）', () => {
    expect(all.filter(([, v]) => v.replaceAll("{'|'}", '').includes('|')).map(([k]) => k)).toEqual([])
  })
})
