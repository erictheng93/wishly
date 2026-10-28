<script setup lang="ts">
// 儲值：建單後以隱藏 <form method=post> 導向綠界付款頁（不可用 fetch）；紀錄列表含超商代碼 / 條碼 / ATM 取號資訊
import { TOPUP_METHODS } from '~/utils/topup' // 自動匯入掃描漏掉這個常數
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('me.topup.title') })
const { api } = useApi()

const WATCH_KEY = 'ws_topup_watch' // 送出後暫存的儲值單 id：從綠界返回時據此輪詢到入帳
const POLL_MS = 3000, POLL_MAX = 10

const native = isNative(), quick = QUICK_AMOUNTS
const { methods: enabled, ready: statusReady } = useTopupMethods()
const methods = computed(() => TOPUP_METHODS.filter(m => enabled.value.includes(m)))
const form = reactive(emptyForm())
const submitted = ref(false), busy = ref(false), disabled = ref(false)
// 後端未開放（status 無可用方式或建單回 TOPUP_DISABLED）；狀態取不到視為未開放
const off = computed(() => disabled.value || (statusReady.value && !methods.value.length))
const frozen = computed(() => wallet.value?.status === 'frozen')
watchEffect(() => { if (methods.value.length && !methods.value.includes(form.method)) form.method = methods.value[0]! })
const err = ref(''), serverFields = ref<Partial<Record<string, string>>>({})
const pay = ref<{ url: string, fields: Record<string, string> } | null>(null)
const payForm = ref<HTMLFormElement | null>(null)
const idem = idemKeys()

const wallet = ref<any>(null), topups = ref<any[]>([])
const loading = ref(true), listErr = ref(''), success = ref('')
const stalled = ref(false), refreshing = ref(false)
const now = ref(Date.now())
const copied = ref('')

const clientErrors = computed(() => formErrors(form))
const fieldErr = (f: 'amount') => {
  if (serverFields.value[f]) return serverFields.value[f]
  const k = clientErrors.value[f]
  // 金額欄一開始是空的，送出前不吵；其餘欄位一樣等到按下送出才顯示
  return submitted.value && k ? t(k) : ''
}
const amountNum = computed(() => (amountError(form.amount) ? 0 : Number(form.amount)))
const picked = (n: number) => form.amount === String(n)
// 內容一改就清掉後端逐欄訊息與錯誤（舊訊息已不適用）
watch(() => ({ ...form }), () => { serverFields.value = {}; err.value = '' })

async function loadWallet() { wallet.value = (await api('/wallet', { query: { limit: 1 } })).wallet }
async function loadTopups() { topups.value = (await api('/wallet/topups', { query: { limit: 20 } })).topups ?? [] }

let timer: ReturnType<typeof setTimeout> | undefined, ticker: ReturnType<typeof setInterval> | undefined
let polls = 0
const readWatch = () => { try { return sessionStorage.getItem(WATCH_KEY) } catch { return null } }
const setWatch = (v: string | null) => { try { v ? sessionStorage.setItem(WATCH_KEY, v) : sessionStorage.removeItem(WATCH_KEY) } catch { /* 無痕模式等：略過 */ } }

const needsPoll = () => topups.value.some(x => awaitingPaymentInfo(x) || (x.id === readWatch() && x.status === 'pending'))

async function refresh() {
  const prev = new Map(topups.value.map(x => [x.id, x.status]))
  listErr.value = ''
  try {
    await Promise.all([loadWallet(), loadTopups()])
  } catch (e: any) {
    if (e?.code === 'TOPUP_DISABLED') disabled.value = true
    else listErr.value = errMsg(e)
    return
  }
  const w = readWatch()
  const watched = topups.value.find(x => x.id === w)
  const justPaid = topups.value.find(x => x.status === 'paid' && (prev.get(x.id) === 'pending' || x === watched))
  if (justPaid) success.value = t('me.topup.success', { pts: fmtPts(justPaid.amount), bal: fmtPts(wallet.value?.balance) })
  if (watched && watched.status !== 'pending') setWatch(null)
}
function schedule() {
  clearTimeout(timer)
  if (!needsPoll()) { stalled.value = false; return }
  if (polls >= POLL_MAX) { stalled.value = true; return }
  timer = setTimeout(async () => { polls++; await refresh(); schedule() }, POLL_MS)
}
async function manualRefresh() {
  refreshing.value = true; polls = 0; stalled.value = false
  await refresh(); refreshing.value = false; schedule()
}
async function init() {
  loading.value = true
  await refresh()
  loading.value = false
  schedule()
}
const onPageShow = (ev: PageTransitionEvent) => { if (ev.persisted) { busy.value = false; pay.value = null; init() } }
onMounted(() => { if (!native) init(); else loading.value = false; ticker = setInterval(() => { now.value = Date.now() }, 30_000); window.addEventListener('pageshow', onPageShow) })
onBeforeUnmount(() => { clearTimeout(timer); clearInterval(ticker); window.removeEventListener('pageshow', onPageShow) })

