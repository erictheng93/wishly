// 語系前綴網址（/en|vi|id/...，見 utils/localized.ts）：依 route.meta.lang 切語系，SSR 也生效（會寫 ws_locale cookie，與手動切換相同）
export default defineNuxtRouteMiddleware(async (to, from) => {
  // 從前綴頁導到無前綴的可收錄頁＝zh-Hant 正本；直接開無前綴網址（分享連結）仍依 cookie
  const lang = (to.meta.lang as string | undefined) ?? (from.meta.lang && isLocalizable(to.path) ? 'zh-Hant' : undefined)
  const i18n = useNuxtApp().$i18n
  if (lang && i18n.locale.value !== lang) await i18n.setLocale(lang as typeof i18n.locale.value)
})
