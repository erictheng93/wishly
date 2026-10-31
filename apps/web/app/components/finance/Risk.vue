<script setup lang="ts">
// 風險與負債：點數負債（不隨區間）、退款與拒付、爭議中清單、待處理項目
const props = defineProps<{ days: number }>()
const { t, te } = useI18n()
const { api } = useApi()
const data = ref<any>(null), busy = ref(false), err = ref('')
let seq = 0
async function load() {
  const my = ++seq // 區間快速切換時只採用最後一次的回應
  busy.value = true; err.value = ''
  try {
    const r = await api('/admin/finance/risk', { query: { days: props.days } })
    if (my === seq) data.value = r
  } catch (e) { if (my === seq) err.value = errMsg(e) }
  finally { if (my === seq) busy.value = false }
}
watch(() => props.days, load)
onMounted(load)

const money = (n: number | null) => n == null ? '–' : t('admin.finance.money', { n: fmtPts(Math.round(n)) })
const label = (m: string) => methodLabel(m, t, te)
const LIAB = computed(() => {
  const l = data.value?.liability
  if (!l) return []
  return ['total', 'unused', 'refundable', 'non_refundable', 'pledged'].map(k => [k, l[k]] as const)
})
// count/amount 一組的項目；to 為既有頁面，沒有連結的不帶
const OPEN = computed(() => {
  const d = data.value
  if (!d) return []
  return [
    { k: 'cashRefunds', to: '/admin/cash-refunds', ...d.cash_refunds_open },
    { k: 'debts', to: '/admin/wallet-debts', ...d.debts_open },
    { k: 'invoices', to: '', ...d.invoices_pending },
  ]
})
</script>
<template>
  <div>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn small" :disabled="busy" @click="load">{{ $t('admin.finance.retry') }}</button></p>
    <p v-else-if="!data" class="c-mute">{{ $t('admin.finance.loading') }}</p>
    <template v-if="data">
      <section class="c-card" :aria-label="$t('admin.finance.risk.liability')">
        <h2 class="c-h2">{{ $t('admin.finance.risk.liability') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.risk.liabilityNote') }}</p>
        <dl class="k-kpis">
          <div v-for="x in LIAB" :key="x[0]">
            <dt>{{ $t(`admin.finance.risk.liab.${x[0]}`) }}</dt>
            <dd>{{ money(x[1]) }}</dd>
          </div>
        </dl>
      </section>

      <section class="c-card" :aria-label="$t('admin.finance.risk.loss')">
        <h2 class="c-h2">{{ $t('admin.finance.risk.loss') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.range', { from: data.range.from, to: data.range.to, days: data.range.days }) }}</p>
        <dl class="k-kpis">
          <div :title="$t('admin.finance.risk.lossRateDef')">
            <dt>{{ $t('admin.finance.risk.lossRate') }}</dt>
            <dd>{{ fmtRate(data.loss_rate) }}</dd>
            <small class="c-mute">{{ $t('admin.finance.risk.lossRateDef') }}</small>
          </div>
          <div>
            <dt>{{ $t('admin.finance.risk.refunds') }}</dt>
            <dd>{{ money(data.refunds.amount) }}</dd>
            <small class="c-mute">{{ $t('admin.finance.risk.cnt', { n: fmtCount(data.refunds.count) }) }}</small>
          </div>
          <div>
            <dt>{{ $t('admin.finance.risk.chargebacks') }}</dt>
            <dd>{{ money(data.chargebacks.amount) }}</dd>
            <small class="c-mute">{{ $t('admin.finance.risk.cnt', { n: fmtCount(data.chargebacks.count) }) }}</small>
          </div>
        </dl>
      </section>

      <section class="c-card" :aria-label="$t('admin.finance.risk.disputed')">
        <h2 class="c-h2">{{ $t('admin.finance.risk.disputed') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.risk.disputedNote') }}</p>
        <p v-if="!data.disputed.length" class="c-mute">{{ $t('admin.finance.risk.disputedEmpty') }}</p>
        <div v-else class="fin-scroll">
          <table class="fin-table">
            <thead><tr>
              <th>{{ $t('admin.finance.risk.user') }}</th><th>{{ $t('admin.finance.risk.amount') }}</th><th>{{ $t('admin.finance.channels.method') }}</th><th>{{ $t('admin.finance.risk.paidAt') }}</th><th />
            </tr></thead>
            <tbody>
              <tr v-for="d in data.disputed" :key="d.id">
                <td>{{ d.display_name }}</td><td>{{ money(d.amount) }}</td><td>{{ label(d.method) }}</td><td>{{ fmtDay(d.paid_at) }}</td>
                <td><NuxtLink :to="`/admin/finance?tab=transactions&status=disputed`">{{ $t('admin.finance.risk.handle') }}</NuxtLink></td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="c-card" :aria-label="$t('admin.finance.risk.open')">
        <h2 class="c-h2">{{ $t('admin.finance.risk.open') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.todo.note') }}</p>
        <dl class="k-kpis">
          <div v-for="x in OPEN" :key="x.k">
            <dt>{{ $t(`admin.finance.risk.${x.k}`) }}</dt>
            <dd :class="{ hot: x.k !== 'invoices' && x.count > 0 }">{{ fmtCount(x.count) }}</dd>
            <small class="c-mute">{{ money(x.amount) }}</small>
            <small v-if="x.k === 'invoices'" class="c-mute fin-note">{{ $t('admin.finance.risk.invoicesNote') }}</small>
          </div>
          <div>
            <dt>{{ $t('admin.finance.risk.frozen') }}</dt>
            <dd>{{ fmtCount(data.frozen_wallets) }}</dd>
          </div>
        </dl>
        <p class="c-row wrap">
          <NuxtLink v-for="x in OPEN.filter(o => o.to)" :key="x.k" :to="x.to" class="c-btn small">{{ $t(`admin.finance.risk.go.${x.k}`) }}</NuxtLink>
        </p>
      </section>
    </template>
  </div>
</template>
<style scoped>
.k-kpis dd.hot{color:var(--red-text)}
.fin-note{display:block;margin-top:6px}
</style>
