<script setup lang="ts">
// 救援事件頁：事件資訊 + 各據點與缺口數字（SSR；公開 API 不帶 cookie）
const { public: { apiBase, siteUrl } } = useRuntimeConfig()
const { t, locale } = useI18n()
const slug = String(useRoute().params.slug)
const { data, error, refresh } = await useAsyncData(`relief-event-${slug}`, () => $fetch<any>(`${apiBase}/api/v1/public/relief-events/${encodeURIComponent(slug)}`))
// 事件已合併：永久轉址到最終事件
if (data.value?.merged_into) await navigateTo(localizedPath(`/e/${data.value.merged_into}`, locale.value), { redirectCode: 301 })
const ev = computed(() => data.value?.event)
const sites = computed<any[]>(() => data.value?.sites ?? [])
const notFound = computed(() => (error.value as any)?.statusCode === 404 || (error.value as any)?.status === 404)
if (notFound.value && import.meta.server) setResponseStatus(useRequestEvent()!, 404)
const origin = useRequestURL().origin
useHead({ script: () => ev.value ? [ldScript(eventLd(siteUrl, slug, ev.value))] : [] })
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
    <div v-if="notFound" class="c-nf"><h1>{{ $t('guest.relief.eventNotFound') }}</h1><NuxtLink :to="localizedPath('/e', $i18n.locale)">{{ $t('guest.relief.back') }}</NuxtLink></div>
    <div v-else-if="error" class="c-nf err"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <template v-else-if="ev">
      <h1>{{ ev.name }}</h1>
      <p class="g-mute x-meta">
        {{ $t('org.relief.hazard.' + ev.hazard_type) }}・{{ $t('guest.relief.counties', { list: ev.counties.join('、') }) }}<template v-if="ev.area_note">（{{ ev.area_note }}）</template><template v-if="ev.occurred_on">・{{ $t('guest.relief.date', { date: ev.occurred_on }) }}</template>
      </p>
      <p v-if="ev.description" class="x-desc">{{ ev.description }}</p>
      <div v-if="!sites.length" class="g-center g-mute">{{ $t('guest.relief.noSites') }}</div>
      <div v-else class="g-grid">
        <article v-for="s in sites" :key="s.slug" class="g-card">
          <NuxtLink :to="localizedPath(`/s/${s.slug}`, $i18n.locale)" class="x-title">{{ s.title }}</NuxtLink>
          <p class="g-mute"><NuxtLink :to="localizedPath(`/o/${s.org.slug}`, $i18n.locale)">{{ s.org.display_name }}</NuxtLink><span class="g-badge">{{ $t('guest.relief.verified') }}</span></p>
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
.g-wrap h1{position:relative;margin:44px 0 16px;padding-bottom:18px;border-bottom:1px solid var(--line);font-weight:900;letter-spacing:-.03em}
.g-wrap h1::after{content:"";position:absolute;left:0;bottom:-1px;width:48px;height:2px;background:var(--red)}
html:lang(vi) .g-wrap h1{line-height:1.22}
.g-grid .g-card{padding:14px 16px 16px;display:flex;flex-direction:column;animation:rise .5s var(--ease) both}
.x-title{display:block;font-weight:800;font-size:19px;line-height:1.35;letter-spacing:-.01em;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:6px 0 4px;background:linear-gradient(var(--red),var(--red)) 0 100%/0 2px no-repeat;transition:background-size var(--t) var(--ease)}
.x-title:hover{background-size:100% 2px}
/* 空狀態：虛線斜線底的標本盒，頂端一枚線條圖釘 */
.g-center.g-mute{display:grid;justify-items:center;gap:14px;margin:28px 0 0;padding:52px 20px;border:1px dashed var(--ink-line);background:var(--hatch),var(--card);font-size:14px;color:var(--mute);text-align:center}
.g-center.g-mute::before{content:"";width:34px;height:34px;background:var(--fg);-webkit-mask:var(--pin) center/contain no-repeat;mask:var(--pin) center/contain no-repeat;--pin:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23000' stroke-width='1.2' stroke-linejoin='round'%3E%3Cpath d='M12 21s-7-6.1-7-11.5a7 7 0 0 1 14 0C19 14.900 12 21 12 21z'/%3E%3Ccircle cx='12' cy='9.500' r='2.500'/%3E%3C/svg%3E")}
.x-meta{margin:-4px 0 18px;padding:8px 12px;max-width:44em;background:var(--bg-alt);border:1px solid var(--line);font:500 13px/1.7 var(--mono);letter-spacing:.02em}
.x-desc{max-width:34em;margin:0 0 12px;font-size:18px;line-height:1.75}
.x-crit{margin-left:0}
</style>
