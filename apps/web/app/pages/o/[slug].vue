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
    <div v-if="notFound" class="c-nf"><h1>{{ $t('guest.relief.orgNotFound') }}</h1><NuxtLink to="/e">{{ $t('guest.relief.back') }}</NuxtLink></div>
    <div v-else-if="error" class="c-nf err"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <template v-else-if="org">
      <header class="x-head">
        <img v-if="org.logo" class="x-logo" :src="org.logo" alt="" loading="lazy">
        <div>
          <span class="x-kicker" aria-hidden="true">ORG / VERIFIED</span>
          <h1>{{ org.display_name }}<span class="g-badge">{{ $t('guest.relief.verified') }}</span></h1>
          <p v-if="org.intro">{{ org.intro }}</p>
          <p class="g-mute">
            <a v-if="org.website" class="x-nw" :href="org.website" target="_blank" rel="noopener nofollow">{{ $t('guest.relief.website') }}</a>
            <template v-if="org.public_email"> ・ <a class="x-nw" :href="`mailto:${org.public_email}`">{{ org.public_email }}</a></template>
            <template v-if="org.public_phone"> ・ <a class="x-nw" :href="`tel:${org.public_phone}`">{{ org.public_phone }}</a></template>
          </p>
        </div>
      </header>
      <h2 class="x-h2">{{ $t('guest.relief.orgSites') }}</h2>
      <div v-if="!org.sites.length" class="g-center g-mute">{{ $t('guest.relief.noSites') }}</div>
      <div v-else class="g-grid">
        <article v-for="(s, n) in org.sites" :key="s.slug" class="g-card">
          <span class="x-no" aria-hidden="true">SITE {{ String(n + 1).padStart(2, '0') }}</span>
          <NuxtLink :to="`/s/${s.slug}`" class="x-title">{{ s.title }}</NuxtLink>
          <p class="g-mute">{{ $t('guest.relief.event') }}：<NuxtLink :to="`/e/${s.event.slug}`">{{ s.event.name }}</NuxtLink></p>
          <p class="g-mute"><a :href="mapsUrl(s.site_address)" target="_blank" rel="noopener">{{ s.site_address }}</a></p>
        </article>
      </div>
    </template>
  </main>
</template>

<style scoped>
.x-kicker{display:block;margin:28px 0 0;font:700 11px var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-kicker::before{content:"";display:inline-block;width:8px;height:8px;background:var(--red);margin-right:8px}
.x-kicker+h1{margin-top:8px}
.g-grid .g-card{border-top:2px solid var(--ink-line);padding:14px 16px 16px;display:flex;flex-direction:column;animation:rise .5s var(--ease) both}
.x-no{font:700 11px var(--mono);letter-spacing:.14em;color:var(--mute)}
.x-title{display:block;font-weight:800;font-size:19px;line-height:1.35;letter-spacing:-.01em;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:6px 0 4px;background:linear-gradient(var(--red),var(--red)) 0 100%/0 2px no-repeat;transition:background-size var(--t) var(--ease)}
.x-title:hover{background-size:100% 2px}
.x-crit{margin-left:0}
.x-head{display:flex;gap:16px;align-items:center;margin:8px 0 20px;padding-bottom:20px;border-bottom:1px solid var(--ink-line)}
.x-head h1{margin:0 0 6px}
.x-logo{flex:none;width:80px;height:80px;object-fit:contain;border:1px solid var(--line);padding:6px;background:var(--card)}
.x-nw{white-space:nowrap}
.x-head+h2.x-h2{border-top:0;padding-top:0;margin-top:24px}
h2.x-h2{font:700 12px var(--mono);letter-spacing:.14em;text-transform:uppercase;margin:28px 0 14px;padding-top:12px;border-top:1px solid var(--ink-line)}
</style>
