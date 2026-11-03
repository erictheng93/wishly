<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 檢舉處理：檢閱被檢舉的清單與使用者，駁回、標記已處理，或直接下架／停權。
definePageMeta({ middleware: 'staff' })
const { t, locale } = useI18n()
useHead({ title: () => t('admin.reports.title') })
const dl = computed(() => (locale.value === 'zh-Hant' ? 'zh-TW' : locale.value))
const { api } = useApi()
const status = ref('open'), kind = ref('')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false), err = ref(''), msg = ref('')
let seq = 0 // 切換篩選時，較慢回來的舊請求不可蓋掉新列表
async function load(more = false) {
  const my = ++seq
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/reports', { query: { status: status.value, kind: kind.value || undefined, cursor: more ? cursor.value : undefined } })
    if (my !== seq) return
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = r.next_cursor
  } catch (e) { if (my === seq) err.value = errMsg(e) } finally { if (my === seq) loading.value = false }
}
watch([status, kind], () => load())
onMounted(() => load())

// 待處理數：只算已載入的列，分頁未載完時僅供參考
const openBy = computed(() => status.value === 'open'
  ? { wishlist: rows.value.filter(r => r.wishlist).length, user: rows.value.filter(r => r.user).length } : null)

type Act = { r: any; mode: 'dismissed' | 'actioned' | 'hide' | 'suspend' }
const act = ref<Act | null>(null)
const note = ref(''), busy = ref(false), actErr = ref('')
const name = (r: any) => r.wishlist?.title ?? r.user?.display_name ?? ''
function ask(r: any, mode: Act['mode']) {
  if (busy.value) return
  act.value = { r, mode }; note.value = ''; actErr.value = ''
}
const needReason = computed(() => act.value?.mode === 'hide' || act.value?.mode === 'suspend')
const confirmTitle = computed(() => {
  const m = act.value?.mode
  return m === 'hide' ? t('admin.reports.hideList') : m === 'suspend' ? t('admin.reports.suspendUser') : m === 'actioned' ? t('admin.reports.markDone') : t('admin.reports.dismiss')
})
const confirmText = computed(() => {
  const m = act.value?.mode
  return m === 'hide' ? t('admin.reports.hideText', { name: name(act.value!.r) }) : m === 'suspend' ? t('admin.reports.suspendText', { name: name(act.value!.r) }) : name(act.value?.r ?? {})
})
async function submit() {
  if (!act.value || busy.value) return
  const { r, mode } = act.value
  busy.value = true; actErr.value = ''
  const text = note.value.trim()
  try {
    if (mode === 'hide') await api(`/admin/wishlists/${r.wishlist.id}/moderation`, { method: 'PATCH', body: { moderation_status: 'hidden', reason: text } })
    else if (mode === 'suspend') await api(`/admin/users/${r.user.id}`, { method: 'PATCH', body: { blocked: true, reason: text } })
    else await api(`/admin/reports/${r.id}`, { method: 'PATCH', body: { status: mode, note: text || undefined } })
    act.value = null
    msg.value = mode === 'hide' ? t('admin.reports.doneHide') : mode === 'suspend' ? t('admin.reports.doneSuspend') : ''
    await load()
  } catch (e) { if (act.value) actErr.value = errMsg(e); else err.value = errMsg(e) } finally { busy.value = false }
}
const href = (r: any) => `/u/${(r.user ?? r.wishlist.owner).handle || (r.user ?? r.wishlist.owner).id}`
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.reports.title')" back="/admin" />
    <div class="admin-filters">
      <label class="c-field"><span>{{ $t('admin.reports.status') }}</span><select v-model="status"><option value="open">{{ $t('admin.reports.open') }}</option><option value="actioned">{{ $t('admin.reports.actioned') }}</option><option value="dismissed">{{ $t('admin.reports.dismissed') }}</option></select></label>
      <label class="c-field"><span>{{ $t('admin.reports.kindLabel') }}</span><select v-model="kind"><option value="">{{ $t('admin.reports.kinds.all') }}</option><option value="wishlist">{{ $t('admin.reports.kinds.wishlist') }}</option><option value="user">{{ $t('admin.reports.kinds.user') }}</option></select></label>
    </div>
    <dl v-if="openBy" class="k-kpis c-mt" data-testid="report-kpis">
      <div><dt>{{ $t('admin.reports.kinds.wishlist') }}</dt><dd>{{ openBy.wishlist }}{{ cursor ? '+' : '' }}</dd></div>
      <div><dt>{{ $t('admin.reports.kinds.user') }}</dt><dd>{{ openBy.user }}{{ cursor ? '+' : '' }}</dd></div>
    </dl>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute">{{ $t('admin.reports.loading') }}</p>
    <p v-else-if="!loading && !rows.length" class="c-center c-mute">{{ status === 'open' ? $t('admin.reports.emptyOpen') : $t('admin.reports.empty') }}</p>
    <article v-for="r in rows" :key="r.id" class="c-card c-mt12">
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
      <div v-if="r.detail" class="c-mute report-note">{{ r.detail }}</div>
      <p v-if="r.resolution_note" class="report-note"><strong>{{ $t('admin.reports.note') }}</strong><br>{{ r.resolution_note }}</p>
      <div class="c-mute">{{ $t('admin.reports.meta', { time: new Date(r.created_at).toLocaleString(dl), reporter: $t(`admin.reports.reporterTypes.${r.reporter}`, r.reporter) }) }}{{ r.item_id ? $t('admin.reports.onItem') : '' }}</div>
      <div v-if="r.status === 'open' && r.target_open_count > 1" class="c-mute" data-testid="more-open">{{ $t('admin.reports.moreOpen', { n: r.target_open_count }) }}</div>
      <div v-if="r.status === 'open'" class="c-row wrap c-mt">
        <button v-if="r.wishlist && r.wishlist.moderation_status !== 'hidden'" class="c-btn danger c-grow" @click="ask(r, 'hide')">{{ $t('admin.reports.hideList') }}</button>
        <button v-if="r.user && !r.user.suspended" class="c-btn danger c-grow" @click="ask(r, 'suspend')">{{ $t('admin.reports.suspendUser') }}</button>
        <button class="c-btn c-grow" @click="ask(r, 'actioned')">{{ $t('admin.reports.markDone') }}</button>
        <button class="c-btn c-grow" @click="ask(r, 'dismissed')">{{ $t('admin.reports.dismiss') }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.reports.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="confirmTitle" :text="confirmText" :ok="confirmTitle" :danger="needReason" :busy="busy" :disabled="needReason && !note.trim()" :error="actErr" @close="act = null" @ok="submit">
      <label v-if="needReason" class="c-field"><span>{{ act?.mode === 'hide' ? $t('admin.reports.hideReason') : $t('admin.reports.suspendReason') }}</span><textarea v-model="note" :disabled="busy" maxlength="200" rows="3" /></label>
      <label v-else class="c-field"><span>{{ $t('admin.reports.noteLabel') }}</span><textarea v-model="note" :disabled="busy" maxlength="1000" rows="3" /></label>
    </CreatorConfirm>
  </main>
</template>

<style scoped>
.report-note{white-space:pre-wrap;overflow-wrap:anywhere}
.admin-filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(140px,1fr));gap:12px}
</style>
