<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('legal.terms.title') })
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-doc">
    <p class="x-idx" aria-hidden="true"><span>INDEX / 090</span><span>TERMS OF SERVICE</span></p>
    <h1>{{ $t('legal.terms.title') }}</h1>
    <p class="g-mute x-draft">{{ $t('legal.terms.draft') }}</p>
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
<style scoped>
.x-idx{display:flex;justify-content:space-between;gap:16px;margin:28px 0 0;padding-bottom:12px;border-bottom:1px solid var(--ink-line);font:600 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-idx span:first-child{color:var(--red-text)}
.g-doc{counter-reset:sec;max-width:760px}
.x-idx+h1{margin-top:20px;font-size:clamp(36px,7vw,64px);font-weight:900;letter-spacing:-.04em}
.x-draft{display:inline-block;margin:8px 0 0;padding:8px 14px 8px 18px;border:1px solid var(--line);background:var(--hatch),var(--bg-alt);box-shadow:inset 4px 0 var(--red);font:600 12px/1.6 var(--mono);letter-spacing:.04em}
.g-doc h2{margin-top:48px;padding-top:18px;border-top:2px solid var(--ink-line);font-size:clamp(20px,3vw,26px);letter-spacing:-.01em}
.g-doc ul{padding-left:0;list-style:none}
.g-doc li{position:relative;padding-left:22px}
.g-doc li::before{content:"";position:absolute;left:0;top:.8em;width:8px;height:8px;background:var(--red)}
</style>
