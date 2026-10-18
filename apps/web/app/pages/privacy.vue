<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('legal.privacy.title') })
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-doc">
    <h1>{{ $t('legal.privacy.title') }}</h1>
    <p class="g-mute">{{ $t('legal.privacy.draft') }}</p>
    <h2 id="collect">{{ $t('legal.privacy.collectH') }}</h2>
    <ul>
      <li>{{ $t('legal.privacy.c1') }}</li>
      <li>{{ $t('legal.privacy.c2') }}</li>
      <li>{{ $t('legal.privacy.c3') }}</li>
      <li>{{ $t('legal.privacy.c4') }}</li>
    </ul>
    <h2>{{ $t('legal.privacy.deviceH') }}</h2>
    <p>{{ $t('legal.privacy.deviceP') }}</p>
    <h2>{{ $t('legal.privacy.rightsH') }}</h2>
    <p>{{ $t('legal.privacy.rightsP') }}</p>
    <h2>{{ $t('legal.privacy.reportH') }}</h2>
    <p>{{ $t('legal.privacy.reportP') }}</p>
  </main>
</template>
