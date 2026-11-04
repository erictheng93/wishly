<script setup lang="ts">
import { reporterSuspicious } from '~/utils/reporter'
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 檢舉處理：檢閱被檢舉的清單與使用者，駁回、標記已處理，或直接下架／停權。
definePageMeta({ middleware: 'staff' })
const { t, locale } = useI18n()
useHead({ title: () => t('admin.reports.title') })
const dl = computed(() => (locale.value === 'zh-Hant' ? 'zh-TW' : locale.value))
const { api } = useApi()
const route = useRoute(), router = useRouter()
const q = (k: string) => { const v = route.query[k]; return typeof v === 'string' ? v : '' }
// 篩選全放網址 query，分享與重新整理都能還原
const status = ref(q('status') || 'open'), kind = ref(q('kind')), reason = ref(q('reason')), from = ref(q('from')), to = ref(q('to'))
type Tgt = { kind: string; id: string; label: string }
const tgt = ref<Tgt | null>(q('target_kind') && q('target_id') ? { kind: q('target_kind'), id: q('target_id'), label: q('target_label') } : null)
const view = ref<'targets' | 'rows'>(tgt.value || q('view') === 'rows' ? 'rows' : q('view') === 'targets' ? 'targets' : status.value === 'open' ? 'targets' : 'rows')
const shown = ref(view.value) // 已載入資料的形狀；切換檢視時舊資料不可套用新版型
const rows = ref<any[]>([]), cursor = ref<string | number | null>(null), loading = ref(false), err = ref(''), msg = ref('')
let seq = 0 // 切換篩選時，較慢回來的舊請求不可蓋掉新列表
async function load(more = false) {
  const my = ++seq, v = view.value
  loading.value = true; err.value = ''
  const f: Record<string, any> = { status: status.value, kind: kind.value || undefined, reason: reason.value || undefined, from: from.value || undefined, to: to.value || undefined }
  if (v === 'rows' && tgt.value) f[`${tgt.value.kind}_id`] = tgt.value.id
  try {
    const r = await api(v === 'targets' ? '/admin/reports/targets' : '/admin/reports', { query: { ...f, ...(v === 'targets' ? { offset: more ? cursor.value : undefined } : { cursor: more ? cursor.value : undefined }) } })
    if (my !== seq) return
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = (v === 'targets' ? r.next_offset : r.next_cursor) ?? null; shown.value = v
  } catch (e) { if (my === seq) err.value = errMsg(e) } finally { if (my === seq) loading.value = false }
}
watch(status, s => { view.value = tgt.value || s !== 'open' ? 'rows' : 'targets' })
watch([status, kind, reason, from, to, view, tgt], () => {
  const query: Record<string, string> = { status: status.value, view: view.value }
  for (const [k, v] of Object.entries({ kind: kind.value, reason: reason.value, from: from.value, to: to.value, target_kind: tgt.value?.kind, target_id: tgt.value?.id, target_label: tgt.value?.label })) if (v) query[k] = v
  router.replace({ query })
  load()
})
onMounted(() => load())
const targetRows = computed(() => shown.value === 'targets' ? rows.value : [])
const reportRows = computed(() => shown.value === 'rows' ? rows.value : [])
function drill(g: any) { tgt.value = { kind: g.kind, id: g[g.kind].id, label: name(g) }; view.value = 'rows' }
function state(g: any) {
  return g.wishlist ? { text: g.wishlist.moderation_status === 'hidden' ? t('admin.reports.stateHidden') : t('admin.reports.stateOk'), bad: g.wishlist.moderation_status === 'hidden' }
    : g.user ? { text: g.user.suspended ? t('admin.reports.stateSuspended') : t('admin.reports.stateOk'), bad: g.user.suspended }
      : { text: t(`admin.reports.orgStatus.${g.org.verification_status}`, g.org.verification_status), bad: g.org.verification_status !== 'verified' }
}
const ft = (v: string) => new Date(v).toLocaleString(dl.value)

