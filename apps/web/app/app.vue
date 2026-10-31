<script setup lang="ts">
import '~/assets/tokens.css'
import '~/assets/creator.css'

// <html lang> 跟著目前語系
const { locale, locales } = useI18n()
useHead({
  htmlAttrs: { lang: computed(() => locales.value.find(l => typeof l !== 'string' && l.code === locale.value)?.language ?? locale.value) },
  // manifest 依語系：zh-Hant 為預設檔，其餘 manifest.<code>.webmanifest（id 皆為 "/"，不會被當成不同 App）；key 讓切語系時取代而非重複
  link: [{ key: 'manifest', rel: 'manifest', href: computed(() => locale.value === 'zh-Hant' ? '/manifest.webmanifest' : `/manifest.${locale.value}.webmanifest`) }],
})
</script>
<template>
  <SystemBanner />
  <NuxtPage />
  <ClientOnly><CreatorInboxToast /></ClientOnly>
</template>
