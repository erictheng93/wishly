<script setup lang="ts">
// 拒付欠款：open → settled（已追回）| written_off（核銷）；note 必填，已結案不可再改
import { DEBT_STATUSES, debtActions, debtPatch, listRows, type DebtTo } from '~/utils/walletOps'
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.walletDebts.title') })
const { api } = useApi()
const status = ref('open')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/wallet-debts', { query: { status: status.value, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...listRows(r)] : listRows(r); cursor.value = r.next_cursor ?? null
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch(status, () => load())
onMounted(() => load())

const act = ref<{ row: any, to: DebtTo } | null>(null)
const note = ref(''), busy = ref(false), aerr = ref('')
const title = computed(() => (act.value ? t(`admin.walletDebts.action.${act.value.to}`) : ''))
const text = computed(() => act.value ? t(`admin.walletDebts.confirm.${act.value.to}`, { name: act.value.row.user?.display_name, amount: fmtPts(act.value.row.amount) }) : '')
function open(row: any, to: DebtTo) { act.value = { row, to }; note.value = ''; aerr.value = '' }
async function submit() {
  const { row, to } = act.value!
  const p = debtPatch(to, note.value)
  if (p.error) { aerr.value = t(p.error); return }
  busy.value = true; aerr.value = ''
  try {
    const r = await api(`/admin/wallet-debts/${row.id}`, { method: 'PATCH', body: p.body })
    Object.assign(row, { status: r.status ?? to, note: r.note ?? p.body!.note, resolved_at: r.resolved_at ?? row.resolved_at })
    msg.value = t(`admin.walletDebts.done.${to}`, { name: row.user?.display_name, amount: fmtPts(row.amount) })
    act.value = null
  } catch (e: any) {
    if (e.code === 'INVALID_STATE_TRANSITION') { act.value = null; await load(); err.value = t('admin.walletDebts.stale') } // 已被結案：關閉對話框、重讀列表並說明
    else aerr.value = errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.walletDebts.title')" back="/admin" />
    <p class="c-mute c-small">{{ $t('admin.walletDebts.intro') }}</p>
    <label class="c-field c-narrow"><span>{{ $t('admin.walletDebts.statusLabel') }}</span>
      <select v-model="status" :disabled="loading">
        <option v-for="s in DEBT_STATUSES" :key="s" :value="s">{{ $t(`admin.walletDebts.status.${s}`) }}</option>
      </select>
    </label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.walletDebts.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.walletDebts.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.walletDebts.empty') }}</p>
    <article v-for="d in rows" :key="d.id" class="c-card" :data-status="d.status">
      <div class="c-row"><strong class="c-grow">{{ d.user?.display_name }}</strong><span class="c-badge" :class="d.status">{{ $t(`admin.walletDebts.status.${d.status}`, d.status) }}</span></div>
      <div class="c-mute">{{ d.user?.email }}</div>
      <dl class="c-dl">
        <dt>{{ $t('admin.walletDebts.amount') }}</dt><dd>{{ $t('admin.walletDebts.pts', { pts: fmtPts(d.amount) }) }}</dd>
        <dt>{{ $t('admin.walletDebts.topup') }}</dt><dd><code>{{ d.topup_id }}</code></dd>
        <dt>{{ $t('admin.walletDebts.createdAt') }}</dt><dd>{{ fmtTime(d.created_at) }}</dd>
        <template v-if="d.resolved_at"><dt>{{ $t('admin.walletDebts.resolvedAt') }}</dt><dd>{{ fmtTime(d.resolved_at) }}</dd></template>
        <template v-if="d.note"><dt>{{ $t('admin.walletDebts.note') }}</dt><dd>{{ d.note }}</dd></template>
      </dl>
      <div v-if="debtActions(d.status).length" class="c-row wrap c-mt">
        <button v-for="to in debtActions(d.status)" :key="to" class="c-btn c-grow" :class="to === 'settled' ? 'primary' : 'danger'" @click="open(d, to)">{{ $t(`admin.walletDebts.action.${to}`) }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.walletDebts.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="title" :text="text" :ok="title" :danger="act?.to === 'written_off'" :busy="busy" :error="aerr" @close="act = null" @ok="submit">
      <label class="c-field"><span>{{ $t('admin.walletDebts.noteLabel') }}</span><input v-model="note" maxlength="200"></label>
    </CreatorConfirm>
  </main>
</template>
