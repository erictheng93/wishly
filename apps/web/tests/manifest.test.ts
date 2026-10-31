import { readFileSync } from 'node:fs'
import { expect, it } from 'vitest'

const load = (f: string) => JSON.parse(readFileSync(`${process.cwd()}/public/${f}`, 'utf8'))
const base = load('manifest.webmanifest')
const variants = { en: 'manifest.en.webmanifest', vi: 'manifest.vi.webmanifest', id: 'manifest.id.webmanifest' }

// 翻譯欄位與 lang 以外必須完全一致，否則各語系安裝出的 App 會不同
const strip = (m: any) => {
  const { name, short_name, description, lang, shortcuts, ...rest } = m
  void name; void short_name; void description; void lang; void shortcuts
  return rest
}

for (const [lang, file] of Object.entries(variants)) {
  it(`${file} 與預設 manifest 只差翻譯欄位`, () => {
    const m = load(file)
    expect(m.lang).toBe(lang)
    expect(m.id).toBe('/')
    expect(strip(m)).toEqual(strip(base))
    expect(m.shortcuts.map((s: any) => s.url)).toEqual(base.shortcuts.map((s: any) => s.url))
    for (const k of ['name', 'description']) expect(m[k]).not.toBe(base[k])
    for (const s of m.shortcuts) expect(s.name && s.short_name).toBeTruthy()
  })
}
