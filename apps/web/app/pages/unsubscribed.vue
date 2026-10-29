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
    <div class="x-n-head" aria-hidden="true"><span>NOTICE / UNSUBSCRIBE</span><span>No.0091</span></div>
    <div class="x-n-body">
    <h1>{{ ok ? $t('guest.unsub.okTitle') : $t('guest.unsub.invalidTitle') }}</h1>
    <p class="g-mute">{{ ok ? $t('guest.unsub.doneBody') : $t('guest.unsub.invalidBody') }}</p>
    <NuxtLink to="/settings">{{ $t('guest.unsub.toSettings') }}</NuxtLink>
    </div>
    </div>
  </main>
</template>
<style scoped>
.x-n{max-width:560px;margin:56px auto 80px;padding:0 0 28px;border:1px solid var(--ink-line);background:var(--card);box-shadow:10px 10px 0 var(--bg-alt),10px 10px 0 1px var(--line);text-align:left}
.x-n-head{display:flex;justify-content:space-between;padding:12px 20px;background:var(--hatch);border-bottom:1px solid var(--ink-line);font:700 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-n-head span:last-child{color:var(--red-text)}
.x-n-body{padding:28px 24px 0}
.x-n h1{margin:0 0 12px;font-size:clamp(28px,6vw,40px);font-weight:900;letter-spacing:-.03em}
.x-n p{margin:0 0 20px}
.x-n .g-btn{width:100%}
.x-n a{display:inline-flex;align-items:center;min-height:44px;font-weight:700;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:5px}
@media (max-width:520px){.x-n{margin:32px 0 56px;box-shadow:none}}
</style>
