<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.analytics.title') })
const { api } = useApi()
const RANGES = [7, 30, 90, 365]
const days = ref(30)
const data = ref<any>(null), busy = ref(false), err = ref('')
async function load() {
  if (busy.value) return
  busy.value = true; err.value = ''
  try { data.value = await api('/admin/analytics', { query: { days: days.value } }) }
  catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
watch(days, load)
onMounted(load)

const TILES = computed(() => {
  const d = data.value
  return d ? [
    [t('admin.analytics.tile.users'), d.totals.users], [t('admin.analytics.tile.guests'), d.totals.guests], [t('admin.analytics.tile.wishlists'), d.totals.wishlists], [t('admin.analytics.tile.friendships'), d.totals.friendships],
    [t('admin.analytics.tile.d1'), d.active.d1], [t('admin.analytics.tile.d7'), d.active.d7], [t('admin.analytics.tile.d30'), d.active.d30],
  ] as [string, number][] : []
})
const METRICS = computed(() => [['active_users', t('admin.analytics.metric.activeUsers')], ['signups', t('admin.analytics.metric.signups')], ['wishlists', t('admin.analytics.metric.wishlists')], ['claims', t('admin.analytics.metric.claims')], ['new_guests', t('admin.analytics.metric.newGuests')]] as [string, string][])
const W = 200, H = 48
// 折線座標；單日區間（理論上不會）避免除以 0
function chart(key: string) {
  const rows: any[] = data.value?.daily ?? []
  const max = Math.max(1, ...rows.map(r => r[key]))
  const x = (i: number) => rows.length > 1 ? i / (rows.length - 1) * W : W / 2
  const y = (v: number) => H - 2 - v / max * (H - 6)
  const pts = rows.map((r, i) => `${x(i).toFixed(1)},${y(r[key]).toFixed(1)}`).join(' ')
  return { pts, area: rows.length ? `0,${H} ${pts} ${W},${H}` : '', max, sum: rows.reduce((s, r) => s + r[key], 0), avg: mean(rows.map(r => r[key])), step: W / Math.max(1, rows.length) }
}
const charts = computed(() => METRICS.value.map(([k, name]) => ({ k, name, ...chart(k) })))
const funnel = computed(() => funnelSteps(data.value?.funnel))
const retention = computed(() => retentionRows(data.value?.retention))
const groups = computed(() => {
  const d = data.value
  if (!d) return []
  return [
    [t('admin.analytics.group.claimStatus'), barRows(d.claims.by_status, L.claimStatus)], [t('admin.analytics.group.claimActor'), barRows(d.claims.by_actor, L.actor)],
    [t('admin.analytics.group.reportReason'), barRows(d.trust.reports_by_reason, L.reason)],
    [t('admin.analytics.group.provider'), barRows(d.distribution.provider, L.provider)], [t('admin.analytics.group.locale'), barRows(d.distribution.locale)],
    [t('admin.analytics.group.wishlistType'), barRows(d.distribution.wishlist_type, L.wishlistType)], [t('admin.analytics.group.wishlistVisibility'), barRows(d.distribution.wishlist_visibility, L.wishlistVisibility)],
    [t('admin.analytics.group.birthdayVisibility'), barRows(d.distribution.birthday_visibility, L.shareLevel)],
  ] as [string, BarRow[]][]
})
const months = computed(() => barRows(data.value?.birthday_month, Object.fromEntries(Array.from({ length: 12 }, (_, i) => [String(i + 1).padStart(2, '0'), `${i + 1}`])), false)
  // JS 物件會把 "10"–"12" 這類整數鍵排在 "01" 前面，須依鍵重新排序
  .sort((a, b) => a.key.localeCompare(b.key)))
const profile = computed(() => {
  const p = data.value?.profile
  return p ? [[t('admin.analytics.hasBio'), p.with_bio], [t('admin.analytics.hasBirthday'), p.with_birthday], [t('admin.analytics.hasAvatar'), p.with_avatar]] as [string, number][] : []
})
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.analytics.title')" back="/admin" />
    <div class="c-row wrap c-mb an-range">
      <div class="c-row c-grow" role="group" :aria-label="$t('admin.analytics.rangeLabel')">
        <button v-for="n in RANGES" :key="n" class="c-btn c-grow" :class="{ on: days === n }" :aria-pressed="days === n" :disabled="busy" @click="days = n">{{ $t('admin.analytics.days', { n }) }}</button>
      </div>
      <button class="c-btn" :disabled="busy" @click="load">{{ busy ? $t('admin.analytics.refreshing') : $t('admin.analytics.refresh') }}</button>
    </div>
    <p class="c-mute">{{ $t('admin.analytics.privacy') }}</p>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="!data && busy" class="c-mute">{{ $t('admin.analytics.loading') }}</p>

    <template v-if="data">
      <p class="c-mute">{{ $t('admin.analytics.range', { from: data.range.from, to: data.range.to, days: data.range.days }) }}</p>
      <section class="c-card an-kpi-card" :aria-label="$t('admin.analytics.overview')">
        <dl class="an-tiles k-kpis">
          <div v-for="t in TILES" :key="t[0]"><dt>{{ t[0] }}</dt><dd>{{ fmtCount(t[1]) }}</dd></div>
        </dl>
      </section>

      <section class="c-card" :aria-label="$t('admin.analytics.dailyTrend')">
        <h2 class="c-h2">{{ $t('admin.analytics.dailyTrend') }}</h2>
        <div class="an-multi">
          <figure v-for="c in charts" :key="c.k" class="an-fig">
            <figcaption><strong>{{ c.name }}</strong><span class="c-mute">{{ c.k === 'active_users' ? $t('admin.analytics.avg', { n: c.avg }) : $t('admin.analytics.sum', { n: fmtCount(c.sum) }) }}・{{ $t('admin.analytics.peak', { n: c.max === 1 && !c.sum ? 0 : c.max }) }}</span></figcaption>
            <svg :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="an-svg" role="img" :aria-label="$t('admin.analytics.chartAria', { name: c.name, value: c.k === 'active_users' ? $t('admin.analytics.avg', { n: c.avg }) : $t('admin.analytics.sum', { n: c.sum }) })">
              <line v-for="g in [H * .25, H * .5, H * .75]" :key="g" x1="0" :x2="W" :y1="g" :y2="g" class="an-grid" vector-effect="non-scaling-stroke" />
              <line x1="0" :x2="W" :y1="H - 1" :y2="H - 1" class="an-axis" vector-effect="non-scaling-stroke" />
              <polygon :points="c.area" class="an-area" />
              <polyline :points="c.pts" class="an-line" vector-effect="non-scaling-stroke" />
              <rect v-for="(r, i) in data.daily" :key="r.date" :x="Number(i) * c.step" y="0" :width="c.step" :height="H" fill="transparent"><title>{{ r.date }}：{{ r[c.k] }}</title></rect>
            </svg>
            <div class="an-dates c-mute"><span>{{ data.daily[0]?.date }}</span><span>{{ data.daily[data.daily.length - 1]?.date }}</span></div>
          </figure>
        </div>
      </section>

      <section class="c-card" :aria-label="$t('admin.analytics.funnel')">
        <h2 class="c-h2">{{ $t('admin.analytics.funnel') }}</h2>
        <p class="c-mute">{{ $t('admin.analytics.funnelNote') }}</p>
        <div v-for="s in funnel" :key="s.key" class="an-row">
          <span class="an-l">{{ s.label }}</span>
          <span class="an-track"><span class="an-bar" :style="{ width: s.w + '%' }" /></span>
          <span class="an-v">{{ fmtCount(s.v) }}<small class="c-mute"> {{ s.step }}</small></span>
        </div>
      </section>

      <section class="c-card" :aria-label="$t('admin.analytics.retention')">
        <h2 class="c-h2">{{ $t('admin.analytics.retention') }}</h2>
        <p class="c-mute">{{ $t('admin.analytics.retentionNote') }}</p>
        <p v-if="retention.every(x => !x.cohort)" class="c-mute">{{ $t('admin.analytics.retentionEmpty') }}</p>
        <div v-for="x in retention" :key="x.key" class="an-row">
          <span class="an-l">{{ x.label }}</span>
          <span class="an-track"><span class="an-bar" :style="{ width: x.w + '%' }" /></span>
          <span class="an-v">{{ x.cohort ? x.pct : '–' }}<small v-if="x.cohort" class="c-mute"> {{ fmtCount(x.retained) }}/{{ fmtCount(x.cohort) }}</small></span>
        </div>
      </section>

      <section class="c-card" :aria-label="$t('admin.analytics.claimsTrust')">
        <h2 class="c-h2">{{ $t('admin.analytics.claimsTrust') }}</h2>
        <dl class="an-tiles k-kpis">
          <div><dt>{{ $t('admin.analytics.reports') }}</dt><dd>{{ fmtCount(data.trust.reports) }}</dd></div>
          <div><dt>{{ $t('admin.analytics.loginFailures') }}</dt><dd>{{ fmtCount(data.trust.login_failures) }}</dd></div>
        </dl>
        <template v-for="g in groups.slice(0, 3)" :key="g[0]">
          <h3 class="an-h3">{{ g[0] }}</h3>
          <p v-if="!g[1].length" class="c-mute">{{ $t('admin.analytics.noData') }}</p>
          <div v-for="r in g[1]" :key="r.key" class="an-row">
            <span class="an-l">{{ r.label }}</span>
            <span class="an-track"><span class="an-bar" :class="{ sup: r.v == null }" :style="r.v == null ? undefined : { width: r.w + '%' }" /></span>
            <span class="an-v">{{ fmtCount(r.v) }}<small class="c-mute"> {{ r.share }}</small></span>
          </div>
        </template>
      </section>

      <section class="c-card" :aria-label="$t('admin.analytics.profile')">
        <h2 class="c-h2">{{ $t('admin.analytics.profile') }}</h2>
        <p class="c-mute">{{ $t('admin.analytics.denominator', { n: fmtCount(data.profile.users) }) }}</p>
        <div v-for="p in profile" :key="p[0]" class="an-row">
          <span class="an-l">{{ p[0] }}</span>
          <span class="an-track"><span class="an-bar" :style="{ width: (data.profile.users ? p[1] / data.profile.users * 100 : 0) + '%' }" /></span>
          <span class="an-v">{{ pct(p[1], data.profile.users) }}<small class="c-mute"> {{ fmtCount(p[1]) }}</small></span>
        </div>
        <dl class="an-tiles k-kpis c-mt"><div><dt>{{ $t('admin.analytics.upcoming') }}</dt><dd>{{ fmtCount(data.profile.upcoming_birthdays_30d) }}</dd></div></dl>
        <h3 class="an-h3">{{ $t('admin.analytics.birthdayMonth') }}</h3>
        <div class="an-cols" role="img" :aria-label="$t('admin.analytics.monthAria')">
          <div v-for="m in months" :key="m.key" class="an-col" :title="$t('admin.analytics.monthTitle', { month: m.label, n: fmtCount(m.v) })">
            <span class="an-cv">{{ fmtCount(m.v) }}</span>
            <span class="an-ctrack"><span class="an-cbar" :class="{ sup: m.v == null }" :style="{ height: (m.v == null ? 0 : m.w) + '%' }" /></span>
            <span class="an-cl c-mute">{{ m.label }}</span>
          </div>
        </div>
      </section>

      <section class="c-card" :aria-label="$t('admin.analytics.distribution')">
        <h2 class="c-h2">{{ $t('admin.analytics.distribution') }}</h2>
        <template v-for="g in groups.slice(3)" :key="g[0]">
          <h3 class="an-h3">{{ g[0] }}</h3>
          <p v-if="!g[1].length" class="c-mute">{{ $t('admin.analytics.noData') }}</p>
          <div v-for="r in g[1]" :key="r.key" class="an-row">
            <span class="an-l">{{ r.label }}</span>
            <span class="an-track"><span class="an-bar" :class="{ sup: r.v == null }" :style="r.v == null ? undefined : { width: r.w + '%' }" /></span>
            <span class="an-v">{{ fmtCount(r.v) }}<small class="c-mute"> {{ r.share }}</small></span>
          </div>
        </template>
      </section>

      <details class="c-card"><summary>{{ $t('admin.analytics.dailyTable') }}</summary>
        <table class="an-table"><thead><tr><th>{{ $t('admin.analytics.date') }}</th><th v-for="m in METRICS" :key="m[0]">{{ m[1] }}</th></tr></thead>
          <tbody><tr v-for="r in data.daily" :key="r.date"><td>{{ r.date }}</td><td v-for="m in METRICS" :key="m[0]">{{ r[m[0]] }}</td></tr></tbody></table>
      </details>
    </template>
  </main>
</template>
<style scoped>
.an-tiles{margin:0;grid-template-columns:repeat(auto-fit,minmax(112px,1fr))}
.an-kpi-card{padding:0!important;border-top:2px solid var(--ink-line)}
.an-kpi-card .k-kpis{border-top:0;border-left:0}
.an-multi{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:0;border-top:1px solid var(--line);border-left:1px solid var(--line)}
.an-fig{margin:0;padding:14px 16px 12px;border-right:1px solid var(--line);border-bottom:1px solid var(--line);min-width:0}
.an-fig figcaption{display:flex;flex-direction:column;gap:2px;font-size:14px}
.an-fig figcaption .c-mute{font:500 11px/1.5 var(--mono);letter-spacing:.04em}
.an-h3{font:600 11px var(--mono);letter-spacing:.14em;text-transform:uppercase;color:var(--mute);margin:22px 0 6px;padding-bottom:6px;border-bottom:1px solid var(--line)}
.an-bar.sup,.an-cbar.sup{width:14px;min-width:14px;background:repeating-linear-gradient(45deg,var(--mute) 0 2px,transparent 2px 5px)}
.an-cols{display:grid;grid-template-columns:repeat(12,1fr);gap:4px;align-items:end}
.an-col{display:flex;flex-direction:column;align-items:center;gap:4px;font:500 11px var(--mono);font-variant-numeric:tabular-nums}
.an-ctrack{display:flex;align-items:flex-end;width:100%;height:96px;background:var(--hatch),var(--bg-alt)}
.an-cbar{display:block;width:100%;background:var(--red)}
.an-cbar.sup{height:14px!important;width:100%}
.an-table{width:100%;border-collapse:collapse;font-size:13px;font-family:var(--mono);font-variant-numeric:tabular-nums;margin-top:8px}
.an-table th,.an-table td{text-align:right;padding:8px 6px;border-bottom:1px solid var(--line)}
.an-table th:first-child,.an-table td:first-child{text-align:left}
@media (max-width:520px){.an-cols{gap:2px}}
</style>

