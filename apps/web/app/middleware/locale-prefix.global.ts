// 語系前綴網址（/en|vi|id/...，見 utils/localized.ts）：依 route.meta.lang 切語系，SSR 也生效（會寫 ws_locale cookie，與手動切換相同）
export default defineNuxtRouteMiddleware(async (to) => {
  const lang = to.meta.lang as string | undefined
  const i18n = useNuxtApp().$i18n
  if (lang && i18n.locale.value !== lang) await i18n.setLocale(lang as typeof i18n.locale.value)
})
