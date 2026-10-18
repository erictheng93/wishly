<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('legal.terms.title') })
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-doc">
    <h1>{{ $t('legal.terms.title') }}</h1>
    <p class="g-mute">{{ $t('legal.terms.draft') }}</p>
    <h2>{{ $t('legal.terms.s1h') }}</h2>
    <p>{{ $t('legal.terms.s1p') }}</p>
    <h2>{{ $t('legal.terms.s2h') }}</h2>
    <ul><li>{{ $t('legal.terms.s2l1') }}</li><li>{{ $t('legal.terms.s2l2') }}</li><li>{{ $t('legal.terms.s2l3') }}</li></ul>
    <h2>{{ $t('legal.terms.s3h') }}</h2>
    <p>{{ $t('legal.terms.s3p') }}</p>
    <h2>{{ $t('legal.terms.s4h') }}</h2>
    <p>{{ $t('legal.terms.s4p') }}</p>
    <h2>{{ $t('legal.terms.s5h') }}</h2>
    <p>{{ $t('legal.terms.s5p') }}</p>
  </main>
</template>
