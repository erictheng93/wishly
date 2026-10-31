<script setup lang="ts">
import '~/assets/tokens.css'
import '~/assets/creator.css'

// <html lang> 跟著目前語系
const { locale, locales } = useI18n()
useHead({ htmlAttrs: { lang: computed(() => locales.value.find(l => typeof l !== 'string' && l.code === locale.value)?.language ?? locale.value) } })
</script>
<template>
  <SystemBanner />
  <NuxtPage />
  <ClientOnly><CreatorInboxToast /></ClientOnly>
</template>
