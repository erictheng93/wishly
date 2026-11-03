import { readFileSync } from 'node:fs'
import { expect, it } from 'vitest'
import { OCCASIONS } from '../app/utils/catalog'
import { SCENE_LOCALES, SCENES, sceneBySlug, scenePath } from '../app/utils/scenes'
import { breadcrumbLd } from '../app/utils/seo'

const load = (l: string) => JSON.parse(readFileSync(new URL(`../i18n/locales/${l}/inspire.json`, import.meta.url), 'utf8')).inspire
const zh = load('zh-Hant')

it('情境落地頁結構：slug 唯一、場合對得上商品目錄', () => {
  expect(SCENES.length).toBeGreaterThanOrEqual(5)
  expect(new Set(SCENES.map(s => s.slug)).size).toBe(SCENES.length)
  for (const s of SCENES) {
    expect(s.slug).toMatch(/^[a-z-]+$/)
    if (s.occasion) expect(OCCASIONS).toContain(s.occasion)
  }
  expect(sceneBySlug('nope')).toBeUndefined()
})

it.each(['zh-Hant', ...SCENE_LOCALES])('%s 文案齊全、與繁中同結構、沒有未跳脫的 vue-i18n 特殊字元', (l) => {
  const m = load(l)
  expect(Object.keys(m.ui).sort()).toEqual(Object.keys(zh.ui).sort())
  expect(Object.keys(m.scenes).sort()).toEqual(SCENES.map(s => s.slug).sort())
  for (const { slug } of SCENES) {
    const s = m.scenes[slug], z = zh.scenes[slug]
    expect(s.seoTitle).toMatch(/ \{'\|'\} Wishly$/)
    for (const f of ['label', 'seoDesc', 'heading']) expect(s[f]).toBeTruthy()
    expect(s.intro.length).toBe(z.intro.length)
    expect(s.steps).toHaveLength(3)
    expect(s.ideas.length).toBe(z.ideas.length)
    expect(s.ideas.length).toBeGreaterThanOrEqual(8)
    expect(s.faq).toHaveLength(3)
  }
  // `|` 是複數分隔、`@` 是連結訊息、`{}` 是插值：只允許 {'|'}
  const leaves = (v: any): string[] => typeof v === 'string' ? [v] : Object.values(v).flatMap(leaves)
  for (const v of leaves(m)) expect(v.replaceAll(`{'|'}`, '')).not.toMatch(/[|@${}]/)
})

it('語系前綴網址：zh-Hant 無前綴', () => {
  expect(scenePath('birthday', 'zh-Hant')).toBe('/inspire/birthday')
  expect(scenePath('birthday', 'vi')).toBe('/vi/inspire/birthday')
})

it('麵包屑帶完整網址與順序', () => {
  expect(breadcrumbLd('https://wishly.tw', [['A', '/a'], ['B', '/a/b']]).itemListElement[1]).toEqual({ '@type': 'ListItem', position: 2, name: 'B', item: 'https://wishly.tw/a/b' })
})
