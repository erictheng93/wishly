import { expect, it } from 'vitest'
import { OCCASIONS } from '../app/utils/catalog'
import { SCENES, sceneBySlug } from '../app/utils/scenes'
import { breadcrumbLd } from '../app/utils/seo'

it('情境落地頁內容齊全、slug 唯一、場合對得上商品目錄', () => {
  expect(SCENES.length).toBeGreaterThanOrEqual(5)
  expect(new Set(SCENES.map(s => s.slug)).size).toBe(SCENES.length)
  for (const s of SCENES) {
    expect(s.slug).toMatch(/^[a-z-]+$/)
    expect(s.seoTitle).toContain('Wishly')
    expect(s.intro.length).toBeGreaterThanOrEqual(2)
    expect(s.steps).toHaveLength(3)
    expect(s.ideas.length).toBeGreaterThanOrEqual(8)
    expect(s.faq).toHaveLength(3)
    if (s.occasion) expect(OCCASIONS).toContain(s.occasion)
  }
  expect(sceneBySlug('nope')).toBeUndefined()
})

it('麵包屑帶完整網址與順序', () => {
  expect(breadcrumbLd('https://wishly.tw', [['A', '/a'], ['B', '/a/b']]).itemListElement[1]).toEqual({ '@type': 'ListItem', position: 2, name: 'B', item: 'https://wishly.tw/a/b' })
})
