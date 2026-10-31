<script setup lang="ts">
// 交易明細：由原 /admin/topups 搬入。篩選初值取自網址（?status=&method=&user_id=），網址變動時同步。
// 拒付爭議：paid →（標記爭議）disputed →（勝訴）paid |（敗訴，扣回）charged_back。
// POST /admin/topups/{id}/dispute 需 Idempotency-Key：開啟確認時產生，失敗重試沿用（網路中斷）或換新（後端已回應）。
import { disputeActions, disputeBody, disputeOutcome, listRows, type DisputeAction } from '~/utils/walletOps'
const STATUSES = ['pending', 'paid', 'disputed', 'charged_back', 'failed', 'expired', 'refunded']
const METHODS = ['card', 'line_pay', 'apple_pay', 'cvs_code', 'cvs_barcode', 'atm']
const { t, te } = useI18n()
const { api } = useApi()
const route = useRoute(), router = useRouter()
const one = (v: unknown) => typeof v === 'string' ? v : ''
const pick = (v: unknown, ok: string[]) => ok.includes(one(v)) ? one(v) : '' // 不認得的值當作全部，避免後端 422

const status = ref(pick(route.query.status, STATUSES))
const method = ref(pick(route.query.method, METHODS))
const userId = ref(one(route.query.user_id))
const qInput = ref(''), q = ref('') // q 為已送出的關鍵字
// 網址是篩選的唯一來源：下拉只改網址，這個 watcher 負責同步 ref 並重新載入
const setQuery = (k: string, v: string) => router.replace({ query: { ...route.query, [k]: v || undefined } })
watch(() => route.query, (r) => { status.value = pick(r.status, STATUSES); method.value = pick(r.method, METHODS); userId.value = one(r.user_id) })
let timer: ReturnType<typeof setTimeout> | undefined
watch(qInput, (v) => { clearTimeout(timer); timer = setTimeout(() => { q.value = v.trim() }, 400) })
onBeforeUnmount(() => clearTimeout(timer))
const submitQ = () => { clearTimeout(timer); q.value = qInput.value.trim() }
const clearUser = () => setQuery('user_id', '')

const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')
let seq = 0
async function load(more = false) {
  const my = ++seq // 篩選快速變動或回應較慢時只採用最後一次
  loading.value = true; err.value = ''
  try {
    const r = await api('/admin/topups', { query: { status: status.value || undefined, method: method.value || undefined, q: q.value || undefined, user_id: userId.value || undefined, limit: 20, cursor: more ? cursor.value : undefined } })
    if (my !== seq) return
    rows.value = more ? [...rows.value, ...listRows(r)] : listRows(r); cursor.value = r.next_cursor ?? null
  } catch (e) {
    if (my === seq) { err.value = errMsg(e); if (!more) { rows.value = []; cursor.value = null } } // 換篩選失敗：不留舊篩選的列與 cursor
  } finally { if (my === seq) loading.value = false }
}
watch([status, method, q, userId], () => load())
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
  <div>
    <p class="c-mute c-small">{{ $t('admin.topups.intro') }}</p>
    <div class="fin-filters">
      <label class="c-field"><span>{{ $t('admin.topups.statusLabel') }}</span>
        <select :value="status" @change="setQuery('status', ($event.target as HTMLSelectElement).value)">
          <option value="">{{ $t('admin.topups.all') }}</option>
          <option v-for="s in STATUSES" :key="s" :value="s">{{ $t(`admin.topups.status.${s}`) }}</option>
        </select>
      </label>
      <label class="c-field"><span>{{ $t('admin.topups.methodLabel') }}</span>
        <select :value="method" @change="setQuery('method', ($event.target as HTMLSelectElement).value)">
          <option value="">{{ $t('admin.topups.all') }}</option>
          <option v-for="m in METHODS" :key="m" :value="m">{{ methodLabel(m, $t, te) }}</option>
        </select>
      </label>
      <form class="fin-search" role="search" @submit.prevent="submitQ">
        <label class="c-field"><span>{{ $t('admin.topups.searchLabel') }}</span>
          <input v-model="qInput" type="search" :placeholder="$t('admin.topups.searchHint')" maxlength="100">
        </label>
      </form>
    </div>
    <p v-if="userId" class="c-row wrap c-small" role="status">
      <span class="c-badge">{{ $t('admin.topups.userFilter', { name: rows[0]?.user?.display_name || userId }) }}</span>
      <button class="c-btn small" @click="clearUser">{{ $t('admin.topups.clearUser') }}</button>
    </p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.topups.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.topups.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.topups.empty') }}</p>
    <article v-for="x in rows" :key="x.id" class="c-card" :data-status="x.status" :class="{ bad: x.status === 'disputed' || x.status === 'charged_back' }">
      <div class="c-row wrap">
        <strong class="c-grow">NT$ {{ fmtPts(x.amount) }}　<span class="c-mute">{{ methodLabel(x.method, $t, te) }}</span></strong>
        <span v-if="x.wallet?.status === 'frozen'" class="c-badge frozen">{{ $t('admin.topups.walletFrozen') }}</span>
        <span class="c-badge" :class="x.status">{{ $t(`admin.topups.status.${x.status}`, x.status) }}</span>
      </div>
      <div>{{ x.user?.display_name }}<span v-if="x.user?.email" class="c-mute">{{ x.user?.display_name ? '　' : '' }}{{ x.user.email }}</span></div>
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
  </div>
</template>
