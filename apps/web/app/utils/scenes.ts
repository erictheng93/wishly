// 情境落地頁（/inspire/{slug}）：接「生日禮物清單」「結婚禮物清單」這類搜尋。
// 文案在 i18n/locales/<語系>/inspire.json（inspire.scenes.<slug>）；這裡只放結構。
// 全站是 no_prefix（語系存 cookie，Googlebot 只看得到 zh-Hant），所以這幾頁另有語系前綴網址 /en|vi|id/inspire/{slug}（nuxt.config 的 pages:extend）
// 讓各語系能被搜尋收錄；無前綴網址是 zh-Hant 正本，但對有語系 cookie 的使用者照樣顯示其語系。
import type { OCCASIONS } from './catalog'

export type Scene = {
  slug: string
  occasion?: typeof OCCASIONS[number] // 對應商品目錄的場合篩選；沒有就不顯示精選商品
  demo?: string // 範例清單 slug（db/seed/demo.sql）
}

export const SCENES: Scene[] = [
  { slug: 'birthday', occasion: 'birthday', demo: 'demobirth0' },
  { slug: 'wedding', demo: 'demowedd00' },
  { slug: 'baby', occasion: 'full_month' },
  { slug: 'housewarming', occasion: 'housewarming' },
  { slug: 'christmas', occasion: 'holiday', demo: 'demogift00' },
]

// 有前綴網址的語系（zh-Hant 是無前綴正本）；要與 nuxt.config 的 SCENE_LOCALES 一致
export const SCENE_LOCALES = ['en', 'vi', 'id'] as const

export const sceneBySlug = (slug: string) => SCENES.find(s => s.slug === slug)
export const scenePath = (slug: string, locale: string) => `${(SCENE_LOCALES as readonly string[]).includes(locale) ? `/${locale}` : ''}/inspire/${slug}`
