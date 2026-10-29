<script setup lang="ts">
// 申請現金退款：點數先扣、營運審核後退款、被拒絕則退回點數。只有儲值購買的點數可退（cash_refundable_balance）。
import { idemKeys } from '~/utils/topup'
import { REFUND_MAX, buildRefundBody, isFrozen, pendingRefunds, refundAmountError, refundError, refundStatusKey } from '~/utils/refund'
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('me.refund.title') })
const { api } = useApi()

const wallet = ref<any>(null), requests = ref<any[]>([])
const loading = ref(true), listErr = ref('')
const amount = ref(''), submitted = ref(false), busy = ref(false)
const err = ref(''), success = ref(''), frozenNow = ref(false)
const idem = idemKeys()

const refundable = computed(() => Number(wallet.value?.cash_refundable_balance) || 0)
const frozen = computed(() => frozenNow.value || isFrozen(wallet.value))
const pending = computed(() => pendingRefunds(requests.value))
const amountKey = computed(() => refundAmountError(amount.value, refundable.value))
const amountNum = computed(() => (amountKey.value ? 0 : Number(amount.value)))
// 金額欄一開始是空的，送出前不吵
const amountErr = computed(() => (submitted.value && amountKey.value ? t(amountKey.value, { max: fmtPts(REFUND_MAX) }) : ''))
watch(amount, () => { err.value = '' })

async function loadWallet() { wallet.value = (await api('/wallet', { query: { limit: 1 } })).wallet }
async function loadRequests() { requests.value = (await api('/wallet/cash-refunds', { query: { limit: 20 } })).requests ?? [] }
async function load() {
  listErr.value = ''
  try { await Promise.all([loadWallet(), loadRequests()]) } catch (e) { listErr.value = errMsg(e) } finally { loading.value = false }
}
onMounted(load)

