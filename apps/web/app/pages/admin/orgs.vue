<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 公益組織與勸募許可審核：組織 pending → 核准 / 補件 / 退件；已通過者退件即撤銷（據點下架，二次確認）；許可可核准 / 退件，已核准者退件即撤銷（據點暫停）
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.orgs.title') })
const { api } = useApi()
const STATUSES = ['pending', 'pending_permits', 'needs_changes', 'verified', 'rejected']
// 可標記補件的欄位（後端 FLAGGABLE）；標籤沿用申請表 org.apply.f.*，其餘三項用 admin.orgs.flag.*
const FLAGS = ['legal_name', 'display_name', 'org_type', 'tax_id', 'registration_no', 'authority', 'website', 'intro', 'logo_key', 'rep_name', 'contact_name', 'contact_title', 'contact_phone', 'public_email', 'public_phone', 'documents', 'permits']
const show = (f: string, v: any) => v == null || v === '' ? '—' : f === 'org_type' ? t(`org.apply.type.${v}`, String(v)) : v
const flagLabel = (f: string) => t(['logo_key', 'documents', 'permits'].includes(f) ? `admin.orgs.flag.${f}` : `org.apply.f.${f}`)
const status = ref('pending')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref(''), dErr = ref('') // dErr：展開的詳情內的錯誤（下載、載入詳情），顯示在卡片內而非頁首

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/orgs', { query: { status: status.value || undefined, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = r.next_cursor ?? null
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch(status, () => { openId.value = null; load() })
onMounted(() => load())

