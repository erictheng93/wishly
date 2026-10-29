<script setup lang="ts">
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t, locale } = useI18n()
useHead({ title: () => t('lists.progress.pageTitle') })
const dl = computed(() => ({ en: 'en', vi: 'vi', id: 'id' } as Record<string, string>)[locale.value] ?? 'zh-TW')
const id = useRoute().params.id as string
const { api } = useApi()
const d = ref<any>(null)
const w = ref<any>(null)
const orders = ref<any>(null)
const err = ref(''), loading = ref(true), updatedAt = ref('')
const slug = computed(() => w.value?.slug)
let tm: any, countdown: any
const left = ref(0)

// SSE 重抓與操作後重抓可能重疊：只套用最後一次發出的請求，避免較早的回應晚到而蓋掉新狀態
let loadSeq = 0
async function load() {
  const seq = ++loadSeq
  try {
    if (!w.value) w.value = (await api(`/wishlists/${id}`)).wishlist
    const dash = await api(`/wishlists/${id}/dashboard`)
    // 採購單進度：只有含眾籌品項才需要；失敗不影響其他區塊（鎖定時 data 為 null，只有彙總）
    const ord = (dash.items || []).some((i: any) => i.funding_mode === 'crowdfund') ? await api(`/wishlists/${id}/orders`).catch(() => null) : null
    if (seq !== loadSeq) return
    d.value = dash; orders.value = ord
    updatedAt.value = new Date().toLocaleTimeString(dl.value)
    err.value = ''
    startCountdown()
  } catch (e) { if (seq === loadSeq) err.value = errMsg(e) } finally { loading.value = false }
}
function startCountdown() {
  clearInterval(countdown)
  if (!d.value?.surprise_locked || !d.value.unlock_at) return
  const end = Date.parse(d.value.unlock_at)
  const f = () => { left.value = end - Date.now(); if (left.value <= 0) { clearInterval(countdown); load() } } // 解鎖後自動重載
  f(); countdown = setInterval(f, 1000)
}
const days = computed(() => Math.floor(left.value / 864e5))
const hms = computed(() => { const s = Math.max(0, Math.floor(left.value / 1000)) % 86400; return t('lists.progress.hms', { h: Math.floor(s / 3600), m: Math.floor(s % 3600 / 60), s: s % 60 }) })

// SSE 事件合併重取（debounce 500ms）
const { status } = useWishlistEvents(slug, () => { clearTimeout(tm); tm = setTimeout(load, 500) })
onMounted(load)
onBeforeUnmount(() => { clearTimeout(tm); clearInterval(countdown) })

