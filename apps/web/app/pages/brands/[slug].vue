<script setup lang="ts">
// 品牌頁：品牌介紹 + 該品牌商品（SSR；公開 API 不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
const slug = String(useRoute().params.slug)
const { data, error, refresh } = await useAsyncData(`brand-${slug}`, () => $fetch<any>(`${apiBase}/api/v1/public/brands/${encodeURIComponent(slug)}`))
const brand = computed(() => data.value?.brand)
const notFound = computed(() => (error.value as any)?.statusCode === 404 || (error.value as any)?.status === 404)
if (notFound.value && import.meta.server) setResponseStatus(useRequestEvent()!, 404)
useSeoMeta({
  title: () => brand.value ? t('guest.brands.brandTitle', { name: brand.value.name }) : t('guest.brands.heading'),
  description: () => brand.value?.description || t('guest.brands.seoDesc'),
})
const { products, next, error: pErr, refresh: pRefresh, more, busy, moreErr } = await useCatalog(`brand-products-${slug}`, () => ({ brand: slug }))
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div v-if="notFound" class="g-center"><h1>{{ $t('guest.brands.notFound') }}</h1><NuxtLink to="/brands">{{ $t('guest.brands.back') }}</NuxtLink></div>
    <div v-else-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <template v-else-if="brand">
      <header class="x-head">
        <img v-if="brand.logo_url" class="x-logo" :src="brand.logo_url" alt="" loading="lazy">
        <div v-else class="x-logo x-ph" aria-hidden="true">{{ brand.name.slice(0, 1) }}</div>
        <div>
          <h1>{{ brand.name }}</h1>
          <p v-if="brand.description" class="g-mute">{{ brand.description }}</p>
          <a v-if="brand.website_url" :href="brand.website_url" target="_blank" rel="noopener nofollow">{{ $t('guest.brands.website') }}</a>
        </div>
      </header>
      <div v-if="pErr" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="pRefresh()">{{ $t('guest.reload') }}</button></div>
      <div v-else-if="!products.length" class="g-center g-mute">{{ $t('guest.brands.noProducts') }}</div>
      <GuestCatalogGrid v-else :products="products" />
      <div v-if="next" class="g-center">
        <p v-if="moreErr" class="g-field-err" role="alert">{{ $t('guest.explore.moreErr') }}</p>
        <button class="g-btn ghost" :disabled="busy" @click="more">{{ busy ? $t('guest.loading') : $t('guest.explore.loadMore') }}</button>
      </div>
    </template>
  </main>
</template>

<style scoped>
.x-head{display:flex;gap:16px;align-items:center;margin:8px 0 16px}
.x-head h1{margin:0 0 4px}
.x-logo{flex:none;width:72px;height:72px;object-fit:contain}
.x-ph{display:flex;align-items:center;justify-content:center;background:var(--bg-alt);border:1px solid var(--line);color:var(--red-text);font-size:32px;font-weight:700}
</style>
