import { describe, expect, it } from 'vitest'
import { eventLd, homeLd, ldScript, listLd, ownItems, robotsTxt, sitemapXml } from '../app/utils/seo'

describe('seo', () => {
  it('robots：正式站擋創建者路徑並指向 sitemap，非正式站全擋', () => {
    const r = robotsTxt('https://wishly.tw', true)
    expect(r).toContain('Disallow: /dashboard')
    expect(r).toContain('Sitemap: https://wishly.tw/sitemap.xml')
    expect(r).not.toMatch(/Disallow: \/s\b/)
    expect(robotsTxt('https://wishly.tw', false)).toBe('User-agent: *\nDisallow: /\n')
  })

  it('sitemap：去重並跳脫 XML', () => {
    const x = sitemapXml('https://wishly.tw', ['/', '/', '/s/a&b'])
    expect(x.match(/<url>/g)).toHaveLength(2)
    expect(x).toContain('<loc>https://wishly.tw/s/a&amp;b</loc>')
  })

  it('JSON-LD：使用者文字不能提早結束 script', () => {
    const s = ldScript(listLd('https://wishly.tw', 'x', { title: '</script><b>', items: [] }))
    expect(s.innerHTML).not.toContain('</')
    expect(JSON.parse(s.innerHTML).name).toBe('</script><b>')
  })

  it('清單 ItemList 排除支持品項', () => {
    const items = [{ title: 'A' }, { title: 'B', funding_mode: 'support' }, { title: 'C', funding_mode: 'crowdfund' }]
    expect(ownItems(items)).toHaveLength(2)
    expect(listLd('https://wishly.tw', 'x', { title: 't', items }).itemListElement.map(i => i.name)).toEqual(['A', 'C'])
  })

  it('事件與首頁結構', () => {
    const e = eventLd('https://wishly.tw', 'quake', { name: '地震', counties: ['花蓮縣'], occurred_on: '2026-04-03' })
    expect(e).toMatchObject({ '@type': 'Event', startDate: '2026-04-03', location: { address: { addressRegion: '花蓮縣' } } })
    const h = homeLd('https://wishly.tw', [{ q: 'Q', a: 'A' }])
    expect(h.map(x => x['@type'])).toEqual(['Organization', 'WebSite', 'FAQPage'])
  })
})
