<script setup lang="ts">
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('lists.edit.pageTitle') })
const route = useRoute()
const id = route.params.id as string
const { api } = useApi()
const w = ref<any>(null)
const items = ref<any[]>([])
const loading = ref(true), err = ref(''), msg = ref(''), stale = ref(false)
const sheet = ref(false), editing = ref<any>(null), prefill = ref<ItemPrefill>({})
const del = ref<any>(null), delBusy = ref(false), delErr = ref('')
const pubBusy = ref(false), pubErrs = ref<string[]>([])
const canNativeShare = ref(false), invite = ref<any>(null), inviteOn = ref(false), inviteBusy = ref(false), inviteErr = ref('')
const external = ref(false), copied = ref(false), showShare = ref(false)
const addrOpen = ref(false), addrEl = ref<HTMLElement>()
function openAddress() { sheet.value = false; addrOpen.value = true; nextTick(() => addrEl.value?.scrollIntoView({ block: 'center' })) }
const meta = reactive<any>({ title: '', description: '', event_date: '', closes_at: '', show_claimer_names: false, visibility: 'friends', claim_ttl_hours: 168, birthday_notify: false })
const allowed = ref<string[]>([])
const rcptKey = ref('')
const relief = computed(() => w.value?.type === 'relief')
// 實收登記：每筆品項一張 sheet；明細來自 GET /orgs/{org}/sites/{id}/receipts
const receipts = ref<Record<string, any>>({}), rcpt = ref<any>(null), rcptF = reactive({ qty: 1, note: '' }), rcptBusy = ref(false), rcptErr = ref('')
async function loadReceipts() {
  if (!relief.value) return
  try { receipts.value = Object.fromEntries((await api(`/orgs/${w.value.org_id}/sites/${id}/receipts`)).items.map((r: any) => [r.item_id, r])) } catch { /* 明細失敗不擋編輯 */ }
}
function openReceipt(i: any) { rcptKey.value = crypto.randomUUID(); rcpt.value = i; rcptF.qty = 1; rcptF.note = ''; rcptErr.value = '' }
async function saveReceipt() {
  rcptBusy.value = true; rcptErr.value = ''
  try {
    await api(`/orgs/${w.value.org_id}/receipts`, { method: 'POST', headers: { 'Idempotency-Key': rcptKey.value }, noRedirect: true, body: { item_id: rcpt.value.id, qty: Number(rcptF.qty), note: rcptF.note || undefined } })
    rcpt.value = null; await load(true)
  } catch (e: any) { rcptErr.value = errMsg(e); if (e.code !== 'NETWORK') rcptKey.value = crypto.randomUUID() } finally { rcptBusy.value = false }
}
const permits = ref<any[]>([])
const permitOpts = computed(() => {
  const today = taipeiToday()
  return permits.value.filter(p => p.id === w.value?.permit_id || (p.status === 'approved' && p.valid_from <= today && p.valid_to >= today))
})
async function loadPermits() {
  if (!relief.value) return
  try { permits.value = await api(`/orgs/${w.value.org_id}/permits`) } catch { /* 許可清單失敗不擋編輯 */ }
}
const coverPreset = ref<string | null>(null), coverKey = ref<string | null>(null), coverBusy = ref(false)
let poll: any

