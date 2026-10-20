// 精選商品目錄（公開 API）：篩選詞彙與查詢組裝。詞彙 key 與後端一致，標籤走 i18n（guest.inspire.r.* / o.* / b.*）
export const RECIPIENTS = ['partner', 'elder', 'baby', 'friend'] as const
export const OCCASIONS = ['birthday', 'full_month', 'housewarming', 'holiday'] as const
export const BUDGETS = ['lt1000', '1000-3000', 'gt3000'] as const
export type CatalogFilter = { brand?: string; recipient?: string; occasion?: string; budget?: string }
export type CatalogProduct = {
  ref: string; title: string; spec?: string | null; note?: string | null; price_amount: number | null
  image_url?: string | null; product_url: string; recipients: string[]; occasions: string[]; brand: { slug: string; name: string } | null
}
// 空值不帶；page 1 也省略（利於 CDN 快取命中）
export function catalogQuery(f: CatalogFilter, page?: number | null) {
  const q: Record<string, string | number> = {}
  for (const k of ['brand', 'recipient', 'occasion', 'budget'] as const) if (f[k]) q[k] = f[k]!
  if (page && page > 1) q.page = page
  return q
}
