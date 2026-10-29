<script setup lang="ts">
import { BUDGETS, OCCASIONS, RECIPIENTS } from '~/utils/catalog'
// 選物靈感：Wishly 精選商品目錄（訪客可看，SSR）；無購物車，只能加到自己的清單或看外部商品
const { t } = useI18n()
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
  <main class="g-wrap">
    <p class="x-idx" aria-hidden="true"><span>INDEX / 020</span><span>CURATED CATALOG</span></p>
    <h1>{{ $t('guest.inspire.heading') }}</h1>
    <div class="x-filters">
    <template v-for="g in groups" :key="g.key">
      <div class="g-chips x-row" role="group" :aria-label="$t(`guest.inspire.${g.key}`)">
        <span class="x-lab">{{ $t(`guest.inspire.${g.key}`) }}</span>
        <button v-for="o in [{ v: '', t: $t('guest.inspire.all') }, ...g.opts]" :key="o.v" class="g-btn ghost sm" :class="{ on: g.model.value === o.v }" :aria-pressed="g.model.value === o.v" @click="g.model.value = o.v">{{ o.t }}</button>
      </div>
    </template>
    </div>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!products.length" class="g-center g-mute">
      <template v-if="filtered">{{ $t('guest.inspire.empty') }}</template>
      <template v-else><strong class="x-soon">{{ $t('guest.inspire.soonTitle') }}</strong><br>{{ $t('guest.inspire.soonBody') }}</template>
    </div>
    <GuestCatalogGrid v-else :products="products" />
    <div v-if="next" class="g-center">
      <p v-if="moreErr" class="g-field-err" role="alert">{{ $t('guest.explore.moreErr') }}</p>
      <button class="g-btn ghost" :disabled="busy" @click="more">{{ busy ? $t('guest.loading') : $t('guest.explore.loadMore') }}</button>
    </div>
  </main>
</template>

<style scoped>
.x-idx{display:flex;justify-content:space-between;gap:16px;margin:28px 0 0;padding-bottom:12px;border-bottom:1px solid var(--ink-line);font:600 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-idx span:first-child{color:var(--red-text)}
.x-idx+h1{margin:20px 0 24px;font-size:clamp(40px,8vw,84px);letter-spacing:-.04em;font-weight:900}
.x-filters{margin:12px 0 28px;border-top:1px solid var(--ink-line);border-bottom:1px solid var(--line)}
.x-row{display:flex;flex-wrap:nowrap;align-items:center;gap:6px;margin:0;padding:8px 0;border-bottom:1px solid var(--line);overflow-x:auto;scrollbar-width:none}
.x-row:last-child{border-bottom:0}
.x-row::-webkit-scrollbar{display:none}
.x-row .g-btn{flex:none;min-height:40px;padding:0 14px;font-size:13px}
.x-lab{flex:none;min-width:5em;font:700 11px/1 var(--mono);letter-spacing:.16em;color:var(--mute);text-transform:uppercase}
.x-lab::before{content:"";display:inline-block;width:6px;height:6px;margin-right:8px;background:var(--red)}
.x-soon{display:block;font-size:20px;color:var(--fg);margin-bottom:8px}
@media (max-width:767px){.x-row{flex-wrap:nowrap}.x-lab{min-width:4em}}
</style>
