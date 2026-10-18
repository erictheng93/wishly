<script setup lang="ts">
// 探索：所有人的公開清單（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
useSeoMeta({ title: () => t('guest.explore.seoTitle'), description: () => t('guest.explore.seoDesc') })
const TYPES = computed(() => [{ v: '', t: t('guest.explore.type.all') }, { v: 'registry', t: t('guest.explore.type.registry') }, { v: 'relief', t: t('guest.explore.type.relief') }])
const typeLabel = (k: string) => ['registry', 'relief'].includes(k) ? t(`guest.explore.type.${k}`) : k
const type = ref('')
const { data, error, refresh } = await useAsyncData(
  'explore',
  () => $fetch<any>(`${apiBase}/api/v1/public/wishlists`, { query: { type: type.value || undefined } }),
  { watch: [type] },
)
// 「載入更多」接在第一頁後面；換篩選或重抓時清掉
const extra = ref<any[]>([]), extraNext = ref<number | null | undefined>(undefined), busy = ref(false), moreErr = ref(false)
watch(data, () => { extra.value = []; extraNext.value = undefined })
const list = computed(() => [...(data.value?.wishlists ?? []), ...extra.value])
const next = computed(() => extraNext.value !== undefined ? extraNext.value : data.value?.next_page ?? null)
async function more() {
  busy.value = true; moreErr.value = false
  try {
    const r = await $fetch<any>(`${apiBase}/api/v1/public/wishlists`, { query: { page: next.value, type: type.value || undefined } })
    extra.value = [...extra.value, ...r.wishlists]; extraNext.value = r.next_page
  } catch { moreErr.value = true } finally { busy.value = false }
}
</script>

<template>
  <GuestTop />
  <main class="g-wrap">
    <h1>{{ $t('guest.explore.heading') }}</h1>
    <div class="g-chips" role="group" :aria-label="$t('guest.explore.typeGroup')">
      <button v-for="o in TYPES" :key="o.v" class="g-btn ghost sm" :class="{ on: type === o.v }" :aria-pressed="type === o.v" @click="type = o.v">{{ o.t }}</button>
    </div>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">{{ $t('guest.explore.empty') }}</div>
    <div v-else class="g-grid">
      <article v-for="w in list" :key="w.slug" class="g-card">
        <img v-if="w.cover_image_url" class="x-cover" :src="w.cover_image_url" alt="" loading="lazy">
        <NuxtLink :to="`/s/${w.slug}`" class="x-title">{{ w.title }}</NuxtLink>
        <p class="g-mute">
          {{ typeLabel(w.type) }}・{{ $t('guest.explore.itemCount', { n: w.item_count }) }}<template v-if="w.event_date">・{{ w.event_date }}</template>
          <template v-if="w.owner?.display_name">{{ $t('guest.explore.by') }}
            <NuxtLink v-if="w.owner.handle" :to="`/u/${w.owner.handle}`">{{ w.owner.display_name }}</NuxtLink><template v-else>{{ w.owner.display_name }}</template>
          </template>
        </p>
        <div class="g-bar" role="img" :aria-label="$t('guest.explore.completionAria', { pct: w.completion_pct })"><i :style="{ width: `${w.completion_pct}%` }" /></div>
        <span class="g-mute">{{ $t('guest.explore.completion', { pct: w.completion_pct }) }}</span>
      </article>
    </div>
    <div v-if="next" class="g-center">
      <p v-if="moreErr" class="g-field-err" role="alert">{{ $t('guest.explore.moreErr') }}</p>
      <button class="g-btn ghost" :disabled="busy" @click="more">{{ busy ? $t('guest.loading') : $t('guest.explore.loadMore') }}</button>
    </div>
  </main>
</template>

<style scoped>
.x-title{display:block;font-weight:700;font-size:18px;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:8px 0}
.x-cover{display:block;width:100%;aspect-ratio:16/9;object-fit:cover;margin-bottom:8px}
</style>
