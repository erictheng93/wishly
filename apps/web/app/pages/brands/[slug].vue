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
      <p class="x-idx" aria-hidden="true"><span>INDEX / 031</span><span>BRAND FILE</span></p>
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
.x-idx{display:flex;justify-content:space-between;gap:16px;margin:28px 0 0;padding-bottom:12px;border-bottom:1px solid var(--ink-line);font:600 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-idx span:first-child{color:var(--red-text)}
.x-head{display:flex;gap:20px;align-items:center;margin:20px 0 32px;padding-bottom:24px;border-bottom:1px solid var(--line)}
.x-head h1{margin:0 0 6px;font-size:clamp(34px,7vw,64px);font-weight:900;letter-spacing:-.04em}
.x-head a{display:inline-flex;align-items:center;min-height:44px;font-weight:700;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:5px}
.x-logo{flex:none;width:96px;height:96px;object-fit:contain;border:1px solid var(--ink-line);background:#fff;padding:8px;box-sizing:border-box}
.x-ph{display:flex;align-items:center;justify-content:center;background:var(--hatch),var(--bg-alt);color:var(--red-text);font-size:44px;font-weight:900}
@media (max-width:520px){.x-head{flex-direction:column;align-items:flex-start}}
</style>
