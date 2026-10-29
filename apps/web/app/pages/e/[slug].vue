<script setup lang="ts">
// 救援事件頁：事件資訊 + 各據點與缺口數字（SSR；公開 API 不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
const slug = String(useRoute().params.slug)
const { data, error, refresh } = await useAsyncData(`relief-event-${slug}`, () => $fetch<any>(`${apiBase}/api/v1/public/relief-events/${encodeURIComponent(slug)}`))
// 事件已合併：永久轉址到最終事件
if (data.value?.merged_into) await navigateTo(`/e/${data.value.merged_into}`, { redirectCode: 301 })
const ev = computed(() => data.value?.event)
const sites = computed<any[]>(() => data.value?.sites ?? [])
const notFound = computed(() => (error.value as any)?.statusCode === 404 || (error.value as any)?.status === 404)
if (notFound.value && import.meta.server) setResponseStatus(useRequestEvent()!, 404)
const origin = useRequestURL().origin
useSeoMeta({
  title: () => ev.value?.name ?? t('guest.relief.heading'),
  description: () => ev.value?.description || t('guest.relief.seoDesc'),
  ogTitle: () => ev.value?.name ?? t('guest.relief.heading'),
  ogDescription: () => ev.value?.description || t('guest.relief.seoDesc'),
  ogImage: `${origin}/og-default.png`, ogType: 'website', ogUrl: `${origin}/e/${slug}`, twitterCard: 'summary_large_image',
})
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <div v-if="notFound" class="g-center"><h1>{{ $t('guest.relief.eventNotFound') }}</h1><NuxtLink to="/e">{{ $t('guest.relief.back') }}</NuxtLink></div>
    <div v-else-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <template v-else-if="ev">
      <span class="x-kicker" aria-hidden="true">RELIEF / EVENT</span>
      <h1>{{ ev.name }}</h1>
      <p class="g-mute">
        {{ $t('org.relief.hazard.' + ev.hazard_type) }}・{{ $t('guest.relief.counties', { list: ev.counties.join('、') }) }}<template v-if="ev.area_note">（{{ ev.area_note }}）</template><template v-if="ev.occurred_on">・{{ $t('guest.relief.date', { date: ev.occurred_on }) }}</template>
      </p>
      <p v-if="ev.description">{{ ev.description }}</p>
      <div v-if="!sites.length" class="g-center g-mute">{{ $t('guest.relief.noSites') }}</div>
      <div v-else class="g-grid">
        <article v-for="(s, n) in sites" :key="s.slug" class="g-card">
          <span class="x-no" aria-hidden="true">SITE {{ String(n + 1).padStart(2, '0') }}</span>
          <NuxtLink :to="`/s/${s.slug}`" class="x-title">{{ s.title }}</NuxtLink>
          <p class="g-mute"><NuxtLink :to="`/o/${s.org.slug}`">{{ s.org.display_name }}</NuxtLink><span class="g-badge">{{ $t('guest.relief.verified') }}</span></p>
          <p class="g-mute"><a :href="mapsUrl(s.site_address)" target="_blank" rel="noopener">{{ s.site_address }}</a></p>
          <p>
            <span class="g-badge critical x-crit">{{ $t('guest.relief.critical', { n: s.needs.critical }) }}</span>
            <span class="g-badge">{{ $t('guest.relief.high', { n: s.needs.high }) }}</span>
          </p>
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
h2.x-h2{font:700 12px var(--mono);letter-spacing:.14em;text-transform:uppercase;margin:28px 0 14px;padding-top:12px;border-top:1px solid var(--ink-line)}
</style>