// 待處理數：只算已載入的列，分頁未載完時僅供參考
const openBy = computed(() => status.value === 'open' && shown.value === 'rows'
  ? { wishlist: rows.value.filter(r => r.wishlist).length, user: rows.value.filter(r => r.user).length, org: rows.value.filter(r => r.org).length } : null)

type Act = { r: any; mode: 'dismissed' | 'actioned' | 'hide' | 'suspend' | 'restore' | 'unsuspend' }
const act = ref<Act | null>(null)
const note = ref(''), busy = ref(false), actErr = ref('')
const name = (r: any) => r.wishlist?.title ?? r.user?.display_name ?? r.org?.display_name ?? ''
function ask(r: any, mode: Act['mode']) {
  if (busy.value) return
  act.value = { r, mode }; note.value = ''; actErr.value = ''
}
const needReason = computed(() => !['actioned', 'dismissed'].includes(act.value?.mode ?? ''))
const TITLE: Record<string, string> = { hide: 'hideList', suspend: 'suspendUser', restore: 'restoreList', unsuspend: 'unsuspendUser', actioned: 'markDone', dismissed: 'dismiss' }
const TEXT: Record<string, string> = { hide: 'hideText', suspend: 'suspendText', restore: 'restoreText', unsuspend: 'unsuspendText' }
const DONE: Record<string, string> = { hide: 'doneHide', suspend: 'doneSuspend', restore: 'doneRestore', unsuspend: 'doneUnsuspend' }
const confirmTitle = computed(() => t(`admin.reports.${TITLE[act.value?.mode ?? 'dismissed']}`))
const confirmText = computed(() => { const m = act.value?.mode ?? ''; return TEXT[m] ? t(`admin.reports.${TEXT[m]}`, { name: name(act.value!.r) }) : name(act.value?.r ?? {}) })
async function submit() {
  if (!act.value || busy.value) return
  const { r, mode } = act.value
  busy.value = true; actErr.value = ''
  const text = note.value.trim()
  try {
    if (mode === 'hide') await api(`/admin/wishlists/${r.wishlist.id}/moderation`, { method: 'PATCH', body: { moderation_status: 'hidden', reason: text } })
    else if (mode === 'restore') await api(`/admin/wishlists/${r.wishlist.id}/moderation`, { method: 'PATCH', body: { moderation_status: 'ok', reason: text } })
    else if (mode === 'suspend' || mode === 'unsuspend') await api(`/admin/users/${r.user.id}`, { method: 'PATCH', body: { blocked: mode === 'suspend', reason: text } })
    else await api(`/admin/reports/${r.id}`, { method: 'PATCH', body: { status: mode, note: text || undefined } })
    act.value = null
    msg.value = DONE[mode] ? t(`admin.reports.${DONE[mode]}`) : ''
    await load()
  } catch (e) { if (act.value) actErr.value = errMsg(e); else err.value = errMsg(e) } finally { busy.value = false }
}
const reasonLabel = computed(() => ({ hide: 'hideReason', suspend: 'suspendReason', restore: 'restoreReason', unsuspend: 'unsuspendReason' } as Record<string, string>)[act.value?.mode ?? ''])
const itemTitle = (r: any) => r.item?.title ?? r.snapshot?.item?.title
const SNAP_MAX = 10
// 處置紀錄：展開才載入，以被檢舉使用者（或清單建立者）為鍵
const hist = reactive<Record<string, any>>({})
const histUid = (r: any) => r.user?.id ?? r.wishlist?.owner?.id
async function toggleHist(r: any) {
  const id = histUid(r)
  if (!hist[id]) hist[id] = { open: false, loading: false, data: null, err: '' }
  const h = hist[id]; h.open = !h.open
  if (!h.open || h.data || h.loading) return
  h.loading = true; h.err = ''
  try { h.data = await api(`/admin/users/${id}/moderation-history`) } catch (e) { h.err = errMsg(e) } finally { h.loading = false }
}
const href = (r: any) => `/u/${(r.user ?? r.wishlist.owner).handle || (r.user ?? r.wishlist.owner).id}`
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.reports.title')" back="/admin" />
    <div class="admin-filters">
      <label class="c-field"><span>{{ $t('admin.reports.status') }}</span><select v-model="status"><option value="open">{{ $t('admin.reports.open') }}</option><option value="actioned">{{ $t('admin.reports.actioned') }}</option><option value="dismissed">{{ $t('admin.reports.dismissed') }}</option></select></label>
      <label class="c-field"><span>{{ $t('admin.reports.kindLabel') }}</span><select v-model="kind"><option value="">{{ $t('admin.reports.kinds.all') }}</option><option value="wishlist">{{ $t('admin.reports.kinds.wishlist') }}</option><option value="user">{{ $t('admin.reports.kinds.user') }}</option><option value="org">{{ $t('admin.reports.kinds.org') }}</option></select></label>
      <label class="c-field"><span>{{ $t('admin.reports.reasonFilter') }}</span><select v-model="reason" data-testid="f-reason"><option value="">{{ $t('admin.reports.reasonAll') }}</option><option v-for="v in ['scam', 'inappropriate', 'copyright', 'personal_info', 'other']" :key="v" :value="v">{{ $t(`admin.reports.reason.${v}`) }}</option></select></label>
      <label class="c-field"><span>{{ $t('admin.reports.from') }}</span><input v-model="from" type="date" data-testid="f-from"></label>
      <label class="c-field"><span>{{ $t('admin.reports.to') }}</span><input v-model="to" type="date" data-testid="f-to"></label>
    </div>
    <div class="c-row wrap c-mt">
      <span class="c-mute">{{ $t('admin.reports.viewLabel') }}</span>
      <button class="c-btn" :class="{ primary: view === 'targets' }" :aria-pressed="view === 'targets'" data-testid="view-targets" @click="tgt = null; view = 'targets'">{{ $t('admin.reports.viewTargets') }}</button>
      <button class="c-btn" :class="{ primary: view === 'rows' }" :aria-pressed="view === 'rows'" data-testid="view-rows" @click="view = 'rows'">{{ $t('admin.reports.viewRows') }}</button>
      <span v-if="tgt" class="c-badge" data-testid="target-chip">{{ $t('admin.reports.filterChip', { name: tgt.label }) }} <button class="g-link" :aria-label="$t('admin.reports.clearFilter')" data-testid="chip-clear" @click="tgt = null">×</button></span>
    </div>
    <dl v-if="openBy" class="k-kpis c-mt" data-testid="report-kpis">
      <div><dt>{{ $t('admin.reports.kinds.wishlist') }}</dt><dd>{{ openBy.wishlist }}{{ cursor ? '+' : '' }}</dd></div>
      <div><dt>{{ $t('admin.reports.kinds.user') }}</dt><dd>{{ openBy.user }}{{ cursor ? '+' : '' }}</dd></div>
      <div><dt>{{ $t('admin.reports.kinds.org') }}</dt><dd>{{ openBy.org }}{{ cursor ? '+' : '' }}</dd></div>
    </dl>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute">{{ $t('admin.reports.loading') }}</p>
    <p v-else-if="!loading && !rows.length" class="c-center c-mute">{{ status === 'open' ? $t('admin.reports.emptyOpen') : $t('admin.reports.empty') }}</p>
    <article v-for="g in targetRows" :key="g.kind + (g[g.kind]?.id)" class="c-card c-mt12" data-testid="target">
      <div class="c-row">
        <strong class="c-grow">{{ $t(`admin.reports.kinds.${g.kind}`) }}：<a :href="g.org ? `/o/${g.org.slug}` : g.wishlist ? `/s/${g.wishlist.slug}` : `/u/${g.user.handle || g.user.id}`" target="_blank" rel="noopener">{{ name(g) }}</a></strong>
        <span class="c-badge" :class="{ hidden: state(g).bad }">{{ state(g).text }}</span>
      </div>
      <div class="c-mute">{{ $t('admin.reports.reasonsLabel') }}{{ g.reasons.map((x: string) => $t(`admin.reports.reason.${x}`, x)).join('、') }}</div>
      <div class="c-mute">{{ $t('admin.reports.firstLatest', { first: ft(g.first_at), latest: ft(g.latest_at) }) }}</div>
      <button class="c-btn c-mt" data-testid="drill" @click="drill(g)">{{ $t('admin.reports.viewReports', { n: g.count }) }}</button>
    </article>
    <article v-for="r in reportRows" :key="r.id" class="c-card c-mt12">
      <div class="c-row"><strong class="c-grow">{{ $t(`admin.reports.reason.${r.reason}`, r.reason) }}</strong><CreatorStatusBadge :v="r.status" /></div>
      <div v-if="r.wishlist">
        {{ $t('admin.reports.kinds.wishlist') }}：<a :href="`/s/${r.wishlist.slug}`" target="_blank" rel="noopener">{{ r.wishlist.title }}</a>
        <span class="c-badge" :class="{ hidden: r.wishlist.moderation_status === 'hidden' }">{{ r.wishlist.moderation_status === 'hidden' ? $t('admin.reports.stateHidden') : $t('admin.reports.stateOk') }}</span>
        <div class="c-mute">{{ $t('admin.reports.owner') }}<a :href="href(r)" target="_blank" rel="noopener">{{ r.wishlist.owner.display_name }}</a></div>
      </div>
      <div v-else-if="r.user">
        {{ $t('admin.reports.kinds.user') }}：<a :href="href(r)" target="_blank" rel="noopener">{{ r.user.display_name }}</a>
        <span class="c-badge" :class="{ hidden: r.user.suspended }">{{ r.user.suspended ? $t('admin.reports.stateSuspended') : $t('admin.reports.stateOk') }}</span>
      </div>
      <div v-else-if="r.org">
        {{ $t('admin.reports.orgLabel') }}：<a :href="`/o/${r.org.slug}`" target="_blank" rel="noopener">{{ r.org.display_name }}</a>
        <span class="c-badge" :class="{ hidden: r.org.verification_status !== 'verified' }">{{ $t(`admin.reports.orgStatus.${r.org.verification_status}`, r.org.verification_status) }}</span>
        <span v-if="r.org.live_sites" class="c-mute" data-testid="org-live">{{ $t('admin.reports.liveSites', { n: r.org.live_sites }) }}</span>
      </div>
      <div v-if="r.detail" class="c-mute report-note">{{ r.detail }}</div>
      <p v-if="r.resolution_note" class="report-note"><strong>{{ $t('admin.reports.note') }}</strong><br>{{ r.resolution_note }}</p>
      <div class="c-mute">{{ $t('admin.reports.meta', { time: new Date(r.created_at).toLocaleString(dl), reporter: $t(`admin.reports.reporterTypes.${r.reporter}`, r.reporter) }) }}<template v-if="r.item_id || r.item">・<template v-if="itemTitle(r)">{{ $t('admin.reports.itemLabel', { title: itemTitle(r) }) }}<span v-if="r.item?.deleted">{{ $t('admin.reports.itemDeleted') }}</span></template><template v-else>{{ $t('admin.reports.onItem').replace(/^・/, '') }}</template></template></div>
      <div v-if="r.reporter_user" class="c-mute" data-testid="reporter-line">{{ $t('admin.reports.reporterLabel') }}<a :href="`/u/${r.reporter_user.handle || r.reporter_user.id}`" target="_blank" rel="noopener">{{ r.reporter_user.display_name }}</a>
        <template v-if="r.reporter_stats">・{{ $t('admin.reports.reporterStats', r.reporter_stats) }}<span v-if="reporterSuspicious(r.reporter_stats)" class="c-badge hidden" data-testid="reporter-flag">{{ $t('admin.reports.reporterFlag') }}</span></template>
      </div>
      <details class="report-snap" data-testid="snapshot">
        <summary>{{ $t('admin.reports.snapshot') }}</summary>
        <p v-if="!r.snapshot" class="c-mute">{{ $t('admin.reports.snapshotNone') }}</p>
        <template v-else>
          <p class="c-mute">{{ $t('admin.reports.snapshotNote') }}</p>
          <div v-if="r.org" class="c-row" data-testid="org-snapshot">
            <img v-if="r.snapshot.logo_url" :src="r.snapshot.logo_url" alt="" class="snap-img" loading="lazy">
            <div><strong>{{ r.snapshot.display_name }}</strong><span v-if="r.snapshot.slug" class="c-mute"> /o/{{ r.snapshot.slug }}</span><div v-if="r.snapshot.description" class="report-note">{{ r.snapshot.description }}</div></div>
          </div>
          <div v-else-if="r.user || 'display_name' in r.snapshot" class="c-row">
            <img v-if="r.snapshot.avatar_url" :src="r.snapshot.avatar_url" alt="" class="snap-img" loading="lazy">
            <div><strong>{{ r.snapshot.display_name }}</strong><span v-if="r.snapshot.handle" class="c-mute"> @{{ r.snapshot.handle }}</span><div v-if="r.snapshot.bio" class="report-note">{{ r.snapshot.bio }}</div></div>
          </div>
          <template v-else>
            <img v-if="r.snapshot.cover_image_url" :src="r.snapshot.cover_image_url" :alt="$t('admin.reports.snapshotCover')" class="snap-cover" loading="lazy">
            <strong>{{ r.snapshot.title }}</strong>
            <div v-if="r.snapshot.description" class="report-note">{{ r.snapshot.description }}</div>
            <div v-if="r.snapshot.item" class="snap-item hit" data-testid="snapshot-item">
              <strong>{{ $t('admin.reports.snapshotReported') }}：{{ r.snapshot.item.title }}</strong>
              <div v-if="r.snapshot.item.description" class="report-note">{{ r.snapshot.item.description }}</div>
              <img v-if="r.snapshot.item.image_url" :src="r.snapshot.item.image_url" alt="" class="snap-img" loading="lazy">
            </div>
            <template v-if="r.snapshot.items?.length">
              <div class="c-mute">{{ $t('admin.reports.snapshotItems', { n: r.snapshot.items.length }) }}</div>
              <ul class="snap-list">
                <li v-for="it in r.snapshot.items.slice(0, SNAP_MAX)" :key="it.id" :class="{ hit: it.id === r.snapshot.item?.id }">{{ it.title }}</li>
              </ul>
              <div v-if="r.snapshot.items.length > SNAP_MAX" class="c-mute">{{ $t('admin.reports.snapshotMore', { n: r.snapshot.items.length - SNAP_MAX }) }}</div>
            </template>
          </template>
        </template>
      </details>
      <div v-if="histUid(r)" class="c-mt">
        <button class="c-btn" :aria-expanded="!!hist[histUid(r)]?.open" data-testid="hist-toggle" @click="toggleHist(r)">{{ r.user ? $t('admin.reports.history') : $t('admin.reports.historyOwner') }}</button>
        <div v-if="hist[histUid(r)]?.open" class="report-snap" data-testid="hist">
          <p v-if="hist[histUid(r)].loading" class="c-mute">{{ $t('admin.reports.loading') }}</p>
          <p v-else-if="hist[histUid(r)].err" class="c-err" role="alert">{{ hist[histUid(r)].err }}</p>
          <template v-else-if="hist[histUid(r)].data">
            <div data-testid="hist-counts">{{ $t('admin.reports.historyCounts', hist[histUid(r)].data.counts) }}</div>
            <strong>{{ $t('admin.reports.historyRecent') }}</strong>
            <p v-if="!hist[histUid(r)].data.recent.length" class="c-mute">{{ $t('admin.reports.historyNone') }}</p>
            <ul class="snap-list">
              <li v-for="x in hist[histUid(r)].data.recent" :key="x.id">
                <a :href="x.wishlist ? `/s/${x.wishlist.slug}` : `/u/${histUid(r)}`" target="_blank" rel="noopener">{{ x.wishlist?.title ?? $t('admin.reports.historyUser') }}</a>・{{ $t(`admin.reports.reason.${x.reason}`, x.reason) }}・{{ $t(`admin.reports.${x.status}`, x.status) }}・{{ ft(x.created_at) }}
              </li>
            </ul>
            <strong>{{ $t('admin.reports.historySusp') }}</strong>
            <p v-if="!hist[histUid(r)].data.suspensions.length" class="c-mute">{{ $t('admin.reports.historyNone') }}</p>
            <ul class="snap-list">
              <li v-for="x in hist[histUid(r)].data.suspensions" :key="x.at">{{ ft(x.at) }}・{{ x.blocked ? $t('admin.reports.suspendedMark') : $t('admin.reports.unsuspendUser') }}・{{ x.reason }}・{{ $t('admin.reports.historyStaff', { name: x.staff?.display_name }) }}</li>
            </ul>
          </template>
        </div>
      </div>
      <div v-if="r.status === 'open' && r.target_open_count > 1" class="c-mute" data-testid="more-open">{{ $t('admin.reports.moreOpen', { n: r.target_open_count }) }}</div>
      <div v-if="r.status === 'open'" class="c-row wrap c-mt">
        <button v-if="r.wishlist && r.wishlist.moderation_status !== 'hidden'" class="c-btn danger c-grow" @click="ask(r, 'hide')">{{ $t('admin.reports.hideList') }}</button>
        <button v-if="r.user && !r.user.suspended" class="c-btn danger c-grow" @click="ask(r, 'suspend')">{{ $t('admin.reports.suspendUser') }}</button>
        <!-- 撤銷組織影響大（據點下架、30 天才能重新申請），只在組織審核頁做：那裡有二次確認與影響說明；後端撤銷時會一併結案這些檢舉 -->
        <NuxtLink v-if="r.org" class="c-btn c-grow" :to="`/admin/orgs?status=${r.org.verification_status}&open=${r.org.id}`">{{ $t('admin.reports.goOrgReview') }}</NuxtLink>
        <button class="c-btn c-grow" @click="ask(r, 'actioned')">{{ $t('admin.reports.markDone') }}</button>
        <button class="c-btn c-grow" @click="ask(r, 'dismissed')">{{ $t('admin.reports.dismiss') }}</button>
      </div>
      <div v-if="(r.wishlist && r.wishlist.moderation_status !== 'ok') || r.user?.suspended" class="c-row wrap c-mt">
        <button v-if="r.wishlist && r.wishlist.moderation_status !== 'ok'" class="c-btn c-grow" @click="ask(r, 'restore')">{{ $t('admin.reports.restoreList') }}</button>
        <button v-if="r.user?.suspended" class="c-btn c-grow" @click="ask(r, 'unsuspend')">{{ $t('admin.reports.unsuspendUser') }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.reports.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="confirmTitle" :text="confirmText" :ok="confirmTitle" :danger="needReason" :busy="busy" :disabled="needReason && !note.trim()" :error="actErr" @close="act = null" @ok="submit">
      <label v-if="needReason" class="c-field"><span>{{ $t(`admin.reports.${reasonLabel}`) }}</span><textarea v-model="note" :disabled="busy" maxlength="200" rows="3" :placeholder="$t('admin.reports.noPii')" /></label>
      <label v-else class="c-field"><span>{{ $t('admin.reports.noteLabel') }}</span><textarea v-model="note" :disabled="busy" maxlength="1000" rows="3" :placeholder="$t('admin.reports.noPii')" /></label>
    </CreatorConfirm>
  </main>
</template>

<style scoped>
.report-snap{margin-top:8px;border:1px solid var(--ink,#111);padding:6px 10px}
.report-snap summary{cursor:pointer}
.snap-img{width:48px;height:48px;object-fit:cover}
.snap-cover{max-width:100%;max-height:120px;object-fit:cover;display:block}
.snap-list{margin:4px 0;padding-left:18px}
.hit{background:rgba(200,16,46,.08)}
.report-note{white-space:pre-wrap;overflow-wrap:anywhere}
.admin-filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px}
</style>
