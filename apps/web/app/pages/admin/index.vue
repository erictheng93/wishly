<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
const { t, locale } = useI18n()
useHead({ title: () => t('admin.index.title') })
const dl = computed(() => (locale.value === 'zh-Hant' ? 'zh-TW' : locale.value))
const { api } = useApi()
const { user: me } = useAuth()
const tab = ref<'reports' | 'lists' | 'users' | 'flags'>('reports')
const err = ref(''), msg = ref('')
const mask = (e: string) => e.replace(/^(.).*(@.*)$/, '$1***$2')

// 檢舉佇列
const rStatus = ref('open')
const reports = ref<any[]>([]), rCursor = ref<string | null>(null), rLoading = ref(false)
const stats = ref<any>(null), statsErr = ref(''), statsBusy = ref(false)
const STAT = computed<Record<string, string>>(() => ({ users: t('admin.index.stat.users'), wishlists: t('admin.index.stat.wishlists'), active_wishlists: t('admin.index.stat.activeWishlists'), hidden_wishlists: t('admin.index.stat.hiddenWishlists'), open_reports: t('admin.index.stat.openReports') }))
async function loadStats() {
  statsBusy.value = true; statsErr.value = ''
  try { stats.value = await api('/admin/stats') }
  catch (e) { statsErr.value = errMsg(e) } finally { statsBusy.value = false }
}
const REASON = computed<Record<string, string>>(() => ({ scam: t('admin.index.reason.scam'), inappropriate: t('admin.index.reason.inappropriate'), copyright: t('admin.index.reason.copyright'), personal_info: t('admin.index.reason.personal_info'), other: t('admin.index.reason.other') }))
async function loadReports(more = false) {
  rLoading.value = true; err.value = ''
  try {
    const r = await api('/admin/reports', { query: { status: rStatus.value, cursor: more ? rCursor.value : undefined } })
    reports.value = more ? [...reports.value, ...r.data] : r.data; rCursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { rLoading.value = false }
}
async function handle(r: any, status: 'actioned' | 'dismissed') {
  err.value = ''
  try { await api(`/admin/reports/${r.id}`, { method: 'PATCH', body: { status } }); await Promise.all([loadReports(), loadStats()]) }
  catch (e) { err.value = errMsg(e) }
}

// 下架 / 恢復
const mod = ref<{ id: string; title: string; hidden: boolean } | null>(null)
const reason = ref(''), modBusy = ref(false), modErr = ref('')
function openMod(w: any) { mod.value = { id: w.id, title: w.title, hidden: w.moderation_status === 'hidden' }; reason.value = ''; modErr.value = '' }
async function doMod() {
  modBusy.value = true; modErr.value = ''
  try {
    const hide = !mod.value!.hidden
    await api(`/admin/wishlists/${mod.value!.id}/moderation`, { method: 'PATCH', body: hide ? { moderation_status: 'hidden', reason: reason.value.trim() } : { moderation_status: 'ok' } })
    mod.value = null; msg.value = hide ? t('admin.index.mod.hidden') : t('admin.index.mod.restored')
    await Promise.all([loadReports(), loadStats(), tab.value === 'lists' ? searchLists() : 0])
  } catch (e) { modErr.value = errMsg(e) } finally { modBusy.value = false }
}

// 清單搜尋與分頁
const lq = ref(''), lists = ref<any[]>([]), lDone = ref(false), lCursor = ref<string | null>(null), lBusy = ref(false)
const lStatus = ref(''), lMod = ref('')
async function searchLists(more = false) {
  if (lBusy.value) return
  lBusy.value = true; err.value = ''
  try {
    const r = await api('/admin/wishlists', { query: { q: lq.value.trim() || undefined, status: lStatus.value || undefined, moderation_status: lMod.value || undefined, cursor: more ? lCursor.value : undefined } })
    lists.value = more ? [...lists.value, ...r.data] : r.data; lCursor.value = r.next_cursor; lDone.value = true
  } catch (e) { err.value = errMsg(e) } finally { lBusy.value = false }
}
watch([lq, lStatus, lMod], () => { lCursor.value = null })
// 使用者查詢與操作
const uq = ref(''), users = ref<any[]>([]), shown = ref<Record<string, boolean>>({}), uDone = ref(false), uCursor = ref<string | null>(null), uBusy = ref(false)
const uStatus = ref(''), uProvider = ref('')
const PROVIDER: Record<string, string> = { email: 'Email', line: 'LINE', google: 'Google' }
async function searchUsers(more = false) {
  if (uBusy.value) return
  uBusy.value = true; err.value = ''
  try {
    const r = await api('/admin/users', { query: { q: uq.value.trim() || undefined, status: uStatus.value || undefined, provider: uProvider.value || undefined, cursor: more ? uCursor.value : undefined } })
    users.value = more ? [...users.value, ...r.data] : r.data; uCursor.value = r.next_cursor; uDone.value = true; shown.value = {}
  } catch (e) { err.value = errMsg(e) } finally { uBusy.value = false }
}
watch([uq, uStatus, uProvider], () => { uCursor.value = null })
const userAct = ref<{ user: any; kind: string; title: string; provider?: string } | null>(null)
const userReason = ref(''), userBusy = ref(false), userErr = ref('')
function askUser(user: any, kind: string, title: string, provider?: string) {
  userAct.value = { user, kind, title, provider }; userReason.value = ''; userErr.value = ''
}
async function doUser() {
  if (userBusy.value || !userAct.value) return
  userBusy.value = true; userErr.value = ''
  const { user, kind, provider, title } = userAct.value
  try {
    const path = `/admin/users/${user.id}`
    const body: any = { reason: userReason.value.trim() }
    if (kind === 'blocked') body.blocked = !user.blocked
    if (kind === 'staff') body.is_staff = !user.is_staff
    await api(kind === 'unlink' ? `${path}/identities/${provider}` : kind === 'recovery' ? `${path}/recovery` : kind === 'mfa' ? `${path}/mfa` : path,
      { method: kind === 'unlink' || kind === 'mfa' ? 'DELETE' : kind === 'recovery' ? 'POST' : 'PATCH', body })
    userAct.value = null; msg.value = kind === 'recovery' ? t('admin.index.userAct.recoverySent') : t('admin.index.userAct.done', { title })
    await Promise.all([searchUsers(), loadStats()])
  } catch (e) { userErr.value = errMsg(e) } finally { userBusy.value = false }
}
// 系統旗標
const readOnly = ref<boolean | null>(null), flagAsk = ref(false), flagBusy = ref(false)
async function setFlag() {
  flagBusy.value = true; err.value = ''
  try { const r = await api('/admin/system-flags/read_only', { method: 'PUT', body: { value: !readOnly.value } }); readOnly.value = r.value; flagAsk.value = false }
  catch (e) { err.value = errMsg(e) } finally { flagBusy.value = false }
}
async function loadFlags() {
  err.value = ''
  try { readOnly.value = !!(await api('/admin/system-flags')).data.find((f: any) => f.key === 'read_only')?.value } catch (e) { readOnly.value = null; err.value = errMsg(e) }
}
watch(rStatus, () => loadReports())
watch(tab, t => { if (t === 'flags') loadFlags() })
onMounted(() => Promise.all([loadReports(), loadStats()]))
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.index.title')" back="/dashboard"><span class="c-badge">STAFF</span></CreatorHeader>
    <p v-if="me && !me.mfa_enabled" class="c-card" role="status">{{ $t('admin.index.mfaHint') }}<NuxtLink to="/admin/mfa">{{ $t('admin.index.mfaSetup') }}</NuxtLink></p>
    <section class="c-card" :aria-label="$t('admin.index.statsLabel')">
      <div class="c-row c-mb"><h2 class="c-h2 c-grow c-m0">{{ $t('admin.index.overview') }}</h2><button class="c-btn small" :disabled="statsBusy" @click="loadStats">{{ statsBusy ? $t('admin.index.refreshing') : $t('admin.index.refreshStats') }}</button></div>
      <p v-if="statsErr" class="c-err" role="alert">{{ statsErr }}</p>
      <dl class="admin-stats">
        <div v-for="(name, key) in STAT" :key="key"><dt>{{ name }}</dt><dd :data-testid="`stat-${key}`">{{ stats?.[key] ?? '–' }}</dd></div>
      </dl>
      <details v-if="stats"><summary>{{ $t('admin.index.detailsSummary') }}</summary>
        <div v-for="(name, key) in { claims_by_status: $t('admin.index.claimsByStatus'), notifications_by_status: $t('admin.index.notificationsByStatus'), audit_last_7d: $t('admin.index.auditLast7d') }" :key="key" class="c-mt">
          <strong>{{ name }}</strong><ul><li v-for="(count, status) in stats[key]" :key="status">{{ $t('admin.index.statusCount', { status, count }) }}</li></ul>
          <p v-if="!Object.keys(stats[key] || {}).length" class="c-mute">{{ $t('admin.index.noRecords') }}</p>
        </div>
      </details>
    </section>
    <div class="c-row wrap c-mb">
      <NuxtLink to="/admin/orders" class="c-btn c-grow">{{ $t('admin.index.nav.orders') }}</NuxtLink>
      <NuxtLink to="/admin/wallets" class="c-btn c-grow">{{ $t('admin.index.nav.wallets') }}</NuxtLink>
      <NuxtLink to="/admin/cash-refunds" class="c-btn c-grow">{{ $t('admin.index.nav.cashRefunds') }}</NuxtLink>
      <NuxtLink to="/admin/wallet-debts" class="c-btn c-grow">{{ $t('admin.index.nav.walletDebts') }}</NuxtLink>
      <NuxtLink to="/admin/topups" class="c-btn c-grow">{{ $t('admin.index.nav.topups') }}</NuxtLink>
      <NuxtLink to="/admin/catalog" class="c-btn c-grow">{{ $t('admin.index.nav.catalog') }}</NuxtLink>
      <NuxtLink to="/admin/orgs" class="c-btn c-grow">{{ $t('admin.index.nav.orgs') }}</NuxtLink>
      <NuxtLink to="/admin/relief-events" class="c-btn c-grow">{{ $t('admin.index.nav.events') }}</NuxtLink>
      <NuxtLink to="/admin/audit" class="c-btn c-grow">{{ $t('admin.index.nav.audit') }}</NuxtLink>
      <NuxtLink to="/admin/analytics" class="c-btn c-grow">{{ $t('admin.index.nav.analytics') }}</NuxtLink>
    </div>
    <nav class="c-tabs" :aria-label="$t('admin.index.tabsLabel')">
      <button v-for="t in [['reports', $t('admin.index.tabs.reports')], ['lists', $t('admin.index.tabs.lists')], ['users', $t('admin.index.tabs.users')], ['flags', $t('admin.index.tabs.flags')]]" :key="t[0]" class="c-btn" :class="{ on: tab === t[0] }" @click="tab = t[0] as any">{{ t[1] }}</button>
    </nav>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>

    <section v-if="tab === 'reports'">
      <label class="c-field"><span>{{ $t('admin.index.reports.status') }}</span><select v-model="rStatus"><option value="open">{{ $t('admin.index.reports.open') }}</option><option value="actioned">{{ $t('admin.index.reports.actioned') }}</option><option value="dismissed">{{ $t('admin.index.reports.dismissed') }}</option></select></label>
      <p v-if="rLoading" class="c-mute">{{ $t('admin.index.loading') }}</p>
      <p v-else-if="!reports.length" class="c-center c-mute">{{ rStatus === 'open' ? $t('admin.index.reports.emptyOpen') : $t('admin.index.reports.empty') }}</p>
      <article v-for="r in reports" :key="r.id" class="c-card">
        <div class="c-row"><strong class="c-grow">{{ REASON[r.reason] || r.reason }}</strong><CreatorStatusBadge :v="r.status" /></div>
        <div>{{ $t('admin.index.reports.list') }}<a :href="`/s/${r.wishlist.slug}`" target="_blank" rel="noopener">{{ r.wishlist.title }}</a>（{{ r.wishlist.slug }}）</div>
        <div v-if="r.detail" class="c-mute">{{ r.detail }}</div>
        <div class="c-mute">{{ $t('admin.index.reports.meta', { time: new Date(r.created_at).toLocaleString(dl), reporter: r.reporter }) }}{{ r.item_id ? $t('admin.index.reports.onItem') : '' }}</div>
        <div v-if="r.status === 'open'" class="c-row wrap c-mt">
          <button class="c-btn danger c-grow" @click="openMod({ id: r.wishlist.id, title: r.wishlist.title, moderation_status: 'ok' })">{{ $t('admin.index.hide') }}</button>
          <button class="c-btn c-grow" @click="handle(r, 'actioned')">{{ $t('admin.index.reports.actioned') }}</button>
          <button class="c-btn c-grow" @click="handle(r, 'dismissed')">{{ $t('admin.index.reports.dismiss') }}</button>
        </div>
      </article>
      <button v-if="rCursor" class="c-btn block" @click="loadReports(true)">{{ $t('admin.index.loadMore') }}</button>
    </section>

    <section v-if="tab === 'lists'">
      <form class="c-row" @submit.prevent="searchLists()"><input v-model="lq" class="c-grow c-input" :placeholder="$t('admin.index.lists.placeholder')" :aria-label="$t('admin.index.lists.searchLabel')" :disabled="lBusy"><button class="c-btn primary" :disabled="lBusy">{{ lBusy ? $t('admin.index.searching') : $t('admin.index.search') }}</button></form>
      <div class="admin-filters c-mt">
        <label class="c-field"><span>{{ $t('admin.index.lists.status') }}</span><select v-model="lStatus" :disabled="lBusy"><option value="">{{ $t('admin.index.all') }}</option><option value="draft">{{ $t('admin.index.lists.draft') }}</option><option value="active">{{ $t('admin.index.lists.active') }}</option><option value="closed">{{ $t('admin.index.lists.closed') }}</option><option value="archived">{{ $t('admin.index.lists.archived') }}</option></select></label>
        <label class="c-field"><span>{{ $t('admin.index.lists.modStatus') }}</span><select v-model="lMod" :disabled="lBusy"><option value="">{{ $t('admin.index.all') }}</option><option value="ok">{{ $t('admin.index.normal') }}</option><option value="hidden">{{ $t('admin.index.hidden') }}</option></select></label>
      </div>
      <p v-if="lDone && !lists.length" class="c-center c-mute">{{ $t('admin.index.lists.empty') }}</p>
      <article v-for="w in lists" :key="w.id" class="c-card c-mt12">
        <div class="c-row"><strong class="c-grow">{{ w.title }}</strong><CreatorStatusBadge :v="w.moderation_status === 'hidden' ? 'hidden' : w.status" /></div>
        <div class="c-mute">{{ $t('admin.index.lists.meta', { slug: w.slug, email: mask(w.owner.email || ''), count: w.open_report_count }) }}</div>
        <div class="c-row c-mt">
          <a :href="`/s/${w.slug}`" target="_blank" rel="noopener" class="c-btn">{{ $t('admin.index.lists.publicPage') }}</a>
          <button class="c-btn c-grow" :class="{ danger: w.moderation_status !== 'hidden' }" @click="openMod(w)">{{ w.moderation_status === 'hidden' ? $t('admin.index.restore') : $t('admin.index.hide') }}</button>
        </div>
      </article>
      <button v-if="lCursor" class="c-btn block" :disabled="lBusy" @click="searchLists(true)">{{ $t('admin.index.lists.loadMore') }}</button>
    </section>

    <section v-if="tab === 'users'">
      <form class="c-row" @submit.prevent="searchUsers()"><input v-model="uq" class="c-grow c-input" :placeholder="$t('admin.index.users.placeholder')" :aria-label="$t('admin.index.users.searchLabel')" :disabled="uBusy"><button class="c-btn primary" :disabled="uBusy">{{ uBusy ? $t('admin.index.searching') : $t('admin.index.search') }}</button></form>
      <div class="admin-filters c-mt">
        <label class="c-field"><span>{{ $t('admin.index.users.accountStatus') }}</span><select v-model="uStatus" :disabled="uBusy"><option value="">{{ $t('admin.index.all') }}</option><option value="active">{{ $t('admin.index.normal') }}</option><option value="blocked">{{ $t('admin.index.users.blocked') }}</option><option value="deleted">{{ $t('admin.index.deleted') }}</option><option value="staff">{{ $t('admin.index.users.staffActive') }}</option></select></label>
        <label class="c-field"><span>{{ $t('admin.index.users.provider') }}</span><select v-model="uProvider" :disabled="uBusy"><option value="">{{ $t('admin.index.all') }}</option><option v-for="(name, key) in PROVIDER" :key="key" :value="key">{{ name }}</option></select></label>
      </div>
      <p v-if="uDone && !users.length" class="c-center c-mute">{{ $t('admin.index.users.empty') }}</p>
      <article v-for="u in users" :key="u.id" class="c-card c-mt12">
        <strong>{{ u.display_name }}</strong> <span v-if="u.is_staff" class="c-badge">STAFF</span> <span v-if="u.blocked" class="c-badge hidden">{{ $t('admin.index.users.badgeBlocked') }}</span> <span v-if="u.deleted" class="c-badge">{{ $t('admin.index.deleted') }}</span>
        <div class="c-mute">{{ shown[u.id] ? u.email : mask(u.email || '') }} <button v-if="u.email" class="c-btn small" @click="shown[u.id] = !shown[u.id]">{{ shown[u.id] ? $t('admin.index.users.hideEmail') : $t('admin.index.users.show') }}</button></div>
        <div class="c-mute admin-id">{{ u.id }}<span v-if="u.handle">・@{{ u.handle }}</span></div>
        <div class="c-mute">{{ $t('admin.index.users.loginMethods') }}{{ u.identities.map((i: any) => PROVIDER[i.provider]).join($t('admin.index.users.sep')) || $t('admin.index.users.none') }}</div>
        <div class="c-row wrap c-mt c-mb" v-if="!u.deleted">
          <button class="c-btn" :class="{ danger: !u.blocked }" @click="askUser(u, 'blocked', u.blocked ? $t('admin.index.users.unblock') : $t('admin.index.users.block'))">{{ u.blocked ? $t('admin.index.users.unblock') : $t('admin.index.users.block') }}</button>
          <button class="c-btn" @click="askUser(u, 'staff', u.is_staff ? $t('admin.index.users.unsetStaff') : $t('admin.index.users.setStaff'))">{{ u.is_staff ? $t('admin.index.users.unsetStaff') : $t('admin.index.users.setStaff') }}</button>
          <button v-for="i in u.identities.filter((i: any) => i.provider !== 'email')" :key="i.provider" class="c-btn" :disabled="u.identities.length <= 1" @click="askUser(u, 'unlink', $t('admin.index.users.unlink', { provider: PROVIDER[i.provider] }), i.provider)">{{ $t('admin.index.users.unlink', { provider: PROVIDER[i.provider] }) }}</button>
          <button class="c-btn" :disabled="!u.email || u.blocked" @click="askUser(u, 'recovery', $t('admin.index.users.recovery'))">{{ $t('admin.index.users.recovery') }}</button>
          <button v-if="u.mfa_enabled" class="c-btn" @click="askUser(u, 'mfa', $t('admin.index.users.resetMfa'))">{{ $t('admin.index.users.resetMfa') }}</button>
        </div>
        <p v-if="!u.email && !u.deleted" class="c-mute">{{ $t('admin.index.users.noEmail') }}</p>
        <div class="c-mute">{{ $t('admin.index.users.meta', { count: u.wishlist_count, date: new Date(u.created_at).toLocaleDateString(dl) }) }}</div>
      </article>
      <button v-if="uCursor" class="c-btn block" :disabled="uBusy" @click="searchUsers(true)">{{ $t('admin.index.users.loadMore') }}</button>
    </section>

    <section v-if="tab === 'flags'" class="c-card">
      <label class="c-switch"><input type="checkbox" :checked="!!readOnly" :disabled="readOnly === null || flagBusy" @click.prevent="flagAsk = true"><span>{{ $t('admin.index.flags.readOnly') }}<br><small class="c-mute">{{ $t('admin.index.flags.hint', { state: readOnly === null ? $t('admin.index.flags.unknown') : readOnly ? $t('admin.index.flags.on') : $t('admin.index.flags.off') }) }}</small></span></label>
      <p class="c-mute">{{ $t('admin.index.flags.auditNote') }}</p>
    </section>

    <CreatorConfirm :open="!!mod" :title="mod?.hidden ? $t('admin.index.restore') : $t('admin.index.mod.hideTitle')" :text="$t('admin.index.mod.text', { title: mod?.title })" :ok="mod?.hidden ? $t('admin.index.restore') : $t('admin.index.mod.hideOk')" :danger="!mod?.hidden" :busy="modBusy" :disabled="!mod?.hidden && !reason.trim()" :error="modErr" @close="mod = null" @ok="doMod">
      <label v-if="mod && !mod.hidden" class="c-field"><span>{{ $t('admin.index.mod.reason') }}</span><input v-model="reason" maxlength="200"></label>
    </CreatorConfirm>
    <CreatorConfirm :open="!!userAct" :title="userAct?.title || ''" :text="$t('admin.index.userAct.text', { name: userAct?.user.display_name })" :ok="$t('admin.index.userAct.ok')" danger :busy="userBusy" :disabled="!userReason.trim()" :error="userErr" @close="!userBusy && (userAct = null)" @ok="doUser">
      <label class="c-field"><span>{{ $t('admin.index.userAct.reason') }}</span><input v-model="userReason" maxlength="200" :disabled="userBusy" required></label>
    </CreatorConfirm>
    <CreatorConfirm :open="flagAsk" :title="$t('admin.index.flags.confirmTitle')" :text="readOnly ? $t('admin.index.flags.confirmOff') : $t('admin.index.flags.confirmOn')" :busy="flagBusy" danger @close="flagAsk = false" @ok="setFlag" />
  </main>
</template>

<style scoped>
.admin-stats{display:grid;grid-template-columns:repeat(auto-fit,minmax(120px,1fr));gap:16px;margin:0 0 16px}
.admin-stats dt{font-size:13px;color:var(--mute)}
.admin-stats dd{margin:4px 0 0;font-size:24px;font-weight:700;font-variant-numeric:tabular-nums}
.admin-filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px}
.admin-id{overflow-wrap:anywhere}
</style>
