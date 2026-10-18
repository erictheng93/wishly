<script setup lang="ts">
// 探索：所有人的公開清單（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
useSeoMeta({ title: '探索公開清單｜Wishly', description: '瀏覽大家公開的心願清單與救助清單，登入會員後即可捐助。' })
const TYPES = [{ v: '', t: '全部' }, { v: 'registry', t: '禮物登記' }, { v: 'relief', t: '救助' }]
const typeLabel: Record<string, string> = { registry: '禮物登記', relief: '救助' }
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
    <h1>探索公開清單</h1>
    <div class="g-chips" role="group" aria-label="清單類型">
      <button v-for="o in TYPES" :key="o.v" class="g-btn ghost sm" :class="{ on: type === o.v }" :aria-pressed="type === o.v" @click="type = o.v">{{ o.t }}</button>
    </div>
    <div v-if="error" class="g-center"><h1>系統暫時無法載入，請稍後再試</h1><button class="g-btn" @click="refresh()">重新載入</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">目前沒有公開的清單。</div>
    <div v-else class="g-grid">
      <article v-for="w in list" :key="w.slug" class="g-card">
        <img v-if="w.cover_image_url" class="x-cover" :src="w.cover_image_url" alt="" loading="lazy">
        <NuxtLink :to="`/s/${w.slug}`" class="x-title">{{ w.title }}</NuxtLink>
        <p class="g-mute">
          {{ typeLabel[w.type] ?? w.type }}・{{ w.item_count }} 件<template v-if="w.event_date">・{{ w.event_date }}</template>
          <template v-if="w.owner?.display_name">・由
            <NuxtLink v-if="w.owner.handle" :to="`/u/${w.owner.handle}`">{{ w.owner.display_name }}</NuxtLink><template v-else>{{ w.owner.display_name }}</template>
          </template>
        </p>
        <div class="g-bar" role="img" :aria-label="`完成度 ${w.completion_pct}%`"><i :style="{ width: `${w.completion_pct}%` }" /></div>
        <span class="g-mute">完成度 {{ w.completion_pct }}%</span>
      </article>
    </div>
    <div v-if="next" class="g-center">
      <p v-if="moreErr" class="g-field-err" role="alert">載入失敗，請再試一次</p>
      <button class="g-btn ghost" :disabled="busy" @click="more">{{ busy ? '載入中…' : '載入更多' }}</button>
    </div>
  </main>
</template>

<style scoped>
.x-title{display:block;font-weight:700;font-size:18px;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:8px 0}
.x-cover{display:block;width:100%;aspect-ratio:16/9;object-fit:cover;margin-bottom:8px}
</style>