async function load(quiet = false) {
  if (!quiet) loading.value = true
  try {
    const r = await api(`/wishlists/${id}`)
    w.value = r.wishlist; items.value = r.items
    Object.assign(meta, { title: r.wishlist.title, description: r.wishlist.description ?? '', event_date: r.wishlist.event_date ?? '', closes_at: toLocalInput(r.wishlist.closes_at), show_claimer_names: r.wishlist.show_claimer_names, visibility: r.wishlist.visibility, claim_ttl_hours: r.wishlist.claim_ttl_hours, birthday_notify: !!r.wishlist.birthday_notify })
    coverPreset.value = r.wishlist.cover_preset ?? null; coverKey.value = null
    if (r.wishlist.visibility === 'selected') allowed.value = (await api(`/wishlists/${id}/allowed-users`).catch(() => ({ users: [] }))).users.map((u: any) => u.id)
    err.value = ''; stale.value = false
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
  loadReceipts(); if (!permits.value.length) loadPermits()
  clearTimeout(poll)
  if (items.value.some(i => i.image_status === 'pending')) poll = setTimeout(() => load(true), 4000) // 圖片處理中輪詢
}
onMounted(async () => {
  canNativeShare.value = typeof navigator !== 'undefined' && !!navigator.share
  await load()
  // 靈感頁連結帶 ?add=1 開啟新增；開完即清掉 query，重整/返回不再重開
  if (!route.query.add || !w.value) return // 載入失敗時保留 query，重整可再試
  if (route.query.add === '1' && !closed.value) openItem(null, sanitizeItemPrefill(route.query))
  navigateTo({ path: route.path, query: {} }, { replace: true })
})
onBeforeUnmount(() => clearTimeout(poll))

const hidden = computed(() => w.value?.moderation_status === 'hidden')
const shareUrl = computed(() => {
  if (!w.value) return ''
  const u = (inviteOn.value && invite.value?.url) || w.value.share_url || `${siteOrigin()}/s/${w.value.slug}`
  return external.value ? u + (u.includes('?') ? '&' : '?') + 'openExternalBrowser=1' : u
})
function openItem(i: any = null, p: ItemPrefill = {}) { editing.value = i; prefill.value = p; sheet.value = true }
const tour = useOnboarding()
async function saved() { sheet.value = false; await load(true); tour.refresh() }
async function patchList(body: any, ok = t('lists.edit.saved')) {
  msg.value = ''; err.value = ''; stale.value = false
  try { w.value = await api(`/wishlists/${id}`, { method: 'PATCH', body: { ...body, expected_updated_at: w.value?.updated_at } }); msg.value = ok; return true }
  catch (e: any) {
    if (e.code === 'STALE_VERSION') { stale.value = true; err.value = t('lists.edit.errStale'); return false }
    err.value = errMsg(e)
    if (e.code === 'WISHLIST_NOT_PUBLISHABLE') {
      pubErrs.value = (e.errors || []).map((x: any) => x.detail)
      if ((e.errors || []).some((x: any) => x.code === 'SHIPPING_ADDRESS_REQUIRED')) openAddress()
    }
    return false
  }
}
async function saveMeta() {
  const body: any = { title: meta.title.trim(), description: meta.description || null, event_date: meta.event_date || null, closes_at: closesAtChange(meta.closes_at, w.value?.closes_at), claim_ttl_hours: meta.claim_ttl_hours }
  if (!relief.value) body.show_claimer_names = meta.show_claimer_names
  if (!relief.value && !w.value?.org_id && meta.birthday_notify !== !!w.value?.birthday_notify) body.birthday_notify = meta.birthday_notify
  if (!relief.value) body.visibility = meta.visibility // 救災據點的可見性由後端固定
  // 封面互斥；沒動就不送
  if (coverKey.value) Object.assign(body, { cover_image_key: coverKey.value, cover_preset: null })
  else if (coverPreset.value !== (w.value?.cover_preset ?? null)) Object.assign(body, { cover_preset: coverPreset.value, cover_image_key: null })
  if (!await patchList(body)) return
  if (!relief.value && meta.visibility === 'selected') {
    try { await api(`/wishlists/${id}/allowed-users`, { method: 'PUT', body: { user_ids: allowed.value } }) }
    catch (e) { msg.value = ''; err.value = errMsg(e) }
  }
}
async function publish() {
  pubBusy.value = true; pubErrs.value = []
  if (await patchList({ status: 'active' }, t('lists.edit.published'))) { showShare.value = true; tour.refresh() }
  pubBusy.value = false
}
async function remove(force = false) {
  delBusy.value = true; delErr.value = ''
  try { await api(`/items/${del.value.id}`, { method: 'DELETE', query: force ? { force: true } : {} }); del.value = null; await load(true) }
  catch (e: any) {
    // 眾籌品項已達標（點數已扣款、進入採購）時 force 也刪不掉：顯示後端原因，不提供「一併刪除」
    const funded = del.value.funding_mode === 'crowdfund' && del.value.funding_status && del.value.funding_status !== 'open'
    if (e.code === 'ITEM_HAS_CLAIMS' && !force && !funded) delErr.value = t('lists.edit.errHasClaims')
    else delErr.value = errMsg(e)
    if (e.code === 'ITEM_HAS_CLAIMS' && !funded) del.value.force = true
  } finally { delBusy.value = false }
}
// 連按排序時依序送出：並行的請求可能晚到先寫，後端留下的順序會與畫面不同
let reorderQ: Promise<unknown> = Promise.resolve()
function move(i: number, d: number) {
  const a = [...items.value]; const j = i + d
  if (j < 0 || j >= a.length) return
  ;[a[i], a[j]] = [a[j], a[i]]
  items.value = a
  reorderQ = reorderQ.then(() => api(`/wishlists/${id}/items/reorder`, { method: 'POST', body: { item_ids: a.map(x => x.id) } }))
    .catch(async (e) => { err.value = errMsg(e); await load(true) })
}
async function copy() {
  try { await copyText(shareUrl.value); copied.value = true; setTimeout(() => (copied.value = false), 2000) }
  catch { (document.getElementById('c-share-input') as HTMLInputElement)?.select() }
}
const canInvite = computed(() => ['friends', 'selected'].includes(w.value?.visibility) && w.value?.status !== 'draft')
// 可見度變更 → 舊邀請作廢（後端也會依清單撤銷），重置開關與快取
watch(() => w.value?.visibility, () => { invite.value = null; inviteOn.value = false; inviteErr.value = '' })
async function toggleInvite() {
  inviteErr.value = ''
  if (!inviteOn.value || invite.value) return // 已有快取就不再建新權杖
  inviteBusy.value = true
  try { invite.value = await api('/friends/invites', { method: 'POST', body: { wishlist_id: id } }) }
  catch (e) { inviteOn.value = false; inviteErr.value = errMsg(e) } finally { inviteBusy.value = false }
}
const shareText = computed(() => `${w.value?.title ?? ''} ${shareUrl.value}`)
async function nativeShare() {
  try { await navigator.share({ title: w.value?.title ?? '', text: w.value?.title ?? '', url: shareUrl.value }) }
  catch (e: any) { if (e?.name !== 'AbortError') await copy() }
}
const mailHref = computed(() => `mailto:?subject=${encodeURIComponent(w.value?.title ?? '')}&body=${encodeURIComponent(shareText.value)}`)
const waHref = computed(() => `https://wa.me/?text=${encodeURIComponent(shareText.value)}`)
const threadsHref = computed(() => `https://www.threads.net/intent/post?text=${encodeURIComponent(shareText.value)}`)
const lineHref = computed(() => `https://line.me/R/share?text=${encodeURIComponent(shareText.value)}`)
const prio = (p: string) => ['high', 'medium', 'low'].includes(p) ? t(`lists.edit.pri.${p}`) : ''
const closed = computed(() => w.value && ['closed', 'archived'].includes(w.value.status))
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="w?.title || $t('lists.edit.pageTitle')" back="/dashboard">
      <CreatorStatusBadge v-if="w" :v="w.status" />
      <NuxtLink :to="`/lists/${id}/progress`" class="c-btn">{{ $t('lists.common.progress') }}</NuxtLink>
    </CreatorHeader>
    <p v-if="loading" class="c-mute" role="status">{{ $t('lists.common.loading') }}</p>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ stale ? $t('lists.common.reload') : $t('lists.common.retry') }}</button></p>
    <template v-if="w">
      <div v-if="hidden" class="c-card bad c-err" role="alert">{{ $t('lists.edit.hidden', { reason: w.moderation_reason ? $t('lists.common.reasonSuffix', { reason: w.moderation_reason }) : '' }) }} <SupportLink :topic="w.title" /></div>
      <p v-if="closed" class="c-mute">{{ $t('lists.edit.closedMsg', { state: w.status === 'closed' ? $t('lists.edit.stateClosed') : $t('lists.edit.stateArchived') }) }} <button v-if="w.status === 'archived'" class="c-btn" @click="patchList({ status: 'closed' })">{{ $t('lists.edit.unarchive') }}</button></p>
      <ul v-if="pubErrs.length" class="c-err"><li v-for="x in pubErrs" :key="x">{{ x }}</li></ul>

      <div class="c-split"><div>
      <p v-if="!items.length && !loading" class="x-empty">{{ $t('lists.edit.noItems') }}</p>
      <div class="c-grid">
      <article v-for="(i, k) in items" :key="i.id" class="c-card ed-item">
        <span class="ed-n" aria-hidden="true">{{ String(k + 1).padStart(2, '0') }}</span>
        <div class="c-row">
          <img v-if="i.image_url" class="c-thumb" :src="i.image_url" alt="">
          <div v-else class="c-thumb">{{ i.image_status === 'pending' ? $t('lists.edit.processing') : $t('creator.image.none') }}</div>
          <div class="c-grow">
            <strong>{{ i.title }}</strong>
            <div v-if="i.funding_mode === 'support'" class="c-mute">{{ $t('lists.edit.supportLine', { name: i.support?.target?.title ?? $t('lists.edit.supportEnded'), points: i.support?.points ?? '–', qty: i.support?.qty ?? '–' }) }}{{ $t('lists.edit.priority', { p: prio(i.priority) }) }}</div>
            <div v-else-if="i.funding_mode === 'crowdfund'" class="c-mute">{{ $t('lists.edit.crowdLine', { pledged: fmtPts(i.pledged_points), target: fmtPts(i.target_points) }) }}<template v-if="i.funding_deadline">{{ $t('lists.edit.until', { time: fmtTime(i.funding_deadline) }) }}</template>{{ $t('lists.edit.priority', { p: prio(i.priority) }) }}</div>
            <div v-else class="c-mute">{{ $t('lists.edit.qtyLine', { n: i.qty_needed }) }}{{ $t('lists.edit.priority', { p: prio(i.priority) }) }}</div>
            <div v-if="relief" class="c-mute">{{ $t(`org.relief.category.${i.category}`) }} · {{ $t(`org.relief.urgency.${i.urgency}`) }} <template v-if="i.funding_mode !== 'crowdfund'"> · {{ $t('org.relief.claimed', { n: i.qty_claimed ?? 0 }) }}</template> · {{ $t('org.relief.received', { n: i.qty_received ?? 0 }) }}</div>
            <CreatorStatusBadge v-if="i.image_status === 'pending' || i.image_status === 'rejected'" :v="i.image_status" />
            <div v-if="i.image_status === 'rejected'" class="c-err">{{ $t('lists.edit.imgRejected') }}</div>
          </div>
        </div>
        <div v-if="!closed" class="c-row wrap c-mt ed-act">
          <button class="c-btn" :disabled="k === 0" :aria-label="$t('lists.edit.up')" @click="move(k, -1)">↑</button>
          <button class="c-btn" :disabled="k === items.length - 1" :aria-label="$t('lists.edit.down')" @click="move(k, 1)">↓</button>
          <button class="c-btn c-grow" @click="openItem(i)">{{ $t('lists.common.edit') }}</button>
          <button v-if="relief" class="c-btn" @click="openReceipt(i)">{{ $t('org.relief.receiptBtn') }}</button>
          <button class="c-btn danger" @click="del = { ...i }; delErr = ''">{{ $t('lists.edit.delete') }}</button>
        </div>
      </article>
      </div>
      <button v-if="!closed" class="c-btn block c-mb16 ed-add" @click="openItem()">{{ $t('lists.edit.addItem') }}</button>
      </div>
      <aside>

      <section v-if="relief" class="c-card">
        <h2 class="c-h2">{{ $t('org.relief.site') }}</h2>
        <p class="c-mute">{{ $t('org.relief.siteAddr') }}</p><p><a :href="mapsUrl(w.site_address)" target="_blank" rel="noopener">{{ w.site_address }}</a></p>
        <label class="c-field"><span>{{ $t('org.relief.permitLabel') }}</span>
          <select name="permit_id" :value="w.permit_id" @change="patchList({ permit_id: ($event.target as HTMLSelectElement).value })"><option v-for="p in permitOpts" :key="p.id" :value="p.id">{{ p.permit_no }} · {{ p.valid_from }} - {{ p.valid_to }}{{ p.valid_to < taipeiToday() ? ` (${$t('org.relief.permitExpiredOpt')})` : '' }}</option></select>
          <small class="c-mute">{{ $t('org.relief.permitResumeHint') }}</small>
        </label>
        <label class="c-field"><span>{{ $t('org.relief.siteStatusLabel') }}</span>
          <select name="site_status" :value="w.site_status" @change="patchList({ site_status: ($event.target as HTMLSelectElement).value })"><option v-for="s in ['active', 'paused', 'closed']" :key="s" :value="s">{{ $t(`org.relief.siteStatus.${s}`) }}</option></select>
        </label>
      </section>
      <details v-if="!relief" ref="addrEl" class="c-card ed-side" :open="addrOpen" @toggle="addrOpen = ($event.target as HTMLDetailsElement).open">
        <summary>{{ $t('lists.edit.shipSummary') }}<span v-if="w.has_shipping_address" class="c-badge active">{{ $t('lists.edit.filled') }}</span></summary>
        <CreatorShippingForm :wishlist-id="id" @saved="w.has_shipping_address = !!$event.has_shipping_address" />
      </details>

      <!-- 封存後只能取消封存（後端拒絕其他修改），不顯示設定與分享 -->
      <details v-if="w.status !== 'archived'" class="c-card ed-side">
        <summary>{{ $t('lists.edit.settings') }}</summary>
        <form class="c-mt12" @submit.prevent="saveMeta">
          <label class="c-field"><span>{{ $t('lists.edit.name') }}</span><input v-model="meta.title" maxlength="100" required></label>
          <label class="c-field"><span>{{ $t('lists.edit.desc') }}</span><textarea v-model="meta.description" rows="2" /></label>
          <label class="c-field"><span>{{ $t('lists.edit.event') }}</span><input v-model="meta.event_date" type="date"></label>
          <label class="c-field"><span>{{ $t('lists.edit.closes') }}</span><input v-model="meta.closes_at" type="datetime-local"></label>
          <CreatorCoverPicker v-model:preset="coverPreset" v-model:image-key="coverKey" :current-url="w.cover_image_url" @busy="coverBusy = $event" />
          <CreatorTtlPicker v-model="meta.claim_ttl_hours" />
          <label v-if="!relief" class="c-switch"><input v-model="meta.show_claimer_names" type="checkbox"><span>{{ $t('lists.edit.showNames') }}</span></label>
          <CreatorVisibilityPicker v-if="!relief" v-model:visibility="meta.visibility" v-model:users="allowed" />
          <CreatorBirthdayNotify v-if="!relief && !w.org_id" v-model="meta.birthday_notify" :visibility="meta.visibility" />
          <p class="c-mute">{{ $t('lists.edit.surprise', { state: w.surprise_mode ? (w.surprise_locked ? $t('lists.edit.surpriseLocked') : $t('lists.edit.surpriseUnlocked')) : $t('lists.edit.surpriseOff') }) }}</p>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn block" :disabled="coverBusy">{{ $t('lists.edit.saveSettings') }}</button>
        </form>
        <div class="c-row c-mt">
          <button v-if="w.status === 'active'" class="c-btn c-grow" @click="patchList({ status: 'closed' }, $t('lists.edit.ended'))">{{ $t('lists.edit.endList') }}</button>
          <button v-if="w.status === 'closed'" class="c-btn c-grow" @click="patchList({ status: 'active' }, $t('lists.edit.reopened'))">{{ $t('lists.edit.reopen') }}</button>
        </div>
      </details>
      </aside></div>

      <div class="c-bottom"><div>
        <button v-if="w.status === 'draft'" class="c-btn primary c-grow" :disabled="pubBusy || !items.length" @click="publish">{{ pubBusy ? $t('lists.edit.publishing') : $t('lists.edit.publish') }}</button>
        <button v-else-if="w.status !== 'archived'" class="c-btn primary c-grow" :disabled="hidden" @click="showShare = true">{{ $t('lists.edit.share') }}</button>
      </div></div>
    </template>

    <CreatorItemSheet :open="sheet" :wishlist-id="id" :item="editing" :prefill="prefill" :relief="relief" @close="sheet = false" @saved="saved" @reload="saved" @address="openAddress" />
    <CreatorSheet :open="!!rcpt" :title="$t('org.relief.receiptTitle')" @close="rcpt = null">
      <form v-if="rcpt" @submit.prevent="saveReceipt">
        <p class="rc-head"><strong>{{ rcpt.title }}</strong><span class="c-mute">　{{ $t('org.relief.receiptTotal', { n: receipts[rcpt.id]?.total ?? 0 }) }}</span></p>
        <ul class="c-mute rc-hist"><li v-for="(e, k) in receipts[rcpt.id]?.entries ?? []" :key="k">+{{ e.qty }} {{ e.note }} {{ $t('org.relief.receiptBy', { name: e.recorded_by_name, time: fmtTime(e.created_at) }) }}</li></ul>
        <label class="c-field"><span>{{ $t('org.relief.receiptQty') }}</span><input v-model="rcptF.qty" name="qty" type="number" inputmode="numeric" min="1" required></label>
        <label class="c-field"><span>{{ $t('org.relief.receiptNote') }}</span><input v-model="rcptF.note" name="note" maxlength="200"></label>
        <p v-if="rcptErr" class="c-err" role="alert">{{ rcptErr }}</p>
        <button class="c-btn primary block" :disabled="rcptBusy">{{ $t('org.relief.receiptSave') }}</button>
      </form>
    </CreatorSheet>
    <CreatorConfirm :open="!!del" :title="$t('lists.edit.delTitle')" :text="$t('lists.edit.delText', { title: del?.title })" :ok="del?.force ? $t('lists.edit.delForce') : $t('lists.edit.delOk')" danger :busy="delBusy" :error="delErr" @close="del = null" @ok="remove(!!del?.force)" />
    <CreatorSheet :open="showShare" :title="$t('lists.edit.shareTitle')" @close="showShare = false">
      <p v-if="w?.status === 'draft'" class="c-err">{{ $t('lists.edit.draftWarn') }}</p>
      <p class="c-mute">{{ visDesc(w?.visibility) }}</p>
      <input id="c-share-input" class="c-input c-mb" :value="shareUrl" readonly @focus="($event.target as HTMLInputElement).select()">
      <label class="c-switch"><input v-model="external" type="checkbox"><span>{{ $t('lists.edit.external') }}</span></label>
      <template v-if="canInvite">
        <label class="c-switch"><input v-model="inviteOn" type="checkbox" :disabled="inviteBusy" @change="toggleInvite"><span>{{ $t('lists.edit.invite') }}</span></label>
        <p class="c-mute">{{ $t(`lists.edit.invite_${w.visibility}`) }}</p>
        <p v-if="inviteErr" class="c-err" role="alert">{{ inviteErr }}</p>
      </template>
      <div class="c-row wrap">
        <button v-if="canNativeShare" class="c-btn primary c-grow" @click="nativeShare">{{ $t('lists.edit.nativeShare') }}</button>
        <button class="c-btn c-grow" @click="copy">{{ copied ? $t('lists.edit.copied') : $t('lists.edit.copy') }}</button>
        <a class="c-btn c-grow" :href="lineHref" target="_blank" rel="noopener">{{ $t('lists.edit.line') }}</a>
        <a class="c-btn c-grow" :href="`sms:?&body=${encodeURIComponent(shareUrl)}`">{{ $t('lists.edit.sms') }}</a>
        <a class="c-btn c-grow" :href="mailHref">{{ $t('lists.edit.email') }}</a>
        <a class="c-btn c-grow" :href="waHref" target="_blank" rel="noopener">WhatsApp</a>
        <a class="c-btn c-grow" :href="threadsHref" target="_blank" rel="noopener">Threads</a>
        <a class="c-btn c-grow" :href="`https://www.facebook.com/sharer/sharer.php?u=${encodeURIComponent(shareUrl)}`" target="_blank" rel="noopener">Facebook</a>
      </div>
      <CreatorQr :text="shareUrl" :size="180" />
      <button class="c-btn block c-mt" @click="showShare = false">{{ $t('lists.common.close') }}</button>
    </CreatorSheet>
  </main>
