<script setup lang="ts">
import { BUDGETS, OCCASIONS, RECIPIENTS } from '~/utils/catalog'
// 選物靈感：Wishly 精選商品目錄（訪客可看，SSR）；無購物車，只能加到自己的清單或看外部商品
const { t } = useI18n()
// 行銷頁內容欄放寬到 1200（頁首跟著 --g-w 對齊）；head 標籤隨頁面卸載，不外洩到其他頁
useSeoMeta({ title: () => t('guest.inspire.seoTitle'), description: () => t('guest.inspire.seoDesc') })
const recipient = ref(''), occasion = ref(''), budget = ref('')
const filter = () => ({ recipient: recipient.value, occasion: occasion.value, budget: budget.value })
const { products, next, error, refresh, more, busy, moreErr } = await useCatalog('inspire', filter)
const filtered = computed(() => !!(recipient.value || occasion.value || budget.value))
const groups = computed(() => [
  { key: 'recipient', model: recipient, opts: RECIPIENTS.map(v => ({ v, t: t(`guest.inspire.r.${v}`) })) },
  { key: 'occasion', model: occasion, opts: OCCASIONS.map(v => ({ v, t: t(`guest.inspire.o.${v}`) })) },
  { key: 'budget', model: budget, opts: BUDGETS.map(v => ({ v, t: t(`guest.inspire.b.${v}`) })) },
])
</script>

<template>
  <GuestTop />
  <main class="g-wrap g-wide x-main">
    <h1 class="x-h1">{{ $t('guest.inspire.heading') }}</h1>
    <nav class="x-scenes" :aria-label="$t('guest.inspire.scenes')"><NuxtLink v-for="s in SCENES" :key="s.slug" class="x-scene" :to="localizedPath(`/inspire/${s.slug}`, $i18n.locale)">{{ $t(`inspire.scenes.${s.slug}.heading`) }}<UiIcon name="arrow-right" :size="22" /></NuxtLink></nav>
    <div class="x-filters">
    <template v-for="g in groups" :key="g.key">
      <div class="g-chips x-row" role="group" :aria-label="$t(`guest.inspire.${g.key}`)">
        <span class="x-lab">{{ $t(`guest.inspire.${g.key}`) }}</span>
        <button v-for="o in [{ v: '', t: $t('guest.inspire.all') }, ...g.opts]" :key="o.v" class="g-btn ghost sm" :class="{ on: g.model.value === o.v }" :aria-pressed="g.model.value === o.v" @click="g.model.value = o.v">{{ o.t }}</button>
      </div>
    </template>
    </div>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!products.length" class="g-center g-mute g-shelf">
      <template v-if="filtered">{{ $t('guest.inspire.empty') }}</template>
      <template v-else><strong class="g-shelf-t">{{ $t('guest.inspire.soonTitle') }}</strong><br>{{ $t('guest.inspire.soonBody') }}</template>
    </div>
    <GuestCatalogGrid v-else :products="products" />
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
@media (min-width:1100px){
  .x-main{display:grid;grid-template-columns:minmax(0,7fr) minmax(0,4fr);column-gap:80px;align-items:start}
  .x-main>.x-h1,.x-main>.g-center,.x-main>:deep(.x-grid){grid-column:1/-1}
  .x-scenes{grid-column:1;grid-row:2}
  .x-filters{grid-column:2;grid-row:2;position:sticky;top:calc(var(--head-h) + 24px);margin:0!important}
}
/* 情境入口：整列大字，右側線箭頭，hover 內縮並轉紅 */
.x-scenes{margin:0;border-bottom:1px solid var(--ink-line)}
.x-scene{display:flex;align-items:center;justify-content:space-between;gap:24px;min-height:76px;padding:14px 0;border-top:1px solid var(--line);color:var(--fg);font-size:clamp(17px,1.7vw,22px);font-weight:800;line-height:1.4;transition:padding var(--t) var(--ease),color var(--t) var(--ease)}
.x-scene:first-child{border-top-color:var(--ink-line)}
.x-scene .ui-icon{transition:transform var(--t) var(--ease)}
@media (hover:hover) and (pointer:fine){.x-scene:hover{color:var(--red-text);padding-left:14px}.x-scene:hover .ui-icon{transform:translateX(6px)}}
.x-filters{margin:40px 0;border-top:1px solid var(--ink-line);border-bottom:1px solid var(--line)}
.x-row{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin:0;padding:16px 0;border-bottom:1px solid var(--line)}
.x-row:last-child{border-bottom:0}
.x-row .g-btn{min-height:44px;padding:0 14px;font-size:14px}
.x-lab{flex:1 0 100%;font:700 12px/1 var(--mono);letter-spacing:.16em;color:var(--mute);text-transform:uppercase}
.x-main :deep(.x-grid){margin-top:72px}
</style>
