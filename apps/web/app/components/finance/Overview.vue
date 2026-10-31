<script setup lang="ts">
// 金流總覽：待處理（不隨區間）、核心數字（含前期比較）、每日實收折線
const props = defineProps<{ days: number }>()
const { t } = useI18n()
const { api } = useApi()
const data = ref<any>(null), busy = ref(false), err = ref('')
let seq = 0
async function load() {
  const my = ++seq // 區間快速切換時只採用最後一次的回應
  busy.value = true; err.value = ''
  try {
    const r = await api('/admin/finance/overview', { query: { days: props.days } })
    if (my === seq) data.value = r
  } catch (e) { if (my === seq) err.value = errMsg(e) }
  finally { if (my === seq) busy.value = false }
}
watch(() => props.days, load)
onMounted(load)

const money = (n: number | null) => n == null ? '–' : t('admin.finance.money', { n: fmtPts(Math.round(n)) })
const TODO = computed(() => {
  const d = data.value?.todo
  if (!d) return []
  return [
    { k: 'disputed', to: '/admin/finance?tab=transactions&status=disputed', ...d.disputed },
    { k: 'cashRefunds', to: '/admin/cash-refunds', ...d.cash_refunds },
    { k: 'debts', to: '/admin/wallet-debts', ...d.debts },
    { k: 'pending', to: '/admin/finance?tab=transactions&status=pending', ...d.pending },
  ]
})
const KPIS = computed(() => {
  const k = data.value?.kpis
  if (!k) return []
  const rows: [string, number | null, number | null, boolean][] = [
    ['gross', k.cur.gross, k.prev.gross, true],
    ['payers', k.cur.payers, k.prev.payers, false],
    ['firstTime', k.cur.first_time, k.prev.first_time, false],
    ['avg', perPayer(k.cur.gross, k.cur.payers), perPayer(k.prev.gross, k.prev.payers), true],
  ]
  return rows.map(([key, cur, prev, isMoney]) => {
    const d = cur == null || prev == null ? null : delta(cur, prev)
    return { key, value: isMoney ? money(cur) : fmtCount(cur ?? undefined), prev: isMoney ? money(prev) : fmtCount(prev ?? undefined), d: d == null ? null : Math.round(d) }
  })
})
const W = 200, H = 48
const chart = computed(() => {
  const rows: { date: string; gross: number }[] = data.value?.daily ?? []
  const max = Math.max(1, ...rows.map(r => r.gross))
  const x = (i: number) => rows.length > 1 ? i / (rows.length - 1) * W : W / 2
  const pts = rows.map((r, i) => `${x(i).toFixed(1)},${(H - 2 - r.gross / max * (H - 6)).toFixed(1)}`).join(' ')
  return { rows, pts, area: rows.length ? `0,${H} ${pts} ${W},${H}` : '', step: W / Math.max(1, rows.length) }
})
</script>
<template>
  <div>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn small" :disabled="busy" @click="load">{{ $t('admin.finance.retry') }}</button></p>
    <p v-else-if="!data" class="c-mute">{{ $t('admin.finance.loading') }}</p>
    <template v-if="data">
      <section class="c-card" :aria-label="$t('admin.finance.todo.title')">
        <h2 class="c-h2">{{ $t('admin.finance.todo.title') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.todo.note') }}</p>
        <dl class="k-kpis">
          <NuxtLink v-for="x in TODO" :key="x.k" :to="x.to" class="fin-todo">
            <dt>{{ $t(`admin.finance.todo.${x.k}`) }}</dt>
            <dd :class="{ hot: x.count > 0 }">{{ fmtCount(x.count) }}</dd>
            <small class="c-mute">{{ money(x.amount) }}</small>
          </NuxtLink>
        </dl>
      </section>

      <section class="c-card" :aria-label="$t('admin.finance.kpi.title')">
        <h2 class="c-h2">{{ $t('admin.finance.kpi.title') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.range', { from: data.range.from, to: data.range.to, days: data.range.days }) }}</p>
        <dl class="k-kpis">
          <div v-for="k in KPIS" :key="k.key" :title="$t(`admin.finance.kpi.${k.key}Def`)">
            <dt>{{ $t(`admin.finance.kpi.${k.key}`) }}</dt>
            <dd>{{ k.value }}</dd>
            <small class="c-mute fin-cmp">{{ k.d == null ? $t('admin.finance.kpi.noPrev') : $t('admin.finance.kpi.vsPrev', { pct: (k.d > 0 ? '+' : '') + k.d + '%' }) }}・{{ k.prev }}</small>
            <small class="c-mute fin-def">{{ $t(`admin.finance.kpi.${k.key}Def`) }}</small>
          </div>
        </dl>
      </section>

      <section class="c-card" :aria-label="$t('admin.finance.daily.title')">
        <h2 class="c-h2">{{ $t('admin.finance.daily.title') }}</h2>
        <p class="c-mute">{{ $t('admin.finance.daily.note') }}</p>
        <p v-if="!chart.rows.some(r => r.gross > 0)" class="c-mute">{{ $t('admin.finance.daily.empty') }}</p>
        <figure v-else class="fin-fig">
          <svg :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="an-svg" role="img" :aria-label="$t('admin.finance.daily.aria')">
            <line v-for="g in [H * .25, H * .5, H * .75]" :key="g" x1="0" :x2="W" :y1="g" :y2="g" class="an-grid" vector-effect="non-scaling-stroke" />
            <line x1="0" :x2="W" :y1="H - 1" :y2="H - 1" class="an-axis" vector-effect="non-scaling-stroke" />
            <polygon :points="chart.area" class="an-area" />
            <polyline :points="chart.pts" class="an-line" vector-effect="non-scaling-stroke" />
            <rect v-for="(r, i) in chart.rows" :key="r.date" :x="Number(i) * chart.step" y="0" :width="chart.step" :height="H" fill="transparent"><title>{{ r.date }}：{{ money(r.gross) }}</title></rect>
          </svg>
          <div class="an-dates c-mute"><span>{{ chart.rows[0]?.date }}</span><span>{{ chart.rows[chart.rows.length - 1]?.date }}</span></div>
        </figure>
      </section>
    </template>
  </div>
</template>
<style scoped>
.fin-todo{display:block;padding:14px 16px 16px;border-right:1px solid var(--line);border-bottom:1px solid var(--line);color:inherit;text-decoration:none;min-width:0}
.fin-todo:hover,.fin-todo:focus-visible{background:var(--bg-alt)}
.fin-todo dt{font:600 10.5px/1.4 var(--mono);letter-spacing:.12em;text-transform:uppercase;color:var(--mute);margin:0}
.fin-todo dd{margin:8px 0 4px;font-size:clamp(30px,4.4vw,44px);font-weight:900;line-height:1;font-variant-numeric:tabular-nums}
.fin-todo dd.hot{color:var(--red-text)}
.fin-cmp,.fin-def{display:block;margin-top:6px;font:500 11px/1.5 var(--mono)}
.fin-def{font-family:inherit;letter-spacing:0}
.fin-fig{margin:0}
.fin-fig .an-svg{height:120px}
</style>
