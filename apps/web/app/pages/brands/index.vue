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
    <h1>{{ $t('guest.brands.heading') }}</h1>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!brands.length" class="g-center g-mute"><strong class="x-soon">{{ $t('guest.brands.soonTitle') }}</strong><br>{{ $t('guest.brands.soonBody') }}</div>
    <div v-else class="g-grid">
      <article v-for="b in brands" :key="b.slug" class="g-card">
        <img v-if="b.logo_url" class="x-logo" :src="b.logo_url" alt="" loading="lazy">
        <div v-else class="x-logo x-ph" aria-hidden="true">{{ b.name.slice(0, 1) }}</div>
        <NuxtLink :to="`/brands/${b.slug}`" class="x-title">{{ b.name }}</NuxtLink>
        <p v-if="b.description" class="g-mute">{{ b.description }}</p>
      </article>
    </div>
  </main>
</template>

<style scoped>
.x-soon{display:block;font-size:20px;color:var(--fg);margin-bottom:8px}
.x-logo{display:block;width:72px;height:72px;object-fit:contain;margin-bottom:8px}
.x-ph{display:flex;align-items:center;justify-content:center;background:var(--bg-alt);border:1px solid var(--line);color:var(--red-text);font-size:32px;font-weight:700}
.x-title{display:block;font-weight:700;font-size:18px;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:8px 0}
</style>
