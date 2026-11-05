<script setup lang="ts">
// 探索：所有人的公開清單（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
const orgsOn = await useOrgsEnabled()
// 行銷頁內容欄放寬到 1200（頁首跟著 --g-w 對齊）；head 標籤隨頁面卸載，不外洩到其他頁
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
  <main class="g-wrap g-wide">
    <h1 class="x-h1">{{ $t('guest.explore.heading') }}</h1>
    <NuxtLink v-if="orgsOn" :to="localizedPath('/e', $i18n.locale)" class="g-banner x-relief"><b>{{ $t('guest.relief.banner') }}</b>・{{ $t('guest.relief.bannerBody') }}<UiIcon name="arrow-right" :size="24" /></NuxtLink>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!list.length" class="g-center g-mute">{{ $t('guest.explore.empty') }}</div>
    <div v-else class="g-grid">
      <article v-for="w in list" :key="w.slug" class="g-card x-card">
        <div class="x-cv"><img class="x-cover" :src="coverSrc(w)" alt="" loading="lazy"></div>
        <NuxtLink :to="localizedPath(`/s/${w.slug}`, $i18n.locale)" class="x-title">{{ w.title }}</NuxtLink>
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
.x-h1{font-size:clamp(52px,9.4vw,136px);font-weight:900;letter-spacing:-.05em;line-height:.98;margin:48px 0 32px}
html:is(:lang(en),:lang(vi),:lang(id)) .x-h1{letter-spacing:-.03em}
html:lang(vi) .x-h1{line-height:1.18}
.x-relief{display:flex;align-items:center;gap:8px;min-height:80px;margin:0 0 56px;padding:0 24px;font-size:18px;color:var(--fg);text-decoration:none;box-shadow:none;border:1px solid var(--ink-line);background:var(--hatch),var(--card);transition:background var(--t) var(--ease)}
.x-relief::before{position:static;flex:0 0 8px;width:8px;height:8px;margin-right:4px}
.x-relief b{font-size:20px;white-space:nowrap}
.x-relief .ui-icon{margin-left:auto;color:var(--red-text);transition:transform var(--t) var(--ease)}
.x-relief:hover{background:var(--bg-alt)}
.x-relief:hover .ui-icon{transform:translateX(6px)}
@media (max-width:767px){.x-relief{font-size:14px;padding:12px 16px 12px 24px}.x-relief b{font-size:17px}}
.g-grid{grid-template-columns:repeat(auto-fill,minmax(min(100%,340px),1fr));gap:32px 24px}
.x-card{display:flex;flex-direction:column;padding:0;background:var(--card)}
.x-card>:not(.x-cv){margin-left:16px;margin-right:16px}
.x-cv{position:relative;margin-bottom:4px;border-bottom:1px solid var(--line)}
.x-title{display:block;font-weight:800;font-size:22px;line-height:1.35;color:var(--fg);text-decoration:none;overflow-wrap:anywhere;padding:16px 0 6px}
.x-title:hover{color:var(--red-text)}
.x-card .g-mute{font-size:13px}
.x-card .g-mute:not(.x-pct){font-family:var(--mono);letter-spacing:.04em}
.x-card .g-bar{height:10px;margin-top:16px}
.x-pct{display:block;margin-bottom:16px!important;font:700 12px var(--mono);letter-spacing:.1em}
/* 預設封面是方形插圖：一律 2:1 填滿裁切，半寬卡片不被撐成巨大色塊 */
.x-cover{display:block;width:100%;aspect-ratio:2/1;object-fit:cover;object-position:center 30%}
@media (hover:hover) and (pointer:fine){.x-card:hover{box-shadow:8px 8px 0 var(--bg-alt),8px 8px 0 1px var(--line);transform:translate(-3px,-3px);border-color:var(--ink-line)}}
/* 第一張放大成橫幅：封面在左，標題與進度在右；張數少時重心落在最新一份 */
@media (min-width:1100px){
  .g-grid{grid-template-columns:repeat(6,minmax(0,1fr))}
  .g-grid>.x-card{grid-column:span 2}
  .g-grid>.x-card:first-child:nth-last-child(3)~.x-card{grid-column:span 3}
  .g-grid>.x-card:first-child:nth-last-child(2)~.x-card{grid-column:1/-1}
  .g-grid>.x-card:first-child{grid-column:1/-1;display:grid;grid-template-columns:1fr 1fr;align-content:start;min-height:460px;grid-auto-rows:min-content;grid-template-rows:repeat(5,min-content) 1fr}
  .g-grid>.x-card:first-child>:not(.x-cv){grid-column:2;margin-left:40px;margin-right:40px}
  .g-grid>.x-card:first-child>.x-cv{grid-column:1;grid-row:1/-1;align-self:stretch;min-height:380px;margin:0;border-bottom:0;border-right:1px solid var(--line)}
  .g-grid>.x-card:first-child .x-cover{position:absolute;inset:0;aspect-ratio:auto;height:100%}
  .g-grid>.x-card:first-child .x-title{margin-top:clamp(40px,9vw,120px);font-size:clamp(30px,3.2vw,44px);font-weight:900;letter-spacing:-.03em;line-height:1.1}
  .g-grid>.x-card:first-child .g-bar{height:14px}
}
</style>
