<script setup lang="ts">
// 儲值單與拒付爭議：paid →（標記爭議）disputed →（勝訴）paid |（敗訴，扣回）charged_back。
// POST /admin/topups/{id}/dispute 需 Idempotency-Key：開啟確認時產生，失敗重試沿用（網路中斷）或換新（後端已回應）。
import { TOPUP_FILTERS, disputeActions, disputeBody, disputeOutcome, listRows, type DisputeAction } from '~/utils/walletOps'
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.topups.title') })
const { api } = useApi()
const status = ref('disputed')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/topups', { query: { status: status.value || undefined, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...listRows(r)] : listRows(r); cursor.value = r.next_cursor ?? null
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch(status, () => load())
onMounted(() => load())

const act = ref<{ row: any, action: DisputeAction } | null>(null)
const note = ref(''), busy = ref(false), aerr = ref(''), key = ref('')
const title = computed(() => (act.value ? t(`admin.topups.action.${act.value.action}`) : ''))
const text = computed(() => act.value ? t(`admin.topups.confirm.${act.value.action}`, { name: act.value.row.user?.display_name, amount: fmtPts(act.value.row.amount) }) : '')
function open(row: any, action: DisputeAction) { act.value = { row, action }; note.value = ''; aerr.value = ''; key.value = crypto.randomUUID() }
async function submit() {
  const { row, action } = act.value!
  busy.value = true; aerr.value = ''
  try {
    const r = await api(`/admin/topups/${row.id}/dispute`, { method: 'POST', headers: { 'Idempotency-Key': key.value }, body: disputeBody(action, note.value) })
    const o = disputeOutcome(action)
    const name = row.user?.display_name, amount = fmtPts(row.amount)
    if (r.applied === false) msg.value = t('admin.topups.notApplied', { name, amount })
    else {
      row.status = r.status ?? o.status
      if (row.wallet && o.walletFrozen) row.wallet.status = 'frozen'
      msg.value = action === 'lost'
        ? t('admin.topups.done.lost', { name, amount, recovered: fmtPts(r.recovered ?? 0), debt: fmtPts(r.debt ?? 0) })
        : t(`admin.topups.done.${action}`, { name, amount })
    }
    act.value = null
  } catch (e: any) {
    if (e.code === 'INVALID_STATE_TRANSITION') { act.value = null; await load(); err.value = t('admin.topups.stale') } // 狀態已變：關閉對話框、重讀列表並說明
    else { aerr.value = errMsg(e); if (e.code !== 'NETWORK') key.value = crypto.randomUUID() }
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.topups.title')" back="/admin" />
    <p class="c-mute c-small">{{ $t('admin.topups.intro') }}</p>
    <label class="c-field c-narrow"><span>{{ $t('admin.topups.statusLabel') }}</span>
      <select v-model="status" :disabled="loading">
        <option v-for="s in TOPUP_FILTERS" :key="s" :value="s">{{ $t(s ? `admin.topups.status.${s}` : 'admin.topups.all') }}</option>
      </select>
    </label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.topups.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.topups.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.topups.empty') }}</p>
    <article v-for="x in rows" :key="x.id" class="c-card" :data-status="x.status" :class="{ bad: x.status === 'disputed' || x.status === 'charged_back' }">
      <div class="c-row wrap">
        <strong class="c-grow">NT$ {{ fmtPts(x.amount) }}　<span class="c-mute">{{ x.method }}</span></strong>
        <span v-if="x.wallet?.status === 'frozen'" class="c-badge frozen">{{ $t('admin.topups.walletFrozen') }}</span>
        <span class="c-badge" :class="x.status">{{ $t(`admin.topups.status.${x.status}`, x.status) }}</span>
      </div>
      <div>{{ x.user?.display_name }}<span class="c-mute">　{{ x.user?.email }}</span></div>
      <dl class="c-dl">
        <dt>{{ $t('admin.topups.provider') }}</dt><dd>{{ x.provider }}<template v-if="x.provider_txn_id">　<code>{{ x.provider_txn_id }}</code></template></dd>
        <dt>{{ $t('admin.topups.createdAt') }}</dt><dd>{{ fmtTime(x.created_at) }}</dd>
        <template v-if="x.paid_at"><dt>{{ $t('admin.topups.paidAt') }}</dt><dd>{{ fmtTime(x.paid_at) }}</dd></template>
      </dl>
      <div v-if="disputeActions(x.status).length" class="c-row wrap c-mt">
        <button v-for="a in disputeActions(x.status)" :key="a" class="c-btn c-grow" :class="a === 'lost' ? 'danger' : 'primary'" @click="open(x, a)">{{ $t(`admin.topups.action.${a}`) }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.topups.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="title" :text="text" :ok="title" :danger="act?.action === 'lost'" :busy="busy" :error="aerr" @close="act = null" @ok="submit">
      <label class="c-field"><span>{{ $t('admin.topups.noteLabel') }}</span><input v-model="note" maxlength="200"></label>
    </CreatorConfirm>
  </main>
</template>
