import { nativeHeaders } from './native'
// 清單可見性：選項文案（同時作為分享時「誰能打開」的提示）
const t = (k: string) => useNuxtApp().$i18n.t(k)
// label / desc 用 getter：語系是每個請求（SSR）或使用者切換後才決定，不能在模組載入時固定
const opt = <V extends string>(v: V) => ({ v, get label() { return t(`creator.vis.${v}.label`) }, get desc() { return t(`creator.vis.${v}.desc`) } })
export const VIS_OPTIONS = [opt('public'), opt('friends'), opt('selected'), opt('private')] as const
export type Visibility = typeof VIS_OPTIONS[number]['v']
export const visDesc = (v: string) => VIS_OPTIONS.find(o => o.v === v)?.desc ?? ''

// 受限清單（SSR 沒有 cookie 會拿到 403）：要等 client 帶憑證重抓
export const isRestricted = (v?: string) => !!v && v !== 'public'

/** 帶憑證讀受限清單：先打 /restricted（不走 CDN 快取），404 時退回舊的 /public（帶憑證）。
 *  退回只為了「前端先上線、後端還是舊版」時好友仍看得到；新後端的 /public 永遠不回受限內容，退回不會外洩。
 *  ponytail: 前後端都上線後拿掉 404 退回（多一次請求只發生在真的 404 時）。 */
export async function fetchListWithCreds<T = any>(apiBase: string, slug: string): Promise<T> {
  const opt = { credentials: 'include' as const, cache: 'no-cache' as const, headers: nativeHeaders() }
  try { return await $fetch<any>(`${apiBase}/api/v1/restricted/wishlists/${slug}`, opt) as T }
  catch (e: any) {
    if ((e?.statusCode ?? e?.response?.status) !== 404) throw e
    return await $fetch<any>(`${apiBase}/api/v1/public/wishlists/${slug}`, opt) as T
  }
}
