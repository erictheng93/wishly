<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.wallets.title') })
const { api } = useApi()
const q = ref(''), rows = ref<ReturnType<typeof normWalletRow>[]>([]), done = ref(false)
const err = ref(''), msg = ref(''), status = ref(''), cursor = ref<string | null>(null), busy = ref(false)
async function search(more = false) {
  if (busy.value) return
  busy.value = true
  err.value = msg.value = ''
  try {
    const r = await api('/admin/wallets', { query: { q: q.value.trim() || undefined, status: status.value || undefined, cursor: more ? cursor.value : undefined } })
    const data = r.data.map(normWalletRow)
    rows.value = more ? [...rows.value, ...data] : data; cursor.value = r.next_cursor; done.value = true
  }
  catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}

watch([q, status], () => { cursor.value = null })
onMounted(() => search())

// 發點 / 扣點（二次確認；Idempotency-Key 於開啟確認時產生，失敗重試沿用）
const g = reactive({ points: '' as string | number, reason: '' })
const target = ref<any>(null), gAsk = ref(false), gBusy = ref(false), gErr = ref(''), gKey = ref('')
function ask(r: any) { target.value = r; g.points = ''; g.reason = ''; gErr.value = '' }
function confirmGrant() {
  gErr.value = grantError(g.points, g.reason)
  if (gErr.value) return
  gKey.value = crypto.randomUUID(); gAsk.value = true
}
async function grant() {
  gBusy.value = true; gErr.value = ''
  try {
    const r = await api('/admin/wallets/grants', { method: 'POST', headers: { 'Idempotency-Key': gKey.value }, body: { user_id: target.value.user_id, points: Number(g.points), reason: g.reason.trim() } })
    Object.assign(target.value, { balance: r.wallet.balance, status: r.wallet.status ?? target.value.status, wallet_id: r.wallet.id ?? target.value.wallet_id })
    msg.value = t(Number(g.points) > 0 ? 'admin.wallets.grantDone' : 'admin.wallets.deductDone', { points: fmtPts(Math.abs(Number(g.points))), name: target.value.display_name, balance: fmtPts(r.wallet.balance) })
    gAsk.value = false; target.value = null
  } catch (e: any) {
    gErr.value = e.code === 'INSUFFICIENT_POINTS' ? t('admin.wallets.insufficient', { balance: fmtPts(e.body?.balance) }) : errMsg(e)
    gAsk.value = false
    if (e.code !== 'NETWORK') gKey.value = crypto.randomUUID()
  } finally { gBusy.value = false }
}