async function submit() {
  submitted.value = true
  if (busy.value || Object.keys(clientErrors.value).length) return
  busy.value = true; err.value = ''; serverFields.value = {}
  const body = buildBody(form)
  try {
    const r = await api('/wallet/topups', { method: 'POST', headers: { 'Idempotency-Key': idem.keyFor(body) }, body })
    setWatch(r.topup.id)
    idem.reset()
    pay.value = r.payment
    await nextTick()
    payForm.value?.submit() // 導向綠界；busy 維持 true 防重複點擊
  } catch (e: any) {
    const r = topupError(e, (k, p) => t(k, p), fmtPts)
    err.value = r.msg; serverFields.value = r.fields
    if (r.disabled) disabled.value = true
    busy.value = false
  }
}

const cx = ref<any>(null), cxBusy = ref(false), cxErr = ref('')
async function cancelTopup() {
  cxBusy.value = true; cxErr.value = ''
  try {
    await api(`/wallet/topups/${cx.value.id}/cancel`, { method: 'POST' })
    cx.value = null; await refresh()
  } catch (e: any) {
    if (e.code === 'INVALID_STATE_TRANSITION') { cx.value = null; await refresh() } // 已不是待付款（剛入帳或逾期）：關閉並重讀
    else cxErr.value = errMsg(e)
  } finally { cxBusy.value = false }
}
async function copy(v: string) {
  try { await copyText(v); copied.value = v } catch { copied.value = ''; err.value = t('me.topup.copyFail') }
  setTimeout(() => { if (copied.value === v) copied.value = '' }, 2000)
}
const left = (iso?: string | null) => {
  const r = remaining(iso, now.value)
  if (!r) return t('me.topup.expired')
  return r.d ? t('me.topup.leftDH', r) : r.h ? t('me.topup.leftHM', r) : t('me.topup.leftM', r)
}
const methodDeadline = (m: string) => t(m === 'card' ? 'me.topup.deadlineCard' : 'me.topup.deadlineOther')
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('me.topup.title')" back="/me/wallet" />

    <section v-if="native" class="c-card">
      <p>{{ $t('me.topup.nativeText') }}</p>
      <button class="c-btn primary" @click="openExternal(`${siteOrigin()}/me/topup`)">{{ $t('me.topup.nativeLink') }}</button>
    </section>

    <template v-else>
      <div aria-live="polite">
        <p v-if="success" class="c-ok" role="status">{{ success }}</p>
      </div>
      <p v-if="wallet && wallet.status === 'frozen'" class="c-err" role="alert" data-testid="frozen">{{ $t('me.frozenBanner') }}</p>
      <p v-if="wallet" class="c-mute">{{ $t('me.topup.balance', { pts: fmtPts(wallet.balance) }) }}</p>

      <p v-if="!statusReady && !disabled" class="c-mute" role="status">{{ $t('me.loading') }}</p>
      <section v-else-if="off" class="c-card" role="status">
        <p class="c-m0">{{ $t('me.topup.err.disabled') }}</p>
      </section>

      <form v-else-if="!frozen" class="c-card" novalidate @submit.prevent="submit">
        <fieldset class="c-field" :aria-describedby="fieldErr('amount') ? 'tp-amount-err' : undefined">
          <legend class="c-lbl">{{ $t('me.topup.amount') }}</legend>
          <div class="c-row wrap">
            <button v-for="n in quick" :key="n" type="button" class="c-btn c-grow" :class="{ primary: picked(n) }" :aria-pressed="picked(n)" @click="form.amount = String(n)">{{ fmtPts(n) }}</button>
          </div>
          <label class="c-field c-mt"><span>{{ $t('me.topup.customAmount') }}</span>
            <input v-model="form.amount" type="text" inputmode="numeric" autocomplete="off" :placeholder="$t('me.topup.amountHint')" :aria-invalid="!!fieldErr('amount')">
          </label>
          <p v-if="fieldErr('amount')" id="tp-amount-err" class="c-err c-small" role="alert">{{ fieldErr('amount') }}</p>
        </fieldset>

        <fieldset class="c-field">
          <legend class="c-lbl">{{ $t('me.topup.methodLegend') }}</legend>
          <label v-for="m in methods" :key="m" class="c-row c-mt12" style="align-items:flex-start;gap:8px">
            <input v-model="form.method" type="radio" name="method" :value="m" style="width:auto;min-height:0;margin-top:4px">
            <span class="c-grow"><strong>{{ $t(`me.topup.method.${m}`) }}</strong>
              <span class="c-mute c-small" style="display:block">{{ $t(`me.topup.methodDesc.${m}`) }}　{{ methodDeadline(m) }}</span></span>
          </label>
        </fieldset>

        <p class="c-mute c-small">{{ $t('me.topup.invoiceMember') }}</p>

        <section class="c-card" aria-labelledby="tp-sum">
          <h2 id="tp-sum" class="c-h2 c-m0">{{ $t('me.topup.summary') }}</h2>
          <p v-if="amountNum" data-testid="summary">{{ $t('me.topup.summaryLine', { amount: fmtPts(amountNum), pts: fmtPts(amountNum) }) }}</p>
          <p v-else class="c-mute">{{ $t('me.topup.summaryEmpty') }}</p>
          <ul class="c-mute c-small">
            <li>{{ $t('me.topup.note1') }}</li>
            <li>{{ $t('me.topup.note2') }}</li>
          </ul>
        </section>

        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <button class="c-btn primary block" type="submit" :disabled="busy">{{ busy ? $t('me.topup.submitting') : $t('me.topup.submit') }}</button>
      </form>

      <!-- 導向綠界：fields 全部以 hidden input 送出 -->
      <form v-if="pay" ref="payForm" method="post" :action="pay.url" hidden data-testid="pay-form">
        <input v-for="(v, k) in pay.fields" :key="k" type="hidden" :name="String(k)" :value="v">
      </form>

      <h2 class="c-h2">{{ $t('me.topup.history') }}</h2>
      <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
      <p v-if="listErr" class="c-err" role="alert">{{ listErr }} <button class="c-btn" @click="manualRefresh">{{ $t('me.retry') }}</button></p>
      <p v-if="!loading && !topups.length && !listErr" class="c-center c-mute">{{ $t('me.topup.empty') }}</p>
      <div aria-live="polite">
        <article v-for="x in topups" :key="x.id" class="c-card" :data-status="x.status">
          <div class="c-row wrap">
            <strong class="c-grow">NT$ {{ fmtPts(x.amount) }}　<span class="c-mute">{{ $t(`me.topup.method.${x.method}`) }}</span></strong>
            <span class="c-badge" :class="x.status">{{ $t(`me.topup.status.${x.status}`, x.status) }}</span>
          </div>
          <div class="c-mute c-small">{{ $t('me.topup.createdAt', { time: fmtTime(x.created_at) }) }}<template v-if="x.paid_at">　{{ $t('me.topup.paidAt', { time: fmtTime(x.paid_at) }) }}</template></div>

          <template v-if="x.status === 'pending' && x.method !== 'card'">
            <p v-if="!x.payment_info" class="c-mute c-mt" role="status">{{ $t('me.topup.fetching') }}</p>
            <div v-else class="c-mt" data-testid="payment-info">
              <div v-for="r in paymentRows(x.payment_info)" :key="r.key" class="c-row wrap">
                <span class="c-mute c-small">{{ $t(`me.topup.pay.${r.key}`) }}</span>
                <code class="c-grow" style="font-family:ui-monospace,Menlo,Consolas,monospace;font-size:16px;word-break:break-all">{{ r.value }}</code>
                <button type="button" class="c-btn small" @click="copy(r.value)">{{ copied === r.value ? $t('me.topup.copied') : $t('me.topup.copy') }}</button>
                <Code39Barcode v-if="x.payment_info.type === 'cvs_barcode'" :value="r.value" class="c-mt12" />
              </div>
              <p class="c-small c-mt12">{{ $t('me.topup.expireAt', { time: fmtTime(x.payment_info.expire_at || x.expires_at) }) }}（{{ left(x.payment_info.expire_at || x.expires_at) }}）</p>
            </div>
          </template>
          <p v-else-if="x.status === 'pending'" class="c-mute c-small c-mt">{{ $t('me.topup.cardPending', { time: fmtTime(x.expires_at) }) }}<br>{{ $t('me.topup.cardRetry') }}</p>
          <button v-if="x.status === 'pending'" type="button" class="c-btn small c-mt" data-testid="cancel-topup" @click="cx = x; cxErr = ''">{{ $t('me.topup.cancel') }}</button>
        </article>
      </div>
      <p v-if="stalled" class="c-mute" role="status">{{ $t('me.topup.stalled') }}</p>
      <button v-if="!loading" class="c-btn block c-mb16" type="button" :disabled="refreshing" @click="manualRefresh">{{ $t('me.topup.refresh') }}</button>
      <CreatorConfirm :open="!!cx" :title="$t('me.topup.cancel')" :text="$t('me.topup.cancelText')" :ok="$t('me.topup.cancel')" danger :busy="cxBusy" :error="cxErr" @close="cx = null" @ok="cancelTopup" />
    </template>
  </main>
</template>
