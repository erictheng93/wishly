<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.unsub.seoTitleSettings'), robots: 'noindex' })
const ok = useRoute().query.ok === '1'
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-center" style="padding-inline:16px">
    <h1>{{ ok ? $t('guest.unsub.okTitle') : $t('guest.unsub.invalidTitle') }}</h1>
    <p class="g-mute">{{ ok ? $t('guest.unsub.doneBody') : $t('guest.unsub.invalidBody') }}</p>
    <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
  </main>
</template>
