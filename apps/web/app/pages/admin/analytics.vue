<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
useHead({ title: '數據分析' })
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
    ['使用者', d.totals.users], ['訪客', d.totals.guests], ['清單', d.totals.wishlists], ['好友關係', d.totals.friendships],
    ['近 1 天活躍', d.active.d1], ['近 7 天活躍', d.active.d7], ['近 30 天活躍', d.active.d30],
  ] as [string, number][] : []
})
const METRICS = [['signups', '註冊'], ['wishlists', '新清單'], ['claims', '捐助'], ['new_guests', '新訪客']] as const
const W = 200, H = 48
// 折線座標；單日區間（理論上不會）避免除以 0
function chart(key: string) {
  const rows: any[] = data.value?.daily ?? []
  const max = Math.max(1, ...rows.map(r => r[key]))
  const x = (i: number) => rows.length > 1 ? i / (rows.length - 1) * W : W / 2
  const y = (v: number) => H - 2 - v / max * (H - 6)
  const pts = rows.map((r, i) => `${x(i).toFixed(1)},${y(r[key]).toFixed(1)}`).join(' ')
  return { pts, area: rows.length ? `0,${H} ${pts} ${W},${H}` : '', max, sum: rows.reduce((s, r) => s + r[key], 0), step: W / Math.max(1, rows.length) }
}
const charts = computed(() => METRICS.map(([k, name]) => ({ k, name, ...chart(k) })))
const funnel = computed(() => funnelSteps(data.value?.funnel))
const groups = computed(() => {
  const d = data.value
  if (!d) return []
  return [
    ['捐助狀態', barRows(d.claims.by_status, L.claimStatus)], ['捐助者類型', barRows(d.claims.by_actor, L.actor)],
    ['檢舉原因', barRows(d.trust.reports_by_reason, L.reason)],
    ['登入來源', barRows(d.distribution.provider, L.provider)], ['語系', barRows(d.distribution.locale)],
    ['清單類型', barRows(d.distribution.wishlist_type, L.wishlistType)], ['清單可見度', barRows(d.distribution.wishlist_visibility, L.wishlistVisibility)],
    ['生日可見度', barRows(d.distribution.birthday_visibility, L.shareLevel)],
  ] as [string, BarRow[]][]
})
const months = computed(() => barRows(data.value?.birthday_month, Object.fromEntries(Array.from({ length: 12 }, (_, i) => [String(i + 1).padStart(2, '0'), `${i + 1}`])), false))
const profile = computed(() => {
  const p = data.value?.profile
  return p ? [['有自我介紹', p.with_bio], ['有生日', p.with_birthday], ['有頭像', p.with_avatar]] as [string, number][] : []
})
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader title="數據分析" back="/admin" />
    <div class="c-row wrap c-mb">
      <div class="c-row c-grow" role="group" aria-label="統計區間">
        <button v-for="n in RANGES" :key="n" class="c-btn c-grow" :class="{ on: days === n }" :aria-pressed="days === n" :disabled="busy" @click="days = n">{{ n }} 天</button>
      </div>
      <button class="c-btn" :disabled="busy" @click="load">{{ busy ? '更新中…' : '重新整理' }}</button>
    </div>
    <p class="c-mute">人數少於 5 的分組以 &lt;5 顯示，避免反推個人。日期以台北時區計算；排除 staff 與已刪除帳號。</p>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="!data && busy" class="c-mute">載入中…</p>

    <template v-if="data">
      <p class="c-mute">區間：{{ data.range.from }} ～ {{ data.range.to }}（{{ data.range.days }} 天）</p>
      <section class="c-card" aria-label="總覽">
        <dl class="an-tiles">
          <div v-for="t in TILES" :key="t[0]"><dt>{{ t[0] }}</dt><dd>{{ fmtCount(t[1]) }}</dd></div>
        </dl>
      </section>

      <section class="c-card" aria-label="每日趨勢">
        <h2 class="c-h2">每日趨勢</h2>
        <div class="an-multi">
          <figure v-for="c in charts" :key="c.k" class="an-fig">
            <figcaption><strong>{{ c.name }}</strong><span class="c-mute">合計 {{ fmtCount(c.sum) }}・單日最高 {{ c.max === 1 && !c.sum ? 0 : c.max }}</span></figcaption>
            <svg :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" class="an-svg" role="img" :aria-label="`${c.name}每日趨勢，合計 ${c.sum}`">
              <line x1="0" :x2="W" :y1="H - 1" :y2="H - 1" class="an-axis" />
              <polygon :points="c.area" class="an-area" />
              <polyline :points="c.pts" class="an-line" vector-effect="non-scaling-stroke" />
              <rect v-for="(r, i) in data.daily" :key="r.date" :x="Number(i) * c.step" y="0" :width="c.step" :height="H" fill="transparent"><title>{{ r.date }}：{{ r[c.k] }}</title></rect>
            </svg>
            <div class="an-dates c-mute"><span>{{ data.daily[0]?.date }}</span><span>{{ data.daily[data.daily.length - 1]?.date }}</span></div>
          </figure>
        </div>
      </section>

      <section class="c-card" aria-label="轉換漏斗">
        <h2 class="c-h2">轉換漏斗</h2>
        <p class="c-mute">對象：區間內註冊的使用者；百分比為相對上一步。</p>
        <div v-for="s in funnel" :key="s.key" class="an-row">
          <span class="an-l">{{ s.label }}</span>
          <span class="an-track"><span class="an-bar" :style="{ width: s.w + '%' }" /></span>
          <span class="an-v">{{ fmtCount(s.v) }}<small class="c-mute"> {{ s.step }}</small></span>
        </div>
      </section>

      <section class="c-card" aria-label="捐助與信任">
        <h2 class="c-h2">捐助與信任</h2>
        <dl class="an-tiles">
          <div><dt>檢舉（區間）</dt><dd>{{ fmtCount(data.trust.reports) }}</dd></div>
          <div><dt>登入失敗（區間）</dt><dd>{{ fmtCount(data.trust.login_failures) }}</dd></div>
        </dl>
        <template v-for="g in groups.slice(0, 3)" :key="g[0]">
          <h3 class="an-h3">{{ g[0] }}</h3>
          <p v-if="!g[1].length" class="c-mute">沒有資料</p>
          <div v-for="r in g[1]" :key="r.key" class="an-row">
            <span class="an-l">{{ r.label }}</span>
            <span class="an-track"><span class="an-bar" :class="{ sup: r.v == null }" :style="r.v == null ? undefined : { width: r.w + '%' }" /></span>
            <span class="an-v">{{ fmtCount(r.v) }}<small class="c-mute"> {{ r.share }}</small></span>
          </div>
        </template>
      </section>

      <section class="c-card" aria-label="個人資料完整度">
        <h2 class="c-h2">個人資料完整度</h2>
        <p class="c-mute">分母：有效使用者 {{ fmtCount(data.profile.users) }} 人</p>
        <div v-for="p in profile" :key="p[0]" class="an-row">
          <span class="an-l">{{ p[0] }}</span>
          <span class="an-track"><span class="an-bar" :style="{ width: (data.profile.users ? p[1] / data.profile.users * 100 : 0) + '%' }" /></span>
          <span class="an-v">{{ pct(p[1], data.profile.users) }}<small class="c-mute"> {{ fmtCount(p[1]) }}</small></span>
        </div>
        <dl class="an-tiles c-mt"><div><dt>30 天內生日</dt><dd>{{ fmtCount(data.profile.upcoming_birthdays_30d) }}</dd></div></dl>
        <h3 class="an-h3">生日月份</h3>
        <div class="an-cols" role="img" aria-label="生日月份分布">
          <div v-for="m in months" :key="m.key" class="an-col" :title="`${m.label} 月：${fmtCount(m.v)}`">
            <span class="an-cv">{{ fmtCount(m.v) }}</span>
            <span class="an-ctrack"><span class="an-cbar" :class="{ sup: m.v == null }" :style="{ height: (m.v == null ? 0 : m.w) + '%' }" /></span>
            <span class="an-cl c-mute">{{ m.label }}</span>
          </div>
        </div>
      </section>

      <section class="c-card" aria-label="分布">
        <h2 class="c-h2">分布</h2>
        <template v-for="g in groups.slice(3)" :key="g[0]">
          <h3 class="an-h3">{{ g[0] }}</h3>
          <p v-if="!g[1].length" class="c-mute">沒有資料</p>
          <div v-for="r in g[1]" :key="r.key" class="an-row">
            <span class="an-l">{{ r.label }}</span>
            <span class="an-track"><span class="an-bar" :class="{ sup: r.v == null }" :style="r.v == null ? undefined : { width: r.w + '%' }" /></span>
            <span class="an-v">{{ fmtCount(r.v) }}<small class="c-mute"> {{ r.share }}</small></span>
          </div>
        </template>
      </section>

      <details class="c-card"><summary>每日資料表</summary>
        <table class="an-table"><thead><tr><th>日期</th><th v-for="m in METRICS" :key="m[0]">{{ m[1] }}</th></tr></thead>
          <tbody><tr v-for="r in data.daily" :key="r.date"><td>{{ r.date }}</td><td v-for="m in METRICS" :key="m[0]">{{ r[m[0]] }}</td></tr></tbody></table>
      </details>
    </template>
  </main>
