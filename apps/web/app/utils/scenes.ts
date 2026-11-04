// 情境落地頁（/inspire/{slug}）：接「生日禮物清單」「結婚禮物清單」這類搜尋。
// 文案在 i18n/locales/<語系>/inspire.json（inspire.scenes.<slug>）；這裡只放結構。
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

export const sceneBySlug = (slug: string) => SCENES.find(s => s.slug === slug)
