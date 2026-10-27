<script setup lang="ts">
// 組織公開頁：已驗證組織的介紹與收件據點（SSR；公開 API 不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
const slug = String(useRoute().params.slug)
const { data: org, error, refresh } = await useAsyncData(`org-${slug}`, () => $fetch<any>(`${apiBase}/api/v1/public/orgs/${encodeURIComponent(slug)}`))
const notFound = computed(() => (error.value as any)?.statusCode === 404 || (error.value as any)?.status === 404)
if (notFound.value && import.meta.server) setResponseStatus(useRequestEvent()!, 404)
const origin = useRequestURL().origin
useSeoMeta({
  title: () => org.value ? t('guest.relief.orgSeo', { name: org.value.display_name }) : t('guest.relief.heading'),
  description: () => org.value?.intro || t('guest.relief.seoDesc'),
  ogTitle: () => org.value?.display_name ?? t('guest.relief.heading'),
  ogDescription: () => org.value?.intro || t('guest.relief.seoDesc'),
  ogImage: () => org.value?.logo ?? `${origin}/og-default.png`, ogType: 'website', ogUrl: `${origin}/o/${slug}`, twitterCard: 'summary',
})
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div v-if="notFound" class="g-center"><h1>{{ $t('guest.relief.orgNotFound') }}</h1><NuxtLink to="/e">{{ $t('guest.relief.back') }}</NuxtLink></div>
    <div v-else-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <template v-else-if="org">
      <header class="x-head">
        <img v-if="org.logo" class="x-logo" :src="org.logo" alt="" loading="lazy">
        <div>
          <h1>{{ org.display_name }}<span class="g-badge">{{ $t('guest.relief.verified') }}</span></h1>
          <p v-if="org.intro">{{ org.intro }}</p>
          <p class="g-mute">
            <a v-if="org.website" :href="org.website" target="_blank" rel="noopener nofollow">{{ $t('guest.relief.website') }}</a>
            <template v-if="org.public_email"> ・ <a :href="`mailto:${org.public_email}`">{{ org.public_email }}</a></template>
            <template v-if="org.public_phone"> ・ <a :href="`tel:${org.public_phone}`">{{ org.public_phone }}</a></template>
          </p>
        </div>
      </header>
      <h2>{{ $t('guest.relief.orgSites') }}</h2>
      <div v-if="!org.sites.length" class="g-center g-mute">{{ $t('guest.relief.noSites') }}</div>
      <div v-else class="g-grid">
        <article v-for="s in org.sites" :key="s.slug" class="g-card">
          <NuxtLink :to="`/s/${s.slug}`" class="x-title">{{ s.title }}</NuxtLink>
          <p class="g-mute">{{ $t('guest.relief.event') }}：<NuxtLink :to="`/e/${s.event.slug}`">{{ s.event.name }}</NuxtLink></p>
          <p class="g-mute"><a :href="mapsUrl(s.site_address)" target="_blank" rel="noopener">{{ s.site_address }}</a></p>
        </article>
      </div>
    </template>
  </main>
</template>

<style scoped>
.x-head{display:flex;gap:16px;align-items:center;margin:8px 0 16px}
.x-head h1{margin:0 0 4px}
.x-logo{flex:none;width:72px;height:72px;object-fit:contain}
.x-title{display:block;font-weight:700;font-size:18px;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:8px 0}
</style>