// 詳情
const openId = ref<string | null>(null), detail = ref<any>(null), dLoading = ref(false)
async function toggle(o: any) {
  if (openId.value === o.id) { openId.value = null; return }
  openId.value = o.id; detail.value = null; dErr.value = ''; await reloadDetail()
}
async function reloadDetail() {
  dLoading.value = true; dErr.value = ''
  try { detail.value = await api(`/admin/orgs/${openId.value}`) }
  catch (e) { dErr.value = errMsg(e) } finally { dLoading.value = false }
}
async function download(orgId: string, d: any) {
  dErr.value = ''
  try {
    const blob = await api<Blob>(`/orgs/${orgId}/documents/${d.id}`, { responseType: 'blob' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `${d.kind}-${d.id}.${d.content_type === 'application/pdf' ? 'pdf' : d.content_type === 'image/png' ? 'png' : 'jpg'}`
    document.body.appendChild(a); a.click(); a.remove()
    setTimeout(() => URL.revokeObjectURL(a.href), 0)
  } catch (e) { dErr.value = errMsg(e) }
}

// 審核動作：kind=org 或 permit；revoke 為已通過 / 已核准者的退件
type Act = { kind: 'org' | 'permit', id: string, decision: 'approve' | 'needs_changes' | 'reject', revoke: boolean, name: string, step: 1 | 2 }
const act = ref<Act | null>(null)
const note = ref(''), flagged = ref<string[]>([]), busy = ref(false), aerr = ref('')
function open(kind: Act['kind'], target: any, decision: Act['decision']) {
  const revoke = decision === 'reject' && (kind === 'org' ? target.verification_status === 'verified' : target.status === 'approved')
  act.value = { kind, id: target.id, decision, revoke, name: kind === 'org' ? target.display_name : target.permit_no, step: 1 }
  note.value = ''; flagged.value = []; aerr.value = ''
}
const actKey = computed(() => (act.value?.revoke ? 'revoke' : act.value?.decision) ?? '')
const title = computed(() => act.value ? t(`admin.orgs.${act.value.kind}Action.${actKey.value}`) : '')
const text = computed(() => {
  const a = act.value; if (!a) return ''
  if (a.kind === 'permit') return t(`admin.orgs.confirm.permit${a.decision === 'approve' ? 'Approve' : a.revoke ? 'Revoke' : 'Reject'}`, { no: a.name })
  return t(`admin.orgs.confirm.${a.revoke && a.step === 2 ? 'revoke2' : actKey.value}`, { name: a.name })
})
async function submit() {
  const a = act.value!
  if (a.decision === 'reject' && !note.value.trim()) { aerr.value = t('admin.orgs.noteRequired'); return }
  if (a.revoke && a.kind === 'org' && a.step === 1) { a.step = 2; return } // 撤銷組織：二次確認
  if (a.decision === 'needs_changes' && !flagged.value.length) { aerr.value = t('admin.orgs.flagRequired'); return }
  busy.value = true; aerr.value = ''
  try {
    const body: any = { decision: a.decision, note: note.value.trim() || undefined }
    if (a.decision === 'needs_changes') body.flagged_fields = flagged.value
    await api(a.kind === 'org' ? `/admin/orgs/${a.id}/review` : `/admin/permits/${a.id}/review`, { method: 'POST', body })
    act.value = null; msg.value = t('admin.orgs.done')
    await load(); if (openId.value && rows.value.some(r => r.id === openId.value)) await reloadDetail(); else openId.value = null
  } catch (e: any) {
    if (e.code === 'INVALID_STATE') { act.value = null; await load(); err.value = t('admin.orgs.stale'); if (openId.value) await reloadDetail() }
    else aerr.value = errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.orgs.title')" back="/admin" />
    <label class="c-field c-narrow"><span>{{ $t('admin.orgs.statusLabel') }}</span>
      <select v-model="status" :disabled="loading">
        <option value="">{{ $t('admin.orgs.all') }}</option>
        <option v-for="s in STATUSES" :key="s" :value="s">{{ $t(`admin.orgs.status.${s}`) }}</option>
      </select>
    </label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.orgs.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.orgs.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.orgs.empty') }}</p>
    <article v-for="o in rows" :key="o.id" class="c-card" :data-org="o.id">
      <div class="c-row"><strong class="c-grow">{{ o.display_name }}</strong><span v-if="o.pending_permits" class="c-badge pending">{{ $t('admin.orgs.pendingPermits', { n: o.pending_permits }) }}</span><span class="c-badge" :class="o.verification_status">{{ $t(`admin.orgs.status.${o.verification_status}`, o.verification_status) }}</span></div>
      <div class="c-mute">{{ o.legal_name }} · {{ $t('admin.orgs.submittedAt') }} {{ fmtTime(o.submitted_at) }}</div>
      <button class="c-btn c-mt" @click="toggle(o)">{{ openId === o.id ? $t('admin.orgs.close') : $t('admin.orgs.view') }}</button>

      <div v-if="openId === o.id" class="c-mt">
        <p v-if="dLoading && detail?.org.id !== o.id" class="c-mute" role="status">{{ $t('admin.orgs.detailLoading') }}</p>
        <p v-else-if="dErr && detail?.org.id !== o.id" class="c-err" role="alert">{{ dErr }} <button class="c-btn" @click="reloadDetail">{{ $t('admin.orgs.retry') }}</button></p>
        <template v-else-if="detail?.org.id === o.id">
          <p v-if="dErr" class="c-err" role="alert">{{ dErr }}</p>
          <dl class="c-dl ao-dl">
            <template v-for="f in FLAGS.filter(f => f !== 'documents' && f !== 'permits' && f !== 'logo_key')" :key="f">
              <dt>{{ flagLabel(f) }}</dt><dd>{{ show(f, detail.org[f]) }}</dd>
            </template>
            <template v-if="detail.org.review_note"><dt>{{ $t('admin.orgs.reviewNote') }}</dt><dd>{{ detail.org.review_note }}</dd></template>
          </dl>

          <h3>{{ $t('admin.orgs.docs') }}</h3>
          <p v-if="!detail.documents.length" class="c-mute">{{ $t('admin.orgs.noDocs') }}</p>
          <div v-for="d in detail.documents" :key="d.id" class="c-row">
            <span class="c-grow">{{ $t(`admin.orgs.docKind.${d.kind}`) }} · {{ fmtTime(d.created_at) }}</span>
            <button class="c-btn" @click="download(o.id, d)">{{ $t('admin.orgs.download') }}</button>
          </div>

          <h3>{{ $t('admin.orgs.permits') }}</h3>
          <p v-if="!detail.permits.length" class="c-mute">{{ $t('admin.orgs.noPermits') }}</p>
          <div v-for="p in detail.permits" :key="p.id" class="c-card" :data-permit="p.id" :data-status="p.status">
            <div class="c-row"><strong class="c-grow">{{ p.permit_no }}</strong><span class="c-badge" :class="p.status">{{ $t(`admin.orgs.permitStatus.${p.status}`, p.status) }}</span></div>
            <div class="c-mute">{{ $t('admin.orgs.validRange', { from: p.valid_from, to: p.valid_to }) }}</div>
            <button v-if="detail.documents.some((d: any) => d.id === p.document_id)" class="c-btn c-mt" @click="download(o.id, detail.documents.find((d: any) => d.id === p.document_id))">{{ $t('admin.orgs.download') }}</button>
            <div v-if="p.status !== 'rejected'" class="c-row wrap c-mt">
              <button v-if="p.status === 'pending'" class="c-btn c-grow primary" @click="open('permit', p, 'approve')">{{ $t('admin.orgs.permitAction.approve') }}</button>
              <button class="c-btn c-grow danger" @click="open('permit', p, 'reject')">{{ $t(`admin.orgs.permitAction.${p.status === 'approved' ? 'revoke' : 'reject'}`) }}</button>
            </div>
          </div>

          <h3>{{ $t('admin.orgs.members') }}</h3>
          <div v-for="m in detail.members" :key="m.user_id" class="c-mute">{{ m.display_name }} · {{ $t(`org.members.r.${m.role}`, m.role) }}</div>

          <div v-if="['pending', 'verified'].includes(detail.org.verification_status)" class="c-row wrap c-mt">
            <template v-if="detail.org.verification_status === 'pending'">
              <button class="c-btn c-grow primary" @click="open('org', detail.org, 'approve')">{{ $t('admin.orgs.orgAction.approve') }}</button>
              <button class="c-btn c-grow" @click="open('org', detail.org, 'needs_changes')">{{ $t('admin.orgs.orgAction.needs_changes') }}</button>
            </template>
            <button class="c-btn c-grow danger" @click="open('org', detail.org, 'reject')">{{ $t(`admin.orgs.orgAction.${detail.org.verification_status === 'verified' ? 'revoke' : 'reject'}`) }}</button>
          </div>
        </template>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.orgs.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="title" :text="text" :ok="title" :danger="act?.decision === 'reject'" :busy="busy" :disabled="act?.decision === 'reject' && !note.trim()" :error="aerr" @close="act = null" @ok="submit">
      <template v-if="act && !(act.revoke && act.kind === 'org' && act.step === 2)">
        <fieldset v-if="act.decision === 'needs_changes'" class="c-field ao-flags">
          <legend>{{ $t('admin.orgs.flagLabel') }}</legend>
          <label v-for="f in FLAGS" :key="f" class="c-row"><input v-model="flagged" type="checkbox" :value="f" :name="`flag-${f}`"><span>{{ flagLabel(f) }}</span></label>
        </fieldset>
        <label class="c-field"><span>{{ $t(act.decision === 'reject' ? 'admin.orgs.noteRequired' : 'admin.orgs.noteLabel') }}</span><textarea v-model="note" :required="act.decision === 'reject'" maxlength="500" rows="3" /></label>
      </template>
      <p v-else class="ao-note"><strong>{{ $t('admin.orgs.reviewNote') }}</strong><br>{{ note }}</p>
    </CreatorConfirm>
  </main>
</template>
<style scoped>
.ao-note{white-space:pre-wrap;overflow-wrap:anywhere}
/* 補件欄位：桌面兩欄 */
@media (min-width:768px){.ao-flags{display:grid;grid-template-columns:1fr 1fr;column-gap:16px}.ao-flags>legend{grid-column:1/-1}}
/* 手機：詳情標籤在上、值在下，網址與長名稱不被窄欄斷成一字一行 */
@media (max-width:767px){.ao-dl{grid-template-columns:1fr;gap:0}.ao-dl dt{padding-top:10px}.ao-dl dd{padding-bottom:10px;border-bottom:1px dotted var(--line)}}
</style>
