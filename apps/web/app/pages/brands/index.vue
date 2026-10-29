<script setup lang="ts">
// 品牌館：品牌目錄（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.brands.seoTitle'), description: () => t('guest.brands.seoDesc') })
const { data, error, refresh } = await useAsyncData('brands', () => $fetch<any>(`${apiBase}/api/v1/public/brands`))
const brands = computed<any[]>(() => data.value?.brands ?? [])
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <p class="x-idx" aria-hidden="true"><span>INDEX / 030</span><span>BRAND DIRECTORY</span></p>
    <h1>{{ $t('guest.brands.heading') }}</h1>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!brands.length" class="g-center g-mute"><strong class="x-soon">{{ $t('guest.brands.soonTitle') }}</strong><br>{{ $t('guest.brands.soonBody') }}</div>
    <div v-else class="g-grid">
      <article v-for="(b, i) in brands" :key="b.slug" class="g-card x-card">
        <span class="x-no" aria-hidden="true">{{ String(i + 1).padStart(2, '0') }}</span>
        <img v-if="b.logo_url" class="x-logo" :src="b.logo_url" alt="" loading="lazy">
        <div v-else class="x-logo x-ph" aria-hidden="true">{{ b.name.slice(0, 1) }}</div>
        <NuxtLink :to="`/brands/${b.slug}`" class="x-title">{{ b.name }}</NuxtLink>
        <p v-if="b.description" class="g-mute">{{ b.description }}</p>
      </article>
    </div>
  </main>
</template>

<style scoped>
.x-idx{display:flex;justify-content:space-between;gap:16px;margin:28px 0 0;padding-bottom:12px;border-bottom:1px solid var(--ink-line);font:600 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-idx span:first-child{color:var(--red-text)}
.x-idx+h1{margin-top:20px;margin-bottom:28px;font-size:clamp(40px,8vw,84px);letter-spacing:-.04em;font-weight:900}
.x-soon{display:block;font-size:20px;color:var(--fg);margin-bottom:8px}
.x-card{position:relative;padding:20px 16px 16px}
.x-no{position:absolute;right:12px;top:12px;font:700 11px var(--mono);letter-spacing:.14em;color:var(--mute)}
.x-logo{display:block;width:72px;height:72px;object-fit:contain;margin-bottom:12px}
.x-ph{display:flex;align-items:center;justify-content:center;background:var(--hatch),var(--bg-alt);border:1px solid var(--line);color:var(--red-text);font-size:34px;font-weight:900}
.x-title{display:block;font-weight:800;font-size:20px;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:8px 0 4px;letter-spacing:-.01em}
.x-title::after{content:"";position:absolute;inset:0}
.x-title:hover{color:var(--red-text)}
</style>