const locked = computed(() => !!d.value?.surprise_locked)
const claimsByItem = computed(() => {
  const m: Record<string, any[]> = {}
  for (const c of d.value?.claims || []) (m[c.item_id] ||= []).push(c)
  return m
})
const contribsByItem = computed(() => {
  const m: Record<string, any[]> = {}
  for (const c of d.value?.contributions || []) (m[c.item_id] ||= []).push(c)
  return m
})
const orderByItem = computed(() => Object.fromEntries((orders.value?.data || []).map((o: any) => [o.item_id, o])))
const fundPct = (i: any) => i.target_points > 0 ? Math.min(100, Math.floor(i.pledged_points / i.target_points * 100)) : 0
const fundStatus = (i: any) => deriveDisplayStatus(i, orderByItem.value[i.item_id])
const pct = computed(() => d.value?.totals?.completion_pct ?? 0)
const busyClaim = ref('')
const claimErr = ref('')
async function setStatus(c: any, status: 'delivered' | 'cancelled') {
  // 已送達不可復原（後端不允許退回），兩種操作都先確認
  if (!confirm(t(status === 'cancelled' ? 'lists.progress.cancelConfirm' : 'lists.progress.deliverConfirm', { name: c.is_anonymous ? t('lists.progress.anon') : c.claimer_name || t('lists.progress.guest') }))) return
  busyClaim.value = c.id; claimErr.value = ''
  try { await api(`/claims/${c.id}`, { method: 'PATCH', body: { status } }); await load() }
  catch (e) { claimErr.value = errMsg(e) } finally { busyClaim.value = '' }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="w?.title ? $t('lists.progress.headerTitle', { title: w.title }) : $t('lists.progress.pageTitle')" :back="`/lists/${id}/edit`">
      <button class="c-btn" :aria-label="$t('lists.progress.refresh')" @click="load">↻</button>
    </CreatorHeader>
    <p v-if="loading" class="c-mute" role="status">{{ $t('lists.common.loading') }}</p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load">{{ $t('lists.common.retry') }}</button></p>
    <template v-if="d">
      <section class="x-ink pg-hero">
        <div class="x-tag" aria-hidden="true"><span><i class="x-dot" />{{ status === 'live' ? 'LIVE' : 'SNAPSHOT' }}</span><span>OVERALL</span></div>
        <div class="c-big x-num">{{ pct }}<small>%</small></div>
        <div class="c-bar" role="progressbar" :aria-valuenow="pct" aria-valuemin="0" aria-valuemax="100" :aria-label="$t('lists.progress.overall')"><i :style="{ width: pct + '%' }" /></div>
        <p class="c-mute">{{ $t('lists.progress.complete', { done: d.totals.fulfilled_count, total: d.totals.item_count }) }}{{ $t('lists.progress.qtyPart', { claimed: d.totals.qty_claimed, needed: d.totals.qty_needed }) }}</p>
        <p v-if="!locked && d.totals.target_points" class="c-mute">{{ $t('lists.progress.pointsLine', { pledged: fmtPts(d.totals.pledged_points), target: fmtPts(d.totals.target_points), n: d.totals.funded_item_count ?? 0 }, d.totals.funded_item_count ?? 0) }}</p>
        <p class="c-mute c-small">{{ status === 'live' ? $t('lists.progress.live') : status === 'polling' ? $t('lists.progress.polling') : '' }}　{{ $t('lists.progress.updatedAt', { time: updatedAt }) }}</p>
      </section>

      <section v-if="locked" class="c-mask" role="status">
        <strong>{{ $t('lists.progress.surprise') }}</strong>
        <p>{{ $t('lists.progress.surpriseDesc') }}</p>
        <p v-if="!d.unlock_at">{{ $t('lists.progress.noEventDate') }}</p>
        <p v-else>{{ $t('lists.progress.countdown', { days, hms, at: new Date(d.unlock_at).toLocaleString(dl, { timeZone: 'Asia/Taipei' }) }, days) }}</p>
      </section>

      <p v-if="!d.totals.item_count" class="x-empty">{{ $t('lists.progress.noItems') }}</p>
      <p v-else-if="!locked && !d.totals.qty_claimed && !d.totals.pledged_points" class="x-empty">{{ $t('lists.progress.nobody') }}</p>
      <p v-if="claimErr" class="c-err" role="alert">{{ claimErr }}</p>

      <h2 class="c-h2 x-h" data-n="ITEMS">{{ $t('lists.progress.itemsTitle') }}</h2>
      <article v-for="(i, n) in d.items" :key="i.item_id" class="c-card pg-item">
        <span class="pg-n" aria-hidden="true">{{ String(Number(n) + 1).padStart(2, '0') }}</span>
        <template v-if="i.funding_mode === 'crowdfund'">
          <div class="c-row"><strong class="c-grow">{{ i.title }}</strong>
            <span v-if="i.pledged_points === null" class="c-mute">{{ $t('lists.progress.hiddenProgress') }}</span>
            <span v-else-if="fundStatus(i)" class="c-badge" :class="fundStatus(i)">{{ displayStatusLabel(fundStatus(i)) }}</span>
          </div>
          <template v-if="i.pledged_points !== null">
            <div class="c-bar" role="progressbar" :aria-valuenow="fundPct(i)" aria-valuemin="0" aria-valuemax="100" :aria-label="i.title"><i :style="{ width: fundPct(i) + '%' }" /></div>
            <p class="c-mute c-m0">{{ $t('lists.progress.raised', { pledged: fmtPts(i.pledged_points), target: fmtPts(i.target_points) }) }}<template v-if="i.funding_deadline && fundStatus(i) === 'open'">{{ $t('lists.progress.deadline', { time: fmtTime(i.funding_deadline) }) }}</template></p>
          </template>
          <p v-if="i.claimer_count" class="c-mute c-small c-m0">{{ $t('lists.progress.claimerCount', { n: i.claimer_count }, i.claimer_count) }}</p>
          <div v-if="!locked" class="c-sub">
            <strong class="c-small">{{ $t('lists.progress.donors') }}</strong>
            <p v-if="!(contribsByItem[i.item_id] || []).length" class="c-mute c-m0">{{ $t('lists.progress.noDonors') }}</p>
            <ul v-else class="c-list">
              <li v-for="c in contribsByItem[i.item_id]" :key="c.id">
                <div class="c-row wrap">
                  <span class="c-grow"><strong>{{ c.is_anonymous ? $t('lists.progress.anon') : c.display_name }}</strong>　{{ $t('lists.progress.pts', { n: fmtPts(c.points) }) }}</span>
                  <span class="c-badge" :class="c.status">{{ label(CONTRIBUTION_STATUS, c.status) }}</span>
                </div>
                <div v-if="c.message" class="c-mute">{{ $t('lists.progress.quote', { msg: c.message }) }}</div>
                <div class="c-mute c-small">{{ $t('lists.progress.pledgedAt', { time: fmtTime(c.created_at) }) }}<template v-if="c.captured_at">{{ $t('lists.progress.capturedAt', { time: fmtTime(c.captured_at) }) }}</template></div>
                <div v-if="c.refunded_points" class="c-mute c-small">{{ $t('lists.progress.refunded', { refunded: fmtPts(c.refunded_points), spent: fmtPts(c.spent_points ?? c.points - c.refunded_points) }) }}</div>
              </li>
            </ul>
            <template v-if="orderByItem[i.item_id]">
              <strong class="c-small">{{ $t('lists.progress.orderTitle') }}</strong>
              <dl class="c-dl">
                <dt>{{ $t('lists.progress.status') }}</dt><dd>{{ label(ORDER_STATUS, orderByItem[i.item_id].status) }}</dd>
                <template v-if="orderByItem[i.item_id].tracking_no"><dt>{{ $t('lists.progress.tracking') }}</dt><dd>{{ orderByItem[i.item_id].tracking_no }}</dd></template>
                <template v-if="orderByItem[i.item_id].placed_at"><dt>{{ $t('lists.progress.placedAt') }}</dt><dd>{{ fmtTime(orderByItem[i.item_id].placed_at) }}</dd></template>
                <template v-if="orderByItem[i.item_id].shipped_at"><dt>{{ $t('lists.progress.shippedAt') }}</dt><dd>{{ fmtTime(orderByItem[i.item_id].shipped_at) }}</dd></template>
                <template v-if="orderByItem[i.item_id].delivered_at"><dt>{{ $t('lists.progress.deliveredAt') }}</dt><dd>{{ fmtTime(orderByItem[i.item_id].delivered_at) }}</dd></template>
              </dl>
            </template>
            <p v-else-if="fundStatus(i) === 'funded'" class="c-mute c-small">{{ $t('lists.progress.fundedWait') }}</p>
          </div>
        </template>
        <template v-else>
        <div class="c-row"><strong class="c-grow">{{ i.title }}</strong>
          <span v-if="i.qty_claimed === null" class="c-mute">{{ $t('lists.progress.hiddenProgress') }}</span>
          <span v-else-if="i.qty_claimed >= i.qty_needed" class="c-badge active">{{ $t('lists.progress.done') }}</span>
          <span v-else>{{ i.qty_claimed }} / {{ i.qty_needed }}</span>
        </div>
        <div v-if="i.qty_claimed !== null" class="c-bar" role="progressbar" :aria-valuenow="Math.min(100, Math.round(i.qty_claimed / i.qty_needed * 100))" aria-valuemin="0" aria-valuemax="100" :aria-label="i.title"><i :style="{ width: Math.min(100, i.qty_claimed / i.qty_needed * 100) + '%' }" /></div>
        <p v-if="i.claimer_count" class="c-mute c-small c-m0">{{ $t('lists.progress.claimerCount', { n: i.claimer_count }, i.claimer_count) }}</p>
        <ul v-for="c in claimsByItem[i.item_id] || []" :key="c.id" class="c-list">
          <li class="c-row wrap">
            <span class="c-grow" style="flex-basis:100%">{{ c.is_anonymous ? $t('lists.progress.anon') : c.claimer_name || $t('lists.progress.guest') }} ×{{ c.qty }}<template v-if="c.note"> ・{{ c.note }}</template></span>
            <CreatorStatusBadge :v="c.status" />
            <template v-if="c.status === 'reserved' || c.status === 'purchased'">
              <button class="c-btn" :disabled="busyClaim === c.id" @click="setStatus(c, 'delivered')">{{ $t('lists.progress.markDelivered') }}</button>
              <button class="c-btn danger" :disabled="busyClaim === c.id" @click="setStatus(c, 'cancelled')">{{ $t('lists.progress.cancelClaim') }}</button>
            </template>
          </li>
        </ul>
        </template>
      </article>
      <section v-if="d.orders_summary && (d.orders_summary.funded_count || d.orders_summary.ordered_count || d.orders_summary.shipped_count || d.orders_summary.delivered_count)" class="c-card">
        <strong>{{ $t('lists.progress.ordersTitle') }}</strong>
        <p class="c-mute">{{ $t('lists.progress.ordersLine', { funded: d.orders_summary.funded_count, ordered: d.orders_summary.ordered_count, shipped: d.orders_summary.shipped_count, delivered: d.orders_summary.delivered_count }) }}</p>
      </section>
      <NuxtLink v-if="!d.totals.qty_claimed && !d.totals.pledged_points && !locked" :to="`/lists/${id}/edit`" class="c-btn primary block">{{ $t('lists.progress.shareList') }}</NuxtLink>
    </template>
  </main>
</template>
<style scoped>
.pg-hero .x-num small{font-size:.4em;letter-spacing:0;margin-left:4px;color:var(--red)}
.pg-hero .c-bar{height:12px;background:repeating-linear-gradient(-45deg,color-mix(in srgb,var(--xi-fg) 38%,transparent) 0 1px,transparent 1px 7px),color-mix(in srgb,var(--xi-fg) 14%,transparent)}
.pg-hero .c-mute{margin:10px 0 0;font:600 12px/1.6 var(--mono);letter-spacing:.03em}
.pg-hero .c-bar+.c-mute{margin-top:14px}
.pg-item{padding:18px 16px 16px;margin-bottom:12px}
.pg-n{position:absolute;top:0;right:0;padding:6px 10px;border-left:1px solid var(--line);border-bottom:1px solid var(--line);font:600 11px/1 var(--mono);letter-spacing:.14em;color:var(--mute);background:var(--bg-alt)}
.pg-item>.c-row:first-of-type{padding-right:44px}
.pg-item>.c-row>strong{font-size:17px;line-height:1.3}
.pg-item .c-bar{height:10px;margin:12px 0 8px}
.pg-item .c-list>li{padding:12px 0;border-top:1px solid var(--line)}
</style>
