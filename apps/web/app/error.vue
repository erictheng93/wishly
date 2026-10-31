<script setup lang="ts">
import '~/assets/tokens.css'
import '~/assets/guest.css'
// 全站錯誤頁（取代 Nuxt 預設英文頁）。SSR 狀態碼由 Nuxt 依 error.statusCode 回傳，這裡只管畫面。
// 403 刻意與 404 同樣呈現：/admin 對非營運人員要「看起來像不存在」。5xx 不顯示內部 message。
const { t } = useI18n()
const props = defineProps<{ error: { statusCode?: number, status?: number } }>()
const code = computed(() => props.error?.statusCode ?? props.error?.status ?? 500)
const missing = computed(() => code.value === 404 || code.value === 403)
const busy = computed(() => code.value === 503 || code.value === 429) // 系統繁忙頁：暫時性，給重新整理
useSeoMeta({ title: () => missing.value ? t('auth.error.notFoundTitle') : busy.value ? t('common.sys.busyTitle') : t('auth.error.errorTitle'), robots: 'noindex' })
</script>

<template>
  <GuestTop />
  <main class="g-wrap er-page">
    <p class="er-code" aria-hidden="true">{{ code === 403 ? 404 : code }}</p>
    <div class="er-body">
    <template v-if="missing">
      <h1>{{ $t('auth.error.notFoundTitle') }}</h1>
      <p class="g-mute">{{ $t('auth.error.notFoundDesc') }}</p>
    </template>
    <template v-else-if="busy">
      <h1>{{ $t('common.sys.busyTitle') }}</h1>
      <p class="g-mute">{{ $t('common.sys.busyDesc') }}</p>
      <button class="g-btn ghost" @click="reloadNuxtApp()">{{ $t('common.sys.reload') }}</button>
    </template>
    <template v-else>
      <h1>{{ $t('auth.error.serverTitle') }}</h1>
      <p class="g-mute">{{ $t('auth.error.serverDesc') }}</p>
    </template>
    <button class="g-btn" @click="clearError({ redirect: '/' })">{{ $t('auth.error.home') }}</button>
    </div>
  </main>
</template>

<style>
/* 不用 scoped：SSR 的 <h1> 不能帶 data-v 屬性（e2e 以原始 HTML 驗證繁中錯誤頁） */
.er-page{padding-top:32px;padding-bottom:96px}
.er-code{font:900 clamp(120px,34vw,320px)/.85 var(--font);letter-spacing:-.06em;color:var(--fg);margin:16px 0 0;font-variant-numeric:tabular-nums}
.er-code::after{content:"";display:block;width:min(240px,50%);height:18px;margin-top:20px;background:var(--stripe);background-size:17px 17px}
.er-body{margin-top:32px;padding-top:20px;border-top:2px solid var(--ink-line);max-width:34em}
.er-body h1{margin:0 0 12px;font-size:clamp(26px,5vw,40px);font-weight:900;letter-spacing:-.03em}
.er-body p{margin:0 0 24px}
.er-body .g-btn{margin-right:8px}
</style>
