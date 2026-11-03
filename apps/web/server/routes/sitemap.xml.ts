import { INDEX_MIN_ITEMS, sitemapXml } from '../../app/utils/seo'
import { SCENE_LOCALES, SCENES, scenePath } from '../../app/utils/scenes'
import { seoIndexable } from '../utils/seoIndexable'

// shortcut: 最多抓 20 頁探索清單（約 480 份），超過時改由後端出 sitemap 索引分檔
const MAX_PAGES = 20

async function pages<T>(url: (p: number) => string, pick: (r: any) => T[]): Promise<T[]> {
  const out: T[] = []
  for (let p = 1; p && p <= MAX_PAGES;) {
    const r: any = await $fetch(url(p))
    out.push(...pick(r))
    p = r.next_page ?? 0
  }
  return out
}

export default defineEventHandler(async (event): Promise<string> => {
  if (!seoIndexable(event)) throw createError({ statusCode: 404 })
  const { apiBase, siteUrl } = useRuntimeConfig(event).public
  const api = `${apiBase}/api/v1/public`
  // 各來源獨立失敗：後端某支掛掉時仍回其餘網址，不讓整份 sitemap 變 5xx
  const safe = <T>(p: Promise<T[]>) => p.catch(() => [] as T[])
  const [status, lists, events, brands]: [any, string[], string[], string[]] = await Promise.all([
    $fetch<any>(`${api}/status`).catch(() => ({})),
    safe(pages(p => `${api}/wishlists?page=${p}`, r => r.wishlists.filter((w: any) => w.item_count >= INDEX_MIN_ITEMS).map((w: any) => `/s/${w.slug}`))),
    safe(pages(p => `${api}/relief-events?page=${p}`, r => r.events.map((e: any) => `/e/${e.slug}`))),
    safe($fetch<any>(`${api}/brands`).then((r: any) => r.brands.map((b: any) => `/brands/${b.slug}`))),
  ])
  const relief: string[] = status.orgs_enabled ? ['/e', '/org', ...events] : []
  setHeader(event, 'content-type', 'application/xml; charset=utf-8')
  setHeader(event, 'cache-control', 'public, max-age=3600')
  return sitemapXml(siteUrl, ['/', '/explore', '/inspire', ...['zh-Hant', ...SCENE_LOCALES].flatMap(l => SCENES.map(s => scenePath(s.slug, l))), '/brands', '/terms', '/privacy', ...relief, ...brands, ...lists])
})