</template>
<style scoped>
.ed-item{padding:16px 16px 14px 16px}
.ed-n{position:absolute;top:0;right:0;padding:6px 10px;border-left:1px solid var(--line);border-bottom:1px solid var(--line);font:600 11px/1 var(--mono);letter-spacing:.14em;color:var(--mute);background:var(--bg-alt)}
.ed-item .c-thumb{width:72px;height:72px}
.ed-item .c-grow strong{font-size:17px;line-height:1.3;display:block;padding-right:36px}
.ed-act{padding-top:12px;border-top:1px dashed var(--line);margin-top:14px}
.ed-add{margin-top:16px;border-style:dashed;min-height:52px;background:var(--bg-alt)}
.ed-side{padding:0 18px}
.ed-side>summary{justify-content:space-between;font-size:15px;letter-spacing:.02em;list-style:none}
.ed-side>summary::-webkit-details-marker{display:none}
.ed-side>summary::after{content:"+" / "";font:500 22px/1 var(--mono);color:var(--red-text);transition:transform var(--t) var(--ease)}
.ed-side[open]>summary::after{transform:rotate(45deg)}
.ed-side[open]{border-color:var(--ink-line);padding-bottom:18px}
.ed-side[open]>summary{border-bottom:1px solid var(--line);margin-bottom:6px}
@media (max-width:767px){.ed-item .c-thumb{width:60px;height:60px}}
.rc-head{display:flex;flex-wrap:wrap;align-items:baseline;gap:4px 12px;margin:0 0 12px;padding-bottom:10px;border-bottom:1px solid var(--ink-line)}
.rc-hist{list-style:none;padding:0;margin:0 0 18px;font:500 12px/1.5 var(--mono)}
.rc-hist li{padding:8px 0;border-bottom:1px dotted var(--line);overflow-wrap:anywhere}
.rc-hist li:last-child{border-bottom:0}
</style>
