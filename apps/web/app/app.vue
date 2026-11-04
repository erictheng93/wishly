<script setup lang="ts">
import '~/assets/tokens.css'
import '~/assets/creator.css'

// <html lang> 跟著目前語系
const { locale, locales } = useI18n()
const { public: { siteUrl } } = useRuntimeConfig()
const route = useRoute()
useHead({
  // canonical 一律指向正式網域、去掉查詢字串（?via= 等），避免預覽網域與帶參數網址被當重複頁
  // 頁面沒指定時的預設分享圖（頁面自己的 og:image 會覆蓋）
  meta: [{ key: 'og:image', property: 'og:image', content: `${siteUrl}/og-default.png` }],
  link: [{ key: 'canonical', rel: 'canonical', href: computed(() => siteUrl + route.path) }],
})
useHead({
  htmlAttrs: { lang: computed(() => locales.value.find(l => typeof l !== 'string' && l.code === locale.value)?.language ?? locale.value) },
  // manifest 依語系：zh-Hant 為預設檔，其餘 manifest.<code>.webmanifest（id 皆為 "/"，不會被當成不同 App）；key 讓切語系時取代而非重複
  link: [{ key: 'manifest', rel: 'manifest', href: computed(() => locale.value === 'zh-Hant' ? '/manifest.webmanifest' : `/manifest.${locale.value}.webmanifest`) }],
})
// 可收錄公開頁（utils/localized.ts）：輸出各語系 hreflang；切換語系時換到該語系網址，讓網址與內容一致
const base = computed(() => stripLocale(route.path))
useHead({
  link: computed(() => isLocalizable(base.value)
    ? [...SEO_LOCALES, 'x-default'].map(h => ({ key: `alt-${h}`, rel: 'alternate' as const, hreflang: h, href: siteUrl + localizedPath(base.value, h === 'x-default' ? 'zh-Hant' : h) }))
    : []),
})
watch(locale, (l) => {
  const to = localizedPath(route.path, l)
  if (isLocalizable(base.value) && to !== route.path) navigateTo({ path: to, query: route.query, hash: route.hash }, { replace: true })
})
</script>
<template>
  <SystemBanner />
  <NuxtPage />
  <ClientOnly><CreatorInboxToast /></ClientOnly>
</template>
