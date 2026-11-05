<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.unsub.seoTitleSettings'), robots: 'noindex' })
const ok = useRoute().query.ok === '1'
</script>
<template>
  <GuestTop />
  <main class="g-wrap">
    <div class="x-n">
    <div class="x-n-body">
    <UiIcon name="mail" :size="40" class="x-n-ic" />
    <h1>{{ ok ? $t('guest.unsub.okTitle') : $t('guest.unsub.invalidTitle') }}</h1>
    <p class="g-mute">{{ ok ? $t('guest.unsub.doneBody') : $t('guest.unsub.invalidBody') }}</p>
    <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
    </div>
    </div>
  </main>
</template>
<style scoped>
.x-n{max-width:560px;margin:48px auto 80px;padding:0 0 28px;border:1px solid var(--line);border-top:2px solid var(--ink-line);background:var(--card);text-align:left}
.x-n-body{padding:32px 24px 0}
.x-n-ic{display:block;margin-bottom:20px}
.x-n h1{margin:0 0 12px;font-size:clamp(28px,6vw,40px);font-weight:900;letter-spacing:-.03em}
.x-n p{margin:0 0 20px}
.x-n .g-btn{width:100%}
.x-n a{display:flex;align-items:center;justify-content:center;min-height:48px;border:1px solid var(--ink-line);font-weight:700;text-decoration:none;color:var(--fg)}
.x-n a:hover,.x-n a:focus-visible{background:var(--fg);color:var(--bg)}
@media (max-width:520px){.x-n{margin:32px 0 56px}}
</style>
