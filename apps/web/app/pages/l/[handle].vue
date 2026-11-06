<script setup lang="ts">
// 連結頁（link-in-bio）：匿名、SSR、可被 CDN 快取，所以不帶 cookie
const handle = String(useRoute().params.handle)
const { t } = useI18n()
const config = useRuntimeConfig()
const origin = useRequestURL().origin
const { data, error } = await useFetch<any>(`${config.public.apiBase}/api/v1/public/links/${encodeURIComponent(handle)}`, { credentials: 'omit', server: true, key: `links-${handle}`, headers: { accept: 'application/json' } })
const u = computed(() => data.value?.user)
useSeoMeta({
  title: () => u.value?.display_name ?? t('me.profile.page'),
  ogTitle: () => u.value?.display_name ?? t('me.profile.page'),
  ogDescription: () => u.value?.bio || undefined,
  ogImage: () => u.value?.avatar_url || `${origin}/og-default.png`,
  twitterCard: 'summary', robots: 'noindex',
})
</script>
<template>
  <main class="c-page">
    <section v-if="error || !u" class="c-gone"><p class="c-gone-t">{{ $t('me.profile.gone') }}</p></section>
    <template v-else>
      <img v-if="u.cover_image_url || u.cover_preset" :src="u.cover_image_url || `/covers/${u.cover_preset}.svg`" alt="" class="lk-cover">
      <section class="lk-head">
        <div class="lk-av" aria-hidden="true"><img v-if="u.avatar_url" :src="u.avatar_url" alt="" class="lk-img"><template v-else>{{ (u.display_name || '?').slice(0, 1) }}</template></div>
        <h1 class="c-h2">{{ u.display_name }}</h1>
        <p v-if="u.bio" class="lk-bio">{{ u.bio }}</p>
      </section>
      <a v-for="l in data.links" :key="l.url" :href="l.url" target="_blank" rel="nofollow ugc noopener noreferrer" class="c-btn block lk-btn"><UiIcon :name="(linkIcon(l.url) as any)" :size="18" />{{ l.title }}</a>
      <NuxtLink :to="`/u/${u.handle ?? handle}`" class="c-btn primary block lk-btn">{{ $t('me.profile.seeWishes') }}</NuxtLink>
    </template>
  </main>
</template>
<style scoped>
.lk-cover{display:block;width:100%;height:140px;object-fit:cover;background:var(--bg-alt)}
.lk-head{display:grid;justify-items:center;gap:8px;padding:24px 0;text-align:center}
.lk-av{width:80px;height:80px;display:grid;place-items:center;background:var(--fg);color:var(--bg);font:900 32px var(--font);overflow:hidden}
.lk-img{width:100%;height:100%;object-fit:cover}
.lk-head .c-h2{margin:0}
.lk-bio{white-space:pre-line;margin:0}
.lk-btn{display:flex;align-items:center;justify-content:center;gap:8px;margin-bottom:10px}
</style>
