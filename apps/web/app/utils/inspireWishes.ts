import { nativeHeaders } from './native'
// 「誰想要這個」：精選商品 id → 想要它的心願。帶憑證才看得到朋友 / 摯友；回應不可快取（no-store）
export type WishScope = '' | 'public' | 'friends' | 'selected'
export const wishesUrl = (apiBase: string, ref: string, scope: WishScope) =>
  `${apiBase}/api/v1/inspire/${encodeURIComponent(ref)}/wishes${scope ? `?scope=${scope}` : ''}`
export const wishLeft = (i: { qty_needed: number; qty_claimed: number }) => Math.max(0, i.qty_needed - i.qty_claimed)
export async function fetchWishes(apiBase: string, ref: string, scope: WishScope): Promise<any[]> {
  const r = await $fetch<any>(wishesUrl(apiBase, ref, scope), { credentials: 'include', cache: 'no-store', headers: nativeHeaders() })
  return r?.wishes ?? []
}
