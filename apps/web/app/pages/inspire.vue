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
    <h1>{{ $t('guest.inspire.heading') }}</h1>
    <template v-for="g in groups" :key="g.key">
      <div class="g-chips" role="group" :aria-label="$t(`guest.inspire.${g.key}`)">
        <span class="g-mute x-lab">{{ $t(`guest.inspire.${g.key}`) }}</span>
        <button v-for="o in [{ v: '', t: $t('guest.inspire.all') }, ...g.opts]" :key="o.v" class="g-btn ghost sm" :class="{ on: g.model.value === o.v }" :aria-pressed="g.model.value === o.v" @click="g.model.value = o.v">{{ o.t }}</button>
      </div>
    </template>
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
.x-lab{align-self:center;min-width:3em}
.x-soon{display:block;font-size:20px;color:var(--fg);margin-bottom:8px}
</style>
