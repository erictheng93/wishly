<script setup lang="ts">
// 探索：所有人的公開清單（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
const orgsOn = await useOrgsEnabled()
useSeoMeta({ title: () => t('guest.explore.seoTitle'), description: () => t('guest.explore.seoDesc') })
// personal 與舊 registry 同為「心願清單」
const typeLabel = (k: string) => t(k === 'relief' ? 'guest.explore.type.relief' : 'guest.explore.type.wishlist')
const { data, error, refresh } = await useAsyncData(
  'explore',
  () => $fetch<any>(`${apiBase}/api/v1/public/wishlists`),
)
// 「載入更多」接在第一頁後面；重抓時清掉
const extra = ref<any[]>([]), extraNext = ref<number | null | undefined>(undefined), busy = ref(false), moreErr = ref(false)
watch(data, () => { extra.value = []; extraNext.value = undefined })
const list = computed(() => [...(data.value?.wishlists ?? []), ...extra.value])
const next = computed(() => extraNext.value !== undefined ? extraNext.value : data.value?.next_page ?? null)
async function more() {
  busy.value = true; moreErr.value = false
  try {
    const r = await $fetch<any>(`${apiBase}/api/v1/public/wishlists`, { query: { page: next.value } })
    extra.value = [...extra.value, ...r.wishlists]; extraNext.value = r.next_page
  } catch { moreErr.value = true } finally { busy.value = false }
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <p class="x-idx" aria-hidden="true"><span>INDEX / 010</span><span>PUBLIC REGISTRY</span></p>
    <h1>{{ $t('guest.explore.heading') }}</h1>
    <NuxtLink v-if="orgsOn" to="/e" class="g-banner x-relief"><b>{{ $t('guest.relief.banner') }}</b>・{{ $t('guest.relief.bannerBody') }}</NuxtLink>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">{{ $t('guest.explore.empty') }}</div>
    <div v-else class="g-grid">
      <article v-for="(w, i) in list" :key="w.slug" class="g-card x-card">
        <div class="x-cv"><img class="x-cover" :src="coverSrc(w)" alt="" loading="lazy"><span class="x-no" aria-hidden="true">{{ String(i + 1).padStart(2, '0') }}</span></div>
        <NuxtLink :to="`/s/${w.slug}`" class="x-title">{{ w.title }}</NuxtLink>
        <p class="g-mute">
          {{ typeLabel(w.type) }}・{{ $t('guest.explore.itemCount', { n: w.item_count }, w.item_count) }}<template v-if="w.event_date">・{{ w.event_date }}</template>
          <template v-if="w.owner?.display_name">{{ $t('guest.explore.by') }}
            <NuxtLink v-if="w.owner.handle" :to="`/u/${w.owner.handle}`">{{ w.owner.display_name }}</NuxtLink><template v-else>{{ w.owner.display_name }}</template>
          </template>
        </p>
        <div class="g-bar" role="img" :aria-label="$t('guest.explore.completionAria', { pct: w.completion_pct })"><i :style="{ width: `${w.completion_pct}%` }" /></div>
        <span class="g-mute x-pct">{{ $t('guest.explore.completion', { pct: w.completion_pct }) }}</span>
      </article>
    </div>
    <div v-if="next" class="g-center">
      <p v-if="moreErr" class="g-field-err" role="alert">{{ $t('guest.explore.moreErr') }}</p>
      <button class="g-btn ghost" :disabled="busy" @click="more">{{ busy ? $t('guest.loading') : $t('guest.explore.loadMore') }}</button>
    </div>
  </main>
</template>

<style scoped>
.x-idx{display:flex;justify-content:space-between;gap:16px;margin:28px 0 0;padding-bottom:12px;border-bottom:1px solid var(--ink-line);font:600 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-idx span:first-child{color:var(--red-text)}
.x-idx+h1{margin-top:20px;font-size:clamp(40px,8vw,84px);letter-spacing:-.04em;font-weight:900}
.x-relief{display:flex;align-items:center;gap:8px;min-height:56px;margin:20px 0 28px;color:var(--fg);text-decoration:none;box-shadow:none;border:1px solid var(--ink-line);background:var(--hatch),var(--card)}
.x-relief::before{content:"";flex:none;width:10px;height:10px;background:var(--red)}
.x-relief::after{content:"→";margin-left:auto;font-weight:700;color:var(--red-text)}
.x-relief:hover{background:var(--bg-alt)}
.x-card{display:flex;flex-direction:column;padding:0}
.x-card>:not(.x-cv){margin-left:16px;margin-right:16px}
.x-cv{position:relative;margin-bottom:4px;border-bottom:1px solid var(--line)}
.x-no{position:absolute;left:0;top:0;padding:0 8px;background:var(--fg);color:var(--bg);font:700 11px/24px var(--mono);letter-spacing:.14em}
.x-title{display:block;font-weight:800;font-size:19px;line-height:1.35;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:10px 0 4px}
.x-title:hover{color:var(--red-text)}
.x-card .g-mute{font-size:12px}
.x-card .g-mute:not(.x-pct){font-family:var(--mono);letter-spacing:.04em}
.x-card .g-bar{margin-top:12px}
.x-pct{margin-bottom:16px!important;font:600 11px var(--mono);letter-spacing:.1em}
.x-cover{display:block;width:100%;aspect-ratio:16/9;object-fit:cover}
.x-cover[src$='.svg']{object-fit:contain;background:var(--bg-alt)}
</style>
