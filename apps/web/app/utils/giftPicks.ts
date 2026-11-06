import { nativeHeaders } from './native'
import type { CatalogProduct, GiftPick } from './catalog'
// 依對方興趣推薦的精選商品；帶憑證（依檢視者與對方的關係）、不可快取
export async function fetchGiftPicks(apiBase: string, handleOrId: string, opts: { occasion?: string; limit?: number } = {}): Promise<{ basis: 'interests' | 'fallback'; products: GiftPick[]; excluded: string[] }> {
  const query: Record<string, string | number> = {}
  if (opts.occasion) query.occasion = opts.occasion
  if (opts.limit) query.limit = opts.limit
  return $fetch(`${apiBase}/api/v1/users/${encodeURIComponent(handleOrId)}/gift-picks`, { query, credentials: 'include', cache: 'no-store', headers: nativeHeaders() })
}
// picks 在前，其餘以 ref 去重接在後面；excluded（對方不想收到）的 ref 一律拿掉
export function mergePicks(picks: GiftPick[], rest: CatalogProduct[], excluded: string[] = []): CatalogProduct[] {
  const seen = new Set([...picks.map(p => p.ref), ...excluded])
  return [...picks, ...rest.filter(p => !seen.has(p.ref))]
}
// CatalogGrid 的 matched prop：ref → 符合的興趣
export const matchedMap = (picks: GiftPick[]): Record<string, string[]> => Object.fromEntries(picks.map(p => [p.ref, p.matched]))
