<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 現金退款審核：requested → processing → completed；requested / processing → rejected（理由必填，點數退回原批次）
import { CASH_REFUND_STATUSES, cashRefundActions, cashRefundPatch, listRows, type RefundTo } from '~/utils/walletOps'
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.cashRefunds.title') })
const { api } = useApi()
const status = ref('requested')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/cash-refunds', { query: { status: status.value || undefined, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...listRows(r)] : listRows(r); cursor.value = r.next_cursor ?? null
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch(status, () => load())
onMounted(() => load())

const act = ref<{ row: any, to: RefundTo } | null>(null)
const reason = ref(''), busy = ref(false), aerr = ref('')
const title = computed(() => (act.value ? t(`admin.cashRefunds.action.${act.value.to}`) : ''))
const text = computed(() => act.value ? t(`admin.cashRefunds.confirm.${act.value.to}`, { name: act.value.row.user?.display_name, amount: fmtPts(act.value.row.amount) }) : '')
function open(row: any, to: RefundTo) { act.value = { row, to }; reason.value = ''; aerr.value = '' }
async function submit() {
  const { row, to } = act.value!
  const p = cashRefundPatch(to, reason.value)
  if (p.error) { aerr.value = t(p.error); return }
  busy.value = true; aerr.value = ''
  try {
    const r = await api(`/admin/cash-refunds/${row.id}`, { method: 'PATCH', body: p.body })
    Object.assign(row, { status: r.status ?? to, reject_reason: r.reject_reason ?? row.reject_reason, completed_at: r.completed_at ?? row.completed_at })
    msg.value = t(`admin.cashRefunds.done.${to}`, { name: row.user?.display_name, amount: fmtPts(row.amount) })
    act.value = null
  } catch (e: any) {
    if (e.code === 'INVALID_STATE_TRANSITION') { act.value = null; await load(); err.value = t('admin.cashRefunds.stale') } // 別人已處理：關閉對話框、重讀列表並說明
    else aerr.value = errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.cashRefunds.title')" back="/admin" />
    <label class="c-field c-narrow"><span>{{ $t('admin.cashRefunds.statusLabel') }}</span>
      <select v-model="status" :disabled="loading">
        <option value="">{{ $t('admin.cashRefunds.all') }}</option>
        <option v-for="s in CASH_REFUND_STATUSES" :key="s" :value="s">{{ $t(`admin.cashRefunds.status.${s}`) }}</option>
      </select>
    </label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.cashRefunds.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.cashRefunds.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.cashRefunds.empty') }}</p>
    <article v-for="r in rows" :key="r.id" class="c-card" :data-status="r.status" :class="{ bad: r.status === 'rejected' }">
      <div class="c-row"><strong class="c-grow">{{ r.user?.display_name }}</strong><span class="c-badge" :class="r.status">{{ $t(`admin.cashRefunds.status.${r.status}`, r.status) }}</span></div>
      <div class="c-mute">{{ r.user?.email }}</div>
      <dl class="c-dl">
        <dt>{{ $t('admin.cashRefunds.amount') }}</dt><dd>{{ $t('admin.cashRefunds.pts', { pts: fmtPts(r.amount) }) }}</dd>
        <dt>{{ $t('admin.cashRefunds.createdAt') }}</dt><dd>{{ fmtTime(r.created_at) }}</dd>
        <template v-if="r.completed_at"><dt>{{ $t('admin.cashRefunds.completedAt') }}</dt><dd>{{ fmtTime(r.completed_at) }}</dd></template>
        <template v-if="r.reject_reason"><dt>{{ $t('admin.cashRefunds.rejectReason') }}</dt><dd>{{ r.reject_reason }}</dd></template>
      </dl>
      <div v-if="cashRefundActions(r.status).length" class="c-row wrap c-mt">
        <button v-for="to in cashRefundActions(r.status)" :key="to" class="c-btn c-grow" :class="to === 'rejected' ? 'danger' : 'primary'" @click="open(r, to)">{{ $t(`admin.cashRefunds.action.${to}`) }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.cashRefunds.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="title" :text="text" :ok="title" :danger="act?.to === 'rejected'" :busy="busy" :error="aerr" @close="act = null" @ok="submit">
      <template v-if="act?.to === 'rejected'">
        <label class="c-field"><span>{{ $t('admin.cashRefunds.reasonLabel') }}</span><input v-model="reason" maxlength="200"></label>
        <p class="c-mute c-small">{{ $t('admin.cashRefunds.rejectNote') }}</p>
      </template>
    </CreatorConfirm>
  </main>
</template>
