// 多語系收錄：全站是 no_prefix（語系存 cookie ws_locale），Googlebot 不帶 cookie 只看得到 zh-Hant。
// 會被收錄的公開頁另有語系前綴網址 /en|zh-hans|vi|id/...（nuxt.config 的 pages:extend 產生、middleware/locale-prefix.global 切語系），
// 無前綴網址是 zh-Hant 正本；創建者頁面不收錄，不加前綴。
export const PREFIX_LOCALES = ['zh-Hans', 'en', 'vi', 'id'] as const
export const SEO_LOCALES = ['zh-Hant', ...PREFIX_LOCALES] as const

// 可收錄的公開頁（不含語系前綴）；新增公開頁要加在這裡，sitemap 也要列
const LOCALIZABLE = /^\/(?:explore|inspire(?:\/[^/]+)?|brands(?:\/[^/]+)?|s\/[^/]+|e(?:\/[^/]+)?|org|o\/[^/]+|terms|privacy)?$/
/** 網址前綴一律小寫（/zh-hans）；vue-router 比對不分大小寫，所以 stripLocale 也不分 */
export const localePrefix = (locale: string) => locale.toLowerCase()
const PREFIX = new RegExp(`^/(${PREFIX_LOCALES.map(localePrefix).join('|')})(?=/|$)`, 'i')

export const isLocalizable = (path: string) => LOCALIZABLE.test(path)
/** 去掉語系前綴：/vi/s/abc → /s/abc、/vi → / */
export const stripLocale = (path: string) => path.replace(PREFIX, '') || '/'
/** 該語系的網址；不可收錄的頁面或 zh-Hant 維持原路徑 */
export const localizedPath = (path: string, locale: string) => {
  const base = stripLocale(path)
  if (!isLocalizable(base) || !(PREFIX_LOCALES as readonly string[]).includes(locale)) return base
  return base === '/' ? `/${localePrefix(locale)}` : `/${localePrefix(locale)}${base}`
}
/** 送給後端的語系碼（Accept-Language、users.locale）：中文用地區碼，其餘同前端 */
export const apiLocale = (locale: string) => ({ 'zh-Hant': 'zh-TW', 'zh-Hans': 'zh-CN' } as Record<string, string>)[locale] ?? locale
