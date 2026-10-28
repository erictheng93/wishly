<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('me.wallet.title') })
const { api, base } = useApi()
const wallet = ref<any>(null), ledger = ref<any[]>([]), lCursor = ref<string | null>(null)
const contribs = ref<any[]>([]), cCursor = ref<string | null>(null)
const loading = ref(true), err = ref(''), msg = ref('')

async function loadWallet(more = false) {
  const r = await api('/wallet', { query: { limit: 20, cursor: more ? lCursor.value : undefined } })
  wallet.value = r.wallet
  ledger.value = more ? [...ledger.value, ...r.data] : r.data; lCursor.value = r.next_cursor
}
async function loadContribs(more = false) {
  const r = await api('/wallet/contributions', { query: { limit: 20, cursor: more ? cCursor.value : undefined } })
  contribs.value = more ? [...contribs.value, ...r.data] : r.data; cCursor.value = r.next_cursor
}
async function load() {
  err.value = ''
  try { await Promise.all([loadWallet(), loadContribs()]) } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
onMounted(load)
const more = (f: () => Promise<void>) => f().catch((e) => { err.value = errMsg(e) })

const visBusy = ref(false)
async function setVis(row: any, ev: Event) {
  const select = ev.target as HTMLSelectElement
  visBusy.value = true; err.value = ''; msg.value = ''
  try {
    const r = await api(`/contributions/${row.contribution.id}`, { method: 'PATCH', body: { visibility: select.value } })
    Object.assign(row.contribution, r.contribution)
    msg.value = t('me.wallet.visUpdated')
  } catch (e) { err.value = errMsg(e) }
  finally { select.value = row.contribution.visibility; visBusy.value = false }
}

// 撤回
const wd = ref<any>(null), wdBusy = ref(false), wdErr = ref('')
async function withdraw() {
  wdBusy.value = true; wdErr.value = ''
  try {
    await api(`/contributions/${wd.value.contribution.id}`, { method: 'DELETE' })
    msg.value = t('me.wallet.withdrawn', { pts: fmtPts(wd.value.contribution.points) }); wd.value = null; await load()
  } catch (e: any) {
    wdErr.value = e.code === 'CONTRIBUTION_LOCKED' ? t('me.wallet.locked') : errMsg(e)
    if (e.code === 'CONTRIBUTION_LOCKED') await load()
  } finally { wdBusy.value = false }
}

// 轉投：目標限同一份清單、仍在募集中的其他眾籌品項（從公開頁資料取得）
const re = ref<any>(null), reTargets = ref<any[]>([]), reTarget = ref(''), reBusy = ref(false), reErr = ref(''), reKey = ref('')
async function openRe(row: any) {
  re.value = row; reTargets.value = []; reTarget.value = ''; reErr.value = ''; reKey.value = crypto.randomUUID()
  try {
    // 受限路徑：好友 / 摯友清單也讀得到（公開路徑只回 public）
    const w = await fetchListWithCreds(useRuntimeConfig().public.apiBase, row.wishlist.slug)
    reTargets.value = (w.items || []).filter((i: any) => i.funding_mode === 'crowdfund' && i.id !== row.item.id && deriveDisplayStatus(i) === 'open')
    reTarget.value = reTargets.value[0]?.id ?? ''
  } catch { reErr.value = t('me.wallet.loadOthersFailed') }
}
async function reallocate() {
  reBusy.value = true; reErr.value = ''
  try {
    await api(`/contributions/${re.value.contribution.id}/reallocate`, { method: 'POST', headers: { 'Idempotency-Key': reKey.value }, body: { target_item_id: reTarget.value } })
    msg.value = t('me.wallet.reallocated'); re.value = null; await load()
  } catch (e: any) {
    reErr.value = e.code === 'REALLOCATION_NOT_ALLOWED' ? t('me.wallet.reallocNotAllowed') : errMsg(e)
    if (e.code !== 'NETWORK') reKey.value = crypto.randomUUID()
  } finally { reBusy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('me.wallet.title')" back="/dashboard" />
    <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load">{{ $t('me.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <template v-if="wallet">
      <section class="c-card c-center">
        <div class="c-big">{{ fmtPts(wallet.balance) }}</div>
        <p class="c-mute c-m0">{{ $t('me.wallet.available') }}<span v-if="wallet.status === 'frozen'" class="c-badge frozen">{{ $t('me.wallet.frozen') }}</span></p>
        <p class="c-mute">{{ $t('me.wallet.held', { pts: fmtPts(wallet.held_points) }) }}</p>
        <p v-if="wallet.status === 'frozen'" class="c-err" role="alert">{{ $t('me.wallet.frozenWarn') }}</p>
        <p class="c-mute c-small">{{ $t('me.wallet.issuedByPlatform') }}</p>
        <button v-if="wallet.status === 'frozen'" type="button" class="c-btn primary" disabled>{{ $t('me.wallet.topup') }}</button>
        <NuxtLink v-else to="/me/topup" class="c-btn primary">{{ $t('me.wallet.topup') }}</NuxtLink>
        <div class="c-mt">
          <NuxtLink v-if="Number(wallet.cash_refundable_balance) > 0 && wallet.status !== 'frozen'" to="/me/refund" class="c-btn primary" data-testid="refund-entry">{{ $t('me.wallet.refund') }}</NuxtLink>
          <p v-else class="c-mute c-small c-m0" data-testid="refund-none">{{ $t(wallet.status === 'frozen' ? 'me.wallet.refundFrozen' : 'me.wallet.refundNone') }}</p>
          <p v-if="Number(wallet.cash_refundable_balance) > 0" class="c-mute c-small c-m0">{{ $t('me.wallet.refundable', { pts: fmtPts(wallet.cash_refundable_balance) }) }}</p>
        </div>
      </section>

      <h2 class="c-h2">{{ $t('me.wallet.myPledges') }}</h2>
      <p v-if="!contribs.length" class="c-center c-mute">{{ $t('me.wallet.noPledges') }}</p>
      <article v-for="r in contribs" :key="r.contribution.id" class="c-card">
        <div class="c-row">
          <img v-if="r.item.image_url" class="c-thumb" :src="r.item.image_url" alt="">
          <div v-else class="c-thumb">{{ $t('me.wallet.noImg') }}</div>
          <div class="c-grow">
            <strong>{{ r.item.title }}</strong>
            <div class="c-mute"><NuxtLink :to="`/s/${r.wishlist.slug}`">{{ r.wishlist.title }}</NuxtLink></div>
            <div>{{ $t('me.wallet.pts', { pts: fmtPts(r.contribution.points) }) }}　<span class="c-badge" :class="r.contribution.status">{{ label(CONTRIBUTION_STATUS, r.contribution.status) }}</span>
              <span v-if="r.item.display_status" class="c-badge" :class="r.item.display_status">{{ displayStatusLabel(r.item.display_status) }}</span></div>
          </div>
        </div>
        <div v-if="r.contribution.message" class="c-mute">{{ $t('me.wallet.quoted', { msg: r.contribution.message }) }}{{ r.contribution.is_anonymous ? $t('me.wallet.anon') : '' }}</div>
        <div class="c-mute c-small">{{ $t('me.wallet.pledgedAt', { time: fmtTime(r.contribution.created_at) }) }}<template v-if="r.contribution.captured_at">{{ $t('me.wallet.reachedAt', { time: fmtTime(r.contribution.captured_at) }) }}</template></div>
        <div v-if="r.contribution.refunded_points" class="c-mute c-small">{{ $t('me.wallet.refunded', { pts: fmtPts(r.contribution.refunded_points) }) }}</div>
        <label class="c-field c-mt"><span>{{ $t('me.whoSees') }}</span>
          <select :value="r.contribution.visibility" :disabled="visBusy" @change="setVis(r, $event)">
            <option value="public">{{ $t('me.vis.public') }}</option><option value="friends">{{ $t('me.vis.friends') }}</option><option value="private">{{ $t('me.vis.private') }}</option>
          </select>
        </label>
        <p v-if="r.contribution.is_anonymous" class="c-mute c-small">{{ $t('me.wallet.anonNote') }}</p>
        <div v-if="r.item.display_status === 'expired' && r.item.reallocation_deadline && (r.can_withdraw || r.can_reallocate)" class="c-err c-small">{{ $t('me.wallet.expiredNote', { time: fmtTime(r.item.reallocation_deadline) }) }}</div>
        <div v-if="r.can_withdraw || r.can_reallocate" class="c-row wrap c-mt">
          <button v-if="r.can_reallocate" class="c-btn c-grow" @click="openRe(r)">{{ $t('me.wallet.realloc') }}</button>
          <button v-if="r.can_withdraw" class="c-btn danger c-grow" @click="wd = r; wdErr = ''">{{ $t('me.wallet.withdraw') }}</button>
        </div>
      </article>
      <button v-if="cCursor" class="c-btn block c-mb16" @click="more(() => loadContribs(true))">{{ $t('me.wallet.loadMore') }}</button>

      <h2 class="c-h2">{{ $t('me.wallet.ledger') }}</h2>
      <p v-if="!ledger.length" class="c-center c-mute">{{ $t('me.wallet.noLedger') }}</p>
      <section v-else class="c-card">
        <ul class="c-list">
          <li v-for="e in ledger" :key="e.id" class="c-row wrap">
            <span class="c-grow"><strong>{{ label(LEDGER_TYPE, e.entry_type) }}</strong><span v-if="e.note" class="c-mute">　{{ e.note }}</span>
              <span class="c-mute c-small" style="display:block">{{ $t('me.wallet.ledgerMeta', { time: fmtTime(e.created_at), bal: fmtPts(e.balance_after) }) }}</span></span>
            <b :class="e.delta >= 0 ? 'c-pos' : 'c-neg'">{{ e.delta > 0 ? '+' : '' }}{{ fmtPts(e.delta) }}</b>
          </li>
        </ul>
        <button v-if="lCursor" class="c-btn block c-mt" @click="more(() => loadWallet(true))">{{ $t('me.wallet.loadMore') }}</button>
      </section>
    </template>

    <CreatorConfirm :open="!!wd" :title="$t('me.wallet.wdTitle')" :text="$t('me.wallet.wdText', { item: wd?.item.title, pts: fmtPts(wd?.contribution.points) })" :ok="$t('me.wallet.wdOk')" danger :busy="wdBusy" :error="wdErr" @close="wd = null" @ok="withdraw" />
    <CreatorConfirm :open="!!re" :title="$t('me.wallet.reTitle')" :text="$t('me.wallet.reText', { pts: fmtPts(re?.contribution.points) })" :ok="$t('me.wallet.reOk')" :busy="reBusy" :disabled="!reTarget" :error="reErr" @close="re = null" @ok="reallocate">
      <p v-if="re && !reTargets.length && !reErr" class="c-mute">{{ $t('me.wallet.noTargets') }}</p>
      <label v-if="reTargets.length" class="c-field"><span>{{ $t('me.wallet.moveTo') }}</span>
        <select v-model="reTarget"><option v-for="tg in reTargets" :key="tg.id" :value="tg.id">{{ $t('me.wallet.targetOpt', { title: tg.title, pts: fmtPts(tg.remaining_points) }) }}</option></select></label>
    </CreatorConfirm>
  </main>
</template>
