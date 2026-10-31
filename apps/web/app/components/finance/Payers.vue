<script setup lang="ts">
// 付費用戶：數字、金額級距分布、前 20 名（點擊到交易明細）
const props = defineProps<{ days: number }>()
const { t } = useI18n()
const { api } = useApi()
const data = ref<any>(null), busy = ref(false), err = ref('')
let seq = 0
async function load() {
  const my = ++seq // 區間快速切換時只採用最後一次的回應
  busy.value = true; err.value = ''
  try {
    const r = await api('/admin/finance/payers', { query: { days: props.days } })
    if (my === seq) data.value = r
  } catch (e) { if (my === seq) { err.value = errMsg(e); data.value = null } } // 失敗時不留舊區間的數字
  finally { if (my === seq) busy.value = false }
}
watch(() => props.days, load)
onMounted(load)

const money = (n: number | null) => n == null ? '–' : t('admin.finance.money', { n: fmtPts(Math.round(n)) })
const KPIS = computed(() => {
  const d = data.value
  if (!d) return []
  return [
    ['payers', fmtCount(d.payers)],
    ['firstTime', fmtCount(d.first_time)],
    ['repeat', fmtRate(d.repeat_rate)],
    ['avg', money(perPayer(d.gross, d.payers))],
  ]
})
const BUCKETS = computed(() => {
  const rows: { key: string; count: number; amount: number }[] = data.value?.buckets ?? []
  const max = Math.max(1, ...rows.map(r => r.count))
  return rows.map(r => ({ ...r, w: r.count / max * 100 }))
})
</script>
<template>
  <div :aria-busy="busy" :class="{ 'fin-busy': busy }">
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn small" :disabled="busy" @click="load">{{ $t('admin.finance.retry') }}</button></p>
    <p v-else-if="!data" class="c-mute">{{ $t('admin.finance.loading') }}</p>
    <template v-if="data">
      <section class="c-card" :aria-label="$t('admin.finance.payers.title')">
        <h2 class="c-h2">{{ $t('admin.finance.payers.title') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.range', { from: data.range.from, to: data.range.to, days: data.range.days }) }}</p>
        <p v-if="!data.payers" class="c-mute">{{ $t('admin.finance.payers.empty') }}</p>
        <dl v-else class="k-kpis">
          <div v-for="k in KPIS" :key="k[0]" :title="$t(`admin.finance.payers.${k[0]}Def`)">
            <dt>{{ $t(`admin.finance.payers.${k[0]}`) }}</dt>
            <dd :class="{ money: k[0] === 'avg' }">{{ k[1] }}</dd>
            <small class="c-mute">{{ $t(`admin.finance.payers.${k[0]}Def`) }}</small>
          </div>
        </dl>
      </section>

      <section v-if="data.payers" class="c-card" :aria-label="$t('admin.finance.payers.buckets')">
        <h2 class="c-h2">{{ $t('admin.finance.payers.buckets') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.payers.bucketsNote') }}</p>
        <div v-for="b in BUCKETS" :key="b.key" class="an-row">
          <span class="an-l">{{ $t(`admin.finance.payers.bucket.${b.key}`) }}</span>
          <span class="an-track"><span class="an-bar" :style="{ width: b.w + '%' }" /></span>
          <span class="an-v">{{ fmtCount(b.count) }}<small class="c-mute"> {{ money(b.amount) }}</small></span>
        </div>
      </section>

      <section v-if="data.payers" class="c-card" :aria-label="$t('admin.finance.payers.top')">
        <h2 class="c-h2">{{ $t('admin.finance.payers.top') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.payers.topNote') }}</p>
        <div class="fin-scroll">
          <table class="fin-table">
            <thead><tr>
              <th>{{ $t('admin.finance.payers.user') }}</th><th>{{ $t('admin.finance.payers.amount') }}</th><th>{{ $t('admin.finance.payers.count') }}</th>
              <th>{{ $t('admin.finance.payers.first') }}</th><th>{{ $t('admin.finance.payers.last') }}</th>
            </tr></thead>
            <tbody>
              <tr v-for="r in data.top" :key="r.user.id">
                <td><NuxtLink :to="`/admin/finance?tab=transactions&user_id=${r.user.id}`">{{ r.user.display_name }}</NuxtLink><small v-if="r.user.email" class="c-mute">{{ r.user.email }}</small></td>
                <td>{{ money(r.amount) }}</td><td>{{ fmtCount(r.count) }}</td><td>{{ fmtDay(r.first_paid_at) }}</td><td>{{ fmtDay(r.last_paid_at) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </template>
  </div>
</template>