// 凍結 / 解凍
const fz = ref<any>(null), fzReason = ref(''), fzBusy = ref(false), fzErr = ref('')
async function setStatus() {
  fzBusy.value = true; fzErr.value = ''
  const to = fz.value.status === 'frozen' ? 'active' : 'frozen'
  try {
    const r = await api(`/admin/wallets/${fz.value.wallet_id}`, { method: 'PATCH', body: { status: to, reason: fzReason.value.trim() } })
    fz.value.status = r.status ?? r.wallet?.status ?? to
    msg.value = t(to === 'frozen' ? 'admin.wallets.frozenDone' : 'admin.wallets.unfrozenDone', { name: fz.value.display_name }); fz.value = null
  } catch (e) { fzErr.value = errMsg(e) } finally { fzBusy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.wallets.title')" back="/admin" />
    <form class="c-row c-mb" @submit.prevent="search()"><input v-model="q" class="c-grow c-input" :placeholder="$t('admin.wallets.placeholder')" :aria-label="$t('admin.wallets.searchLabel')" :disabled="busy"><button class="c-btn primary" :disabled="busy">{{ busy ? $t('admin.wallets.searching') : $t('admin.wallets.search') }}</button></form>
    <label class="c-field c-narrow"><span>{{ $t('admin.wallets.status') }}</span><select v-model="status" :disabled="busy"><option value="">{{ $t('admin.wallets.all') }}</option><option value="active">{{ $t('admin.wallets.active') }}</option><option value="frozen">{{ $t('admin.wallets.frozen') }}</option><option value="none">{{ $t('admin.wallets.none') }}</option></select></label>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="done && !rows.length" class="c-center c-mute">{{ $t('admin.wallets.empty') }}</p>
    <article v-for="r in rows" :key="r.user_id" class="c-card" :class="{ bad: r.status === 'frozen' }">
      <div class="c-row"><strong class="c-grow">{{ r.display_name }}</strong><span v-if="r.status === 'frozen'" class="c-badge frozen">{{ $t('admin.wallets.frozen') }}</span></div>
      <div class="c-mute">{{ r.email }}</div>
      <div>{{ $t('admin.wallets.balance') }} <b>{{ fmtPts(r.balance) }}</b> {{ $t('admin.wallets.pts') }}</div>
      <div class="c-row wrap c-mt">
        <button class="c-btn primary c-grow" @click="ask(r)">{{ $t('admin.wallets.grant') }}</button>
        <button v-if="r.wallet_id" class="c-btn c-grow" :class="{ danger: r.status !== 'frozen' }" @click="fz = r; fzReason = ''; fzErr = ''">{{ r.status === 'frozen' ? $t('admin.wallets.unfreeze') : $t('admin.wallets.freeze') }}</button>
      </div>
    </article>

    <button v-if="cursor" class="c-btn block" :disabled="busy" @click="search(true)">{{ $t('admin.wallets.loadMore') }}</button>
    <CreatorSheet :open="!!target" :title="$t('admin.wallets.grant')" @close="target = null">
      <p>{{ $t('admin.wallets.target') }}<strong>{{ target?.display_name }}</strong>（{{ target?.email }}）　{{ $t('admin.wallets.currentBalance', { balance: fmtPts(target?.balance) }) }}</p>
      <form @submit.prevent="confirmGrant">
        <label class="c-field"><span>{{ $t('admin.wallets.pointsLabel') }}</span><input v-model="g.points" type="number" inputmode="numeric" step="1" required></label>
        <label class="c-field"><span>{{ $t('admin.wallets.reasonLabel') }}</span><input v-model="g.reason" maxlength="200" required></label>
        <p v-if="gErr" class="c-err" role="alert">{{ gErr }}</p>
        <div class="c-row">
          <button type="button" class="c-btn c-grow" @click="target = null">{{ $t('admin.wallets.cancel') }}</button>
          <button class="c-btn primary c-grow">{{ $t('admin.wallets.next') }}</button>
        </div>
      </form>
    </CreatorSheet>
    <CreatorConfirm :open="gAsk" :title="Number(g.points) > 0 ? $t('admin.wallets.confirmGrant') : $t('admin.wallets.confirmDeduct')" :text="$t(Number(g.points) > 0 ? 'admin.wallets.grantText' : 'admin.wallets.deductText', { points: fmtPts(Math.abs(Number(g.points))), name: target?.display_name, reason: g.reason })" :ok="Number(g.points) > 0 ? $t('admin.wallets.okGrant') : $t('admin.wallets.okDeduct')" :danger="Number(g.points) < 0" :busy="gBusy" @close="gAsk = false" @ok="grant" />
    <CreatorConfirm :open="!!fz" :title="fz?.status === 'frozen' ? $t('admin.wallets.unfreezeTitle') : $t('admin.wallets.freezeTitle')" :text="`${fz?.display_name}（${fz?.email}）`" :ok="fz?.status === 'frozen' ? $t('admin.wallets.unfreeze') : $t('admin.wallets.freeze')" :danger="fz?.status !== 'frozen'" :busy="fzBusy" :disabled="!fzReason.trim()" :error="fzErr" @close="fz = null" @ok="setStatus">
      <label class="c-field"><span>{{ $t('admin.wallets.fzReason') }}</span><input v-model="fzReason" maxlength="200"></label>
      <p v-if="fz?.status !== 'frozen'" class="c-mute c-small">{{ $t('admin.wallets.freezeNote') }}</p>
    </CreatorConfirm>
  </main>
</template>