async function submit() {
  submitted.value = true
  if (busy.value || amountKey.value) return
  busy.value = true; err.value = ''; success.value = ''
  const body = buildRefundBody(amount.value)
  try {
    const r = await api('/wallet/cash-refunds', { method: 'POST', headers: { 'Idempotency-Key': idem.keyFor(body) }, body })
    idem.reset()
    success.value = t('me.refund.success', { pts: fmtPts(r.request.amount), bal: fmtPts(r.wallet?.balance) })
    amount.value = ''; submitted.value = false
    requests.value = [r.request, ...requests.value.filter(x => x.id !== r.request.id)]
    await loadWallet().catch(() => {})
  } catch (e: any) {
    const r = refundError(e, (k, p) => t(k, p), fmtPts)
    err.value = r.msg
    if (r.frozen) frozenNow.value = true
    if (r.reloadWallet) loadWallet().catch(() => {})
  } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('me.refund.title')" back="/me/wallet" />
    <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <p v-if="listErr" class="c-err" role="alert">{{ listErr }} <button class="c-btn" @click="load">{{ $t('me.retry') }}</button></p>
    <div aria-live="polite">
      <p v-if="success" class="c-ok" role="status">{{ success }}</p>
    </div>

    <template v-if="wallet">
      <p v-if="frozen" class="c-err" role="alert" data-testid="frozen">{{ $t('me.refund.err.frozen') }} <SupportLink /></p>

      <section class="x-ink">
        <div class="x-tag" aria-hidden="true"><span><b>CASH REFUND</b></span><span>PT → NT$</span></div>
        <div class="c-big x-num" data-testid="refundable">{{ fmtPts(refundable) }}</div>
        <p class="x-cap">{{ $t('me.refund.refundable') }}</p>
        <p class="c-mute c-small c-m0 rf-note">{{ $t('me.refund.onlyTopup') }}</p>
      </section>

      <p v-if="pending" class="c-ok" role="status" data-testid="pending">{{ $t('me.refund.pending', { n: pending }) }}</p>

      <template v-if="!frozen">
        <section v-if="refundable <= 0" class="c-card" role="status" data-testid="not-refundable">
          <p class="c-m0">{{ $t('me.refund.none') }}</p>
        </section>

        <form v-else class="c-card rf-form" novalidate @submit.prevent="submit">
          <div class="c-field">
            <label class="c-lbl" for="rf-amount">{{ $t('me.refund.amount') }}</label>
            <div class="c-row">
              <input id="rf-amount" v-model="amount" class="c-grow" type="text" inputmode="numeric" autocomplete="off" :placeholder="$t('me.refund.amountHint', { max: fmtPts(Math.min(refundable, REFUND_MAX)) })" :aria-invalid="!!amountErr" :aria-describedby="amountErr ? 'rf-amount-err' : undefined">
              <button type="button" class="c-btn" @click="amount = String(Math.min(refundable, REFUND_MAX))">{{ $t('me.refund.all') }}</button>
            </div>
          </div>
          <p v-if="amountErr" id="rf-amount-err" class="c-err c-small" role="alert">{{ amountErr }}</p>

          <section class="c-card" aria-labelledby="rf-sum">
            <h2 id="rf-sum" class="c-h2 c-m0">{{ $t('me.refund.summary') }}</h2>
            <p v-if="amountNum" data-testid="summary">{{ $t('me.refund.summaryLine', { pts: fmtPts(amountNum), amount: fmtPts(amountNum) }) }}</p>
            <p v-else class="c-mute">{{ $t('me.refund.summaryEmpty') }}</p>
            <ul class="c-mute c-small">
              <li>{{ $t('me.refund.note1') }}</li>
              <li>{{ $t('me.refund.note2') }}</li>
            </ul>
          </section>

          <p v-if="err" class="c-err" role="alert">{{ err }}</p>
          <button class="c-btn primary block" type="submit" :disabled="busy">{{ busy ? $t('me.refund.submitting') : $t('me.refund.submit') }}</button>
        </form>
      </template>
      <p v-else-if="err" class="c-err" role="alert">{{ err }}</p>
    </template>

    <h2 class="c-h2 x-h" data-n="01">{{ $t('me.refund.history') }}</h2>
    <p v-if="!loading && !requests.length && !listErr" class="x-empty">{{ $t('me.refund.empty') }}</p>
    <div aria-live="polite">
      <article v-for="x in requests" :key="x.id" class="c-card" :data-status="x.status">
        <div class="c-row wrap">
          <strong class="c-grow">{{ $t('me.refund.pts', { pts: fmtPts(x.amount) }) }}</strong>
          <span class="c-badge" :class="x.status">{{ refundStatusKey(x.status) ? $t(refundStatusKey(x.status)!) : x.status }}</span>
        </div>
        <div class="c-mute c-small">{{ $t('me.refund.createdAt', { time: fmtTime(x.created_at) }) }}<template v-if="x.completed_at">　{{ $t('me.refund.completedAt', { time: fmtTime(x.completed_at) }) }}</template></div>
        <template v-if="x.status === 'rejected'">
          <p v-if="x.reject_reason" class="c-err c-small c-mt">{{ $t('me.refund.rejectReason', { reason: x.reject_reason }) }}</p>
          <p class="c-mute c-small">{{ $t('me.refund.rejectedNote') }}</p>
        </template>
        <p v-else-if="x.status !== 'completed'" class="c-mute c-small c-mt">{{ $t('me.refund.inReview') }}</p>
      </article>
    </div>
  </main>
</template>
<style scoped>
.rf-note{margin-top:14px!important;padding-top:12px;border-top:1px solid color-mix(in srgb,var(--xi-fg) 22%,transparent)}
.rf-form{margin-top:20px;padding:20px}
.rf-form .c-card{background:var(--bg-alt)}
.rf-form .c-row{align-items:stretch}
.rf-form input{font:700 20px var(--mono)}
</style>
