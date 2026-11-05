<script setup lang="ts">
// 組織公開頁：已驗證組織的介紹與收件據點（SSR；公開 API 不帶 cookie）
const { public: { apiBase, siteUrl } } = useRuntimeConfig()
const { t } = useI18n()
const slug = String(useRoute().params.slug)
const { data: org, error, refresh } = await useAsyncData(`org-${slug}`, () => $fetch<any>(`${apiBase}/api/v1/public/orgs/${encodeURIComponent(slug)}`))
const notFound = computed(() => (error.value as any)?.statusCode === 404 || (error.value as any)?.status === 404)
if (notFound.value && import.meta.server) setResponseStatus(useRequestEvent()!, 404)
const reporting = ref(false), toast = ref('') // 只在使用者點擊後才掛 sheet，SSR 不讀 cookie
const origin = useRequestURL().origin
useHead({ script: () => org.value ? [ldScript(orgLd(siteUrl, slug, org.value))] : [] })
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
    <div v-if="notFound" class="c-nf"><h1>{{ $t('guest.relief.orgNotFound') }}</h1><NuxtLink :to="localizedPath('/e', $i18n.locale)">{{ $t('guest.relief.back') }}</NuxtLink></div>
    <div v-else-if="error" class="c-nf err"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <template v-else-if="org">
      <header class="x-head">
        <img v-if="org.logo" class="x-logo" :src="org.logo" alt="" loading="lazy">
        <div>
          <h1>{{ org.display_name }}<span class="g-badge">{{ $t('guest.relief.verified') }}</span></h1>
          <p v-if="org.intro" class="x-intro">{{ org.intro }}</p>
          <p class="g-mute x-contact">
            <a v-if="org.website" class="x-nw" :href="org.website" target="_blank" rel="noopener nofollow">{{ $t('guest.relief.website') }}</a>
            <a v-if="org.public_email" class="x-nw" :href="`mailto:${org.public_email}`"><UiIcon name="mail" :size="16" />{{ org.public_email }}</a>
            <a v-if="org.public_phone" class="x-nw" :href="`tel:${org.public_phone}`">{{ org.public_phone }}</a>
          </p>
        </div>
      </header>
      <h2 class="x-h2">{{ $t('guest.relief.orgSites') }}</h2>
      <div v-if="!org.sites.length" class="g-center g-mute">{{ $t('guest.relief.noSites') }}</div>
      <div v-else class="g-grid">
        <article v-for="s in org.sites" :key="s.slug" class="g-card">
          <NuxtLink :to="localizedPath(`/s/${s.slug}`, $i18n.locale)" class="x-title">{{ s.title }}</NuxtLink>
          <p class="g-mute">{{ $t('guest.relief.event') }}：<NuxtLink :to="localizedPath(`/e/${s.event.slug}`, $i18n.locale)">{{ s.event.name }}</NuxtLink></p>
          <p class="g-mute"><a :href="mapsUrl(s.site_address)" target="_blank" rel="noopener">{{ s.site_address }}</a></p>
        </article>
      </div>
      <div class="g-foot"><button class="g-link" @click="reporting = true">{{ $t('guest.report.orgTitle') }}</button></div>
      <GuestReportSheet v-if="reporting" :slug="slug" kind="org" @close="reporting = false" @sent="reporting = false; toast = $t('guest.report.sent')" />
    </template>
  </main>
  <div v-if="toast" class="g-banner g-toast" role="status" @click="toast = ''">{{ toast }}</div>
</template>

<style scoped>
.x-head h1{position:relative;margin:44px 0 16px;padding-bottom:18px;border-bottom:1px solid var(--line);font-weight:900;letter-spacing:-.03em}
.x-head h1::after{content:"";position:absolute;left:0;bottom:-1px;width:48px;height:2px;background:var(--red)}
html:lang(vi) .x-head h1{line-height:1.22}
.g-grid .g-card{padding:14px 16px 16px;display:flex;flex-direction:column;animation:rise .5s var(--ease) both}
.x-title{display:block;font-weight:800;font-size:19px;line-height:1.35;letter-spacing:-.01em;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:6px 0 4px;background:linear-gradient(var(--red),var(--red)) 0 100%/0 2px no-repeat;transition:background-size var(--t) var(--ease)}
.x-title:hover{background-size:100% 2px}
/* 空狀態：虛線斜線底的標本盒，頂端一枚線條圖釘 */
.g-center.g-mute{display:grid;justify-items:center;gap:14px;margin:28px 0 0;padding:52px 20px;border:1px dashed var(--ink-line);background:var(--hatch),var(--card);font-size:14px;color:var(--mute);text-align:center}
.g-center.g-mute::before{content:"";width:34px;height:34px;background:var(--fg);-webkit-mask:var(--pin) center/contain no-repeat;mask:var(--pin) center/contain no-repeat;--pin:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23000' stroke-width='1.2' stroke-linejoin='round'%3E%3Cpath d='M12 21s-7-6.1-7-11.5a7 7 0 0 1 14 0C19 14.900 12 21 12 21z'/%3E%3Ccircle cx='12' cy='9.500' r='2.500'/%3E%3C/svg%3E")}
.x-head{display:flex;gap:16px;align-items:flex-start;margin:0 0 8px}
.x-logo{flex:none;width:80px;height:80px;margin-top:44px;object-fit:contain;border:1px solid var(--line);padding:6px;background:var(--card)}
.x-head>div{min-width:0}
.x-head h1{display:flex;flex-wrap:wrap;align-items:baseline;gap:8px 14px;font-size:clamp(34px,7vw,56px);margin-bottom:20px}
.x-head h1 .g-badge{margin:0;font:700 12px/24px var(--font);letter-spacing:.04em;color:var(--ok);border-color:currentColor;padding:0 9px 0 7px;vertical-align:0}
.x-head h1 .g-badge::before{content:"";display:inline-block;width:12px;height:12px;margin-right:5px;vertical-align:-1px;background:currentColor;-webkit-mask:var(--ck) center/contain no-repeat;mask:var(--ck) center/contain no-repeat;--ck:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23000' stroke-width='2.2' stroke-linecap='square'%3E%3Cpath d='M5 12.500l4.500 4.500L19 7'/%3E%3C/svg%3E")}
.x-intro{max-width:34em;margin:0 0 12px;font-size:18px;line-height:1.75;white-space:pre-line}
.x-contact{display:flex;flex-wrap:wrap;gap:4px 20px;margin:0}
.x-contact a{display:inline-flex;align-items:center;gap:8px;min-height:36px;font-size:14px;text-decoration:none;border-bottom:1px solid var(--red)}
.x-nw{white-space:nowrap}
h2.x-h2{font-size:15px;font-weight:800;letter-spacing:.02em;margin:32px 0 12px}
.g-foot{margin-top:32px}
</style>
