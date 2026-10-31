<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('legal.privacy.title') })
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-doc">
    <p class="x-idx" aria-hidden="true"><span>INDEX / 090</span><span>PRIVACY NOTICE</span></p>
    <h1>{{ $t('legal.privacy.title') }}</h1>
    <p class="g-mute x-draft">{{ $t('legal.privacy.draft') }}</p>
    <h2 id="collect">{{ $t('legal.privacy.collectH') }}</h2>
    <ul>
      <li>{{ $t('legal.privacy.c1') }}</li>
      <li>{{ $t('legal.privacy.c2') }}</li>
      <li>{{ $t('legal.privacy.c3') }}</li>
      <li>{{ $t('legal.privacy.c4') }}</li>
    </ul>
    <h2>{{ $t('legal.privacy.deviceH') }}</h2>
    <p>{{ $t('legal.privacy.deviceP') }}</p>
    <h2>{{ $t('legal.privacy.viewH') }}</h2>
    <p>{{ $t('legal.privacy.viewP') }}</p>
    <h2>{{ $t('legal.privacy.rightsH') }}</h2>
    <p>{{ $t('legal.privacy.rightsP') }}</p>
    <h2>{{ $t('legal.privacy.reportH') }}</h2>
    <p>{{ $t('legal.privacy.reportP') }}</p>
    <h2>{{ $t('legal.privacy.loginH') }}</h2>
    <p>{{ $t('legal.privacy.loginP') }}</p>
    <h2 id="contact">{{ $t('legal.privacy.contactH') }}</h2>
    <p>{{ $t('legal.privacy.contactP') }} <SupportLink /></p>
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
