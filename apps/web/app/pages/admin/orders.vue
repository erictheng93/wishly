<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.orders.title') })
const { api } = useApi()
const status = ref('pending'), fulfillment = ref('')
const rows = ref<any[]>([]), cursor = ref<string | null>(null), loading = ref(false)
const err = ref(''), msg = ref('')
const ACTION = computed<Record<string, string>>(() => ({ placed: t('admin.orders.action.placed'), shipped: t('admin.orders.action.shipped'), delivered: t('admin.orders.action.delivered'), failed: t('admin.orders.action.failed'), cancelled: t('admin.orders.action.cancelled') }))
const actLabel = computed(() => (act.value ? ACTION.value[act.value.to] ?? '' : ''))

async function load(more = false) {
  loading.value = true; err.value = ''
  addressGeneration++; addresses.value = {}; addressBusy.value = {}; addressErrors.value = {}
  try {
    const r = await api('/admin/orders', { query: { status: status.value || undefined, fulfillment_type: fulfillment.value || undefined, limit: 20, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
watch([status, fulfillment], () => load())
onMounted(() => load())

// 明文只保留到隱藏／重新載入；載入列表不會要求收件資訊。
const addresses = ref<Record<string, any>>({}), addressBusy = ref<Record<string, boolean>>({})
const addressErrors = ref<Record<string, string>>({})
let addressGeneration = 0
async function toggleAddress(o: any) {
  if (addresses.value[o.id]) { delete addresses.value[o.id]; return }
  if (addressBusy.value[o.id]) return
  const generation = addressGeneration
  addressBusy.value[o.id] = true; addressErrors.value[o.id] = ''
  try {
    const address = await api(`/admin/orders/${o.id}/shipping-address`)
    if (generation === addressGeneration) addresses.value[o.id] = address
  } catch (e) { if (generation === addressGeneration) addressErrors.value[o.id] = errMsg(e) }
  finally { if (generation === addressGeneration) addressBusy.value[o.id] = false }
}

// 狀態更新：依目前狀態只提供合法動作（orderActions），欄位依目標狀態顯示
const act = ref<{ row: any, to: string } | null>(null)
const form = reactive({ merchant_order_id: '', amount: '' as string | number, tracking_no: '', failure_reason: '' })
const busy = ref(false), aerr = ref('')
function open(row: any, to: string) {
  act.value = { row, to }; aerr.value = ''
  Object.assign(form, { merchant_order_id: row.merchant_order_id ?? '', amount: row.amount ?? row.target_points ?? '', tracking_no: row.tracking_no ?? '', failure_reason: '' })
}
async function submit() {
  const { row, to } = act.value!
  const p = buildOrderPatch(to, form, row.target_points ?? null)
  if (p.error) { aerr.value = p.error; return }
  busy.value = true; aerr.value = ''
  try {
    const r = await api(`/admin/orders/${row.id}`, { method: 'PATCH', body: p.body })
    msg.value = t('admin.orders.updated', { title: row.item?.title, status: label(ORDER_STATUS, r.status) }) + (r.refunded_points ? t('admin.orders.refundedSuffix', { points: fmtPts(r.refunded_points) }) : '')
    act.value = null; await load()
  } catch (e: any) {
    aerr.value = e.code === 'INVALID_STATE_TRANSITION' ? t('admin.orders.stale') : errMsg(e)
    if (e.code === 'INVALID_STATE_TRANSITION') load()
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.orders.title')" back="/admin" />
    <div class="c-filters">
    <label class="c-field"><span>{{ $t('admin.orders.status') }}</span>
      <select v-model="status"><option value="">{{ $t('admin.orders.all') }}</option><option v-for="(t, k) in ORDER_STATUS" :key="k" :value="k">{{ t }}</option></select></label>
    <label class="c-field"><span>{{ $t('admin.orders.fulfillment') }}</span><select v-model="fulfillment"><option value="">{{ $t('admin.orders.all') }}</option><option value="concierge">{{ $t('admin.orders.concierge') }}</option><option value="catalog">{{ $t('admin.orders.catalog') }}</option></select></label>
    </div>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.orders.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.orders.loading') }}</p>
    <p v-else-if="!rows.length && !err" class="c-center c-mute">{{ $t('admin.orders.empty') }}</p>
    <article v-for="o in rows" :key="o.id" class="c-card" :class="{ bad: o.status === 'failed' }">
      <div class="c-row"><strong class="c-grow">{{ o.item?.title }}</strong><span class="c-badge" :class="o.status">{{ label(ORDER_STATUS, o.status) }}</span></div>
      <div class="c-mute">{{ $t('admin.orders.list', { title: o.wishlist?.title }) }}<template v-if="o.item?.product_url">・<a :href="o.item.product_url" target="_blank" rel="noopener nofollow">{{ $t('admin.orders.productLink') }}</a></template></div>
      <dl class="c-dl">
        <dt>{{ $t('admin.orders.targetPoints') }}</dt><dd>{{ fmtPts(o.target_points) }}</dd>
        <dt>{{ $t('admin.orders.amount') }}</dt><dd>{{ o.status === 'pending' ? $t('admin.orders.notReported') : fmtPts(o.amount) }}</dd>
        <template v-if="o.refunded_points"><dt>{{ $t('admin.orders.refundedPoints') }}</dt><dd>{{ $t('admin.orders.refundedNote', { points: fmtPts(o.refunded_points) }) }}</dd></template>
        <template v-if="o.merchant_order_id"><dt>{{ $t('admin.orders.merchantOrderId') }}</dt><dd>{{ o.merchant_order_id }}</dd></template>
        <template v-if="o.tracking_no"><dt>{{ $t('admin.orders.trackingNo') }}</dt><dd>{{ o.tracking_no }}</dd></template>
        <template v-if="o.failure_reason"><dt>{{ $t('admin.orders.failureReason') }}</dt><dd>{{ o.failure_reason }}</dd></template>
        <template v-if="o.operator"><dt>{{ $t('admin.orders.operator') }}</dt><dd>{{ o.operator.display_name }}</dd></template>
        <dt>{{ $t('admin.orders.createdAt') }}</dt><dd>{{ fmtTime(o.created_at) }}</dd>
        <template v-if="o.placed_at"><dt>{{ $t('admin.orders.placedAt') }}</dt><dd>{{ fmtTime(o.placed_at) }}</dd></template>
        <template v-if="o.shipped_at"><dt>{{ $t('admin.orders.shippedAt') }}</dt><dd>{{ fmtTime(o.shipped_at) }}</dd></template>
        <template v-if="o.delivered_at"><dt>{{ $t('admin.orders.deliveredAt') }}</dt><dd>{{ fmtTime(o.delivered_at) }}</dd></template>
      </dl>
      <div v-if="o.shipping_address" class="c-sub">
        <strong class="c-small">{{ $t('admin.orders.shippingTitle') }}</strong>
        <button class="c-btn small" :disabled="addressBusy[o.id]" :aria-expanded="!!addresses[o.id]" @click="toggleAddress(o)">{{ $t(addresses[o.id] ? 'admin.orders.hideAddress' : 'admin.orders.showAddress') }}</button>
        <p v-if="addressErrors[o.id]" class="c-err" role="alert">{{ addressErrors[o.id] }}</p>
        <dl class="c-dl"><dt>{{ $t('admin.orders.recipient') }}</dt><dd>{{ (addresses[o.id] || o.shipping_address).recipient_name }}</dd><dt>{{ $t('admin.orders.phone') }}</dt><dd>{{ (addresses[o.id] || o.shipping_address).phone }}</dd><dt>{{ $t('admin.orders.address') }}</dt><dd>{{ (addresses[o.id] || o.shipping_address).address }}</dd></dl>
      </div>
      <div v-if="orderActions(o.status).length" class="c-row wrap c-mt">
        <button v-for="to in orderActions(o.status)" :key="to" class="c-btn c-grow" :class="to === 'failed' || to === 'cancelled' ? 'danger' : 'primary'" @click="open(o, to)">{{ ACTION[to] }}</button>
      </div>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.orders.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="actLabel" :text="act?.row.item?.title ?? ''" :ok="actLabel" :danger="act?.to === 'failed' || act?.to === 'cancelled'" :busy="busy" :error="aerr" @close="act = null" @ok="submit">
      <template v-if="act?.to === 'placed'">
        <label class="c-field"><span>{{ $t('admin.orders.merchantIdLabel') }}</span><input v-model="form.merchant_order_id" maxlength="100"></label>
        <label class="c-field"><span>{{ $t('admin.orders.amountLabel', { target: fmtPts(act.row.target_points) }) }}</span><input v-model="form.amount" type="number" inputmode="numeric" min="1" :max="act.row.target_points"></label>
        <p class="c-mute c-small">{{ $t('admin.orders.diffNote') }}</p>
      </template>
      <label v-if="act?.to === 'shipped'" class="c-field"><span>{{ $t('admin.orders.trackingLabel') }}</span><input v-model="form.tracking_no" maxlength="100"></label>
      <template v-if="act?.to === 'failed' || act?.to === 'cancelled'">
        <label class="c-field"><span>{{ $t('admin.orders.reasonLabel') }}</span><input v-model="form.failure_reason" maxlength="200"></label>
        <p class="c-mute c-small">{{ $t('admin.orders.failNote') }}</p>
      </template>
      <p v-if="act?.to === 'delivered'" class="c-mute">{{ $t('admin.orders.deliveredNote') }}</p>
    </CreatorConfirm>
  </main>
</template>
