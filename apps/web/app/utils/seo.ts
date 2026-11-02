// SEO 共用：robots.txt、sitemap.xml 與 JSON-LD 的純函式（server route 與頁面共用，方便單測）

// 品項少於此數的公開清單不給收錄：內容太薄的頁面會拉低整站評價
export const INDEX_MIN_ITEMS = 3

// 只放創建者 / 帳號相關路徑；/s、/e、/o、/explore 等公開頁要讓爬蟲進
const DISALLOW = ['/dashboard', '/lists', '/me', '/settings', '/welcome', '/friends', '/invite', '/u/', '/admin', '/login', '/register', '/forgot-password', '/org/apply', '/org/invite', '/orgs', '/unsubscribe', '/api/']

export function robotsTxt(siteUrl: string, indexable: boolean): string {
  if (!indexable) return 'User-agent: *\nDisallow: /\n'
  return `User-agent: *\n${DISALLOW.map(p => `Disallow: ${p}`).join('\n')}\n\nSitemap: ${siteUrl}/sitemap.xml\n`
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&apos;')

export function sitemapXml(siteUrl: string, paths: string[]): string {
  const urls = [...new Set(paths)].map(p => `<url><loc>${esc(siteUrl + p)}</loc></url>`).join('')
  return `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${urls}</urlset>`
}

// 支持品項（指向別份清單）不算這份清單自己的內容
export const ownItems = (items: any[] | undefined) => (items ?? []).filter(i => i.funding_mode !== 'support')

// JSON-LD 以 <script type="application/ld+json"> 輸出；</ 跳脫避免使用者文字提早結束 script
export const ldScript = (data: object) => ({ type: 'application/ld+json' as const, innerHTML: JSON.stringify(data).replace(/</g, '\\u003c') })

export function listLd(siteUrl: string, slug: string, w: any) {
  return {
    '@context': 'https://schema.org', '@type': 'ItemList', name: w.title, url: `${siteUrl}/s/${slug}`,
    ...(w.description ? { description: w.description } : {}),
    itemListElement: ownItems(w.items).map((i, n) => ({ '@type': 'ListItem', position: n + 1, name: i.title })),
  }
}

export function eventLd(siteUrl: string, slug: string, ev: any) {
  return {
    '@context': 'https://schema.org', '@type': 'Event', name: ev.name, url: `${siteUrl}/e/${slug}`,
    ...(ev.description ? { description: ev.description } : {}),
    ...(ev.occurred_on ? { startDate: ev.occurred_on } : {}),
    eventStatus: 'https://schema.org/EventScheduled', eventAttendanceMode: 'https://schema.org/OfflineEventAttendanceMode',
    location: { '@type': 'Place', name: (ev.counties ?? []).join('、') || ev.name, address: { '@type': 'PostalAddress', addressRegion: (ev.counties ?? []).join('、'), addressCountry: 'TW' } },
  }
}

export function orgLd(siteUrl: string, slug: string, org: any) {
  return {
    '@context': 'https://schema.org', '@type': 'NGO', name: org.display_name, url: `${siteUrl}/o/${slug}`,
    ...(org.intro ? { description: org.intro } : {}),
    ...(org.logo ? { logo: org.logo } : {}),
  }
}

export function homeLd(siteUrl: string, faq: { q: string, a: string }[]) {
  return [
    { '@context': 'https://schema.org', '@type': 'Organization', name: 'Wishly', url: siteUrl, logo: `${siteUrl}/icons/icon-512.png` },
    { '@context': 'https://schema.org', '@type': 'WebSite', name: 'Wishly', url: siteUrl },
    faqLd(faq),
  ]
}

export const faqLd = (faq: { q: string, a: string }[]) => ({ '@context': 'https://schema.org', '@type': 'FAQPage', mainEntity: faq.map(f => ({ '@type': 'Question', name: f.q, acceptedAnswer: { '@type': 'Answer', text: f.a } })) })

export const breadcrumbLd = (siteUrl: string, crumbs: [string, string][]) => ({
  '@context': 'https://schema.org', '@type': 'BreadcrumbList',
  itemListElement: crumbs.map(([name, path], n) => ({ '@type': 'ListItem', position: n + 1, name, item: siteUrl + path })),
})