</template>
<style scoped>
.an-tiles{display:grid;grid-template-columns:repeat(auto-fit,minmax(120px,1fr));gap:16px;margin:0}
.an-tiles dt{font-size:13px;color:var(--mute)}
.an-tiles dd{margin:4px 0 0;font-size:24px;font-weight:700;font-variant-numeric:tabular-nums}
.an-multi{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:16px}
.an-fig{margin:0}
.an-fig figcaption{display:flex;justify-content:space-between;gap:8px;font-size:13px;flex-wrap:wrap}
.an-svg{width:100%;height:56px;display:block;margin-top:4px}
.an-axis{stroke:var(--line);stroke-width:1}
.an-area{fill:var(--red);opacity:.12}
.an-line{fill:none;stroke:var(--red);stroke-width:2;stroke-linejoin:round}
.an-dates{display:flex;justify-content:space-between;font-size:12px}
.an-h3{font-size:14px;margin:16px 0 4px}
.an-row{display:grid;grid-template-columns:minmax(72px,110px) 1fr minmax(72px,auto);gap:8px;align-items:center;margin:4px 0}
.an-l{font-size:14px;overflow-wrap:anywhere}
.an-v{font-size:14px;font-variant-numeric:tabular-nums;text-align:right}
.an-track{display:block;height:12px;background:var(--fill)}
.an-bar{display:block;height:100%;background:var(--red);min-width:2px}
.an-bar.sup,.an-cbar.sup{width:14px;min-width:14px;background:repeating-linear-gradient(45deg,var(--mute) 0 2px,transparent 2px 5px)}
.an-cols{display:grid;grid-template-columns:repeat(12,1fr);gap:4px;align-items:end}
.an-col{display:flex;flex-direction:column;align-items:center;gap:2px;font-size:12px;font-variant-numeric:tabular-nums}
.an-ctrack{display:flex;align-items:flex-end;width:100%;height:80px;background:var(--fill)}
.an-cbar{display:block;width:100%;background:var(--red)}
.an-cbar.sup{height:14px!important;width:100%}
.an-table{width:100%;border-collapse:collapse;font-size:13px;font-variant-numeric:tabular-nums}
.an-table th,.an-table td{text-align:right;padding:2px 6px;border-bottom:1px solid var(--line)}
.an-table th:first-child,.an-table td:first-child{text-align:left}
</style>
