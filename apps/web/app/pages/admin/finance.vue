<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 金流後台：總覽＋分頁。分頁放網址 ?tab=，時間區間跨分頁共用。
import { FINANCE_RANGES, FINANCE_TABS, type FinanceTab } from '~/utils/finance'
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.finance.title') })
const route = useRoute(), router = useRouter()
const tab = computed(() => financeTab(route.query.tab))
const days = ref(30)
const TABS = FINANCE_TABS, RANGES = FINANCE_RANGES
const setTab = (k: FinanceTab) => router.replace({ query: { tab: k } })
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.finance.title')" back="/admin" />
    <nav class="c-tabs" :aria-label="$t('admin.finance.tabsLabel')">
      <button v-for="k in TABS" :key="k" class="c-btn" :class="{ on: tab === k }" :aria-current="tab === k ? 'page' : undefined" @click="setTab(k)">{{ $t(`admin.finance.tabs.${k}`) }}</button>
    </nav>
    <div v-if="tab !== 'transactions'" class="c-row wrap c-mb an-range">
      <div class="c-row c-grow" role="group" :aria-label="$t('admin.finance.rangeLabel')">
        <button v-for="n in RANGES" :key="n" class="c-btn c-grow" :class="{ on: days === n }" :aria-pressed="days === n" @click="days = n">{{ n === 1 ? $t('admin.finance.today') : $t('admin.finance.days', { n }) }}</button>
      </div>
    </div>
    <FinanceOverview v-if="tab === 'overview'" :days="days" />
    <FinancePayers v-else-if="tab === 'payers'" :days="days" />
    <FinanceChannels v-else-if="tab === 'channels'" :days="days" />
    <FinanceRisk v-else-if="tab === 'risk'" :days="days" />
    <FinanceTransactions v-else />
  </main>
</template>
