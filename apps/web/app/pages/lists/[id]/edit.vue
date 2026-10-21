<script setup lang="ts">
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
const external = ref(false), copied = ref(false), showShare = ref(false)
const addrOpen = ref(false), addrEl = ref<HTMLElement>()
function openAddress() { sheet.value = false; addrOpen.value = true; nextTick(() => addrEl.value?.scrollIntoView({ block: 'center' })) }
const meta = reactive<any>({ title: '', description: '', event_date: '', closes_at: '', show_claimer_names: false, visibility: 'friends' })
const allowed = ref<string[]>([])
let poll: any

async function load(quiet = false) {
  if (!quiet) loading.value = true
  try {
    const r = await api(`/wishlists/${id}`)
    w.value = r.wishlist; items.value = r.items
    Object.assign(meta, { title: r.wishlist.title, description: r.wishlist.description ?? '', event_date: r.wishlist.event_date ?? '', closes_at: toLocalInput(r.wishlist.closes_at), show_claimer_names: r.wishlist.show_claimer_names, visibility: r.wishlist.visibility })
    if (r.wishlist.visibility === 'selected') allowed.value = (await api(`/wishlists/${id}/allowed-users`).catch(() => ({ users: [] }))).users.map((u: any) => u.id)
    err.value = ''; stale.value = false
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
  clearTimeout(poll)
  if (items.value.some(i => i.image_status === 'pending')) poll = setTimeout(() => load(true), 4000) // 圖片處理中輪詢
}
onMounted(async () => {
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
  const u = w.value.share_url || `${siteOrigin()}/s/${w.value.slug}`
  return external.value ? u + '?openExternalBrowser=1' : u
})
function openItem(i: any = null, p: ItemPrefill = {}) { editing.value = i; prefill.value = p; sheet.value = true }
async function saved() { sheet.value = false; await load(true) }
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
  const body: any = { title: meta.title.trim(), description: meta.description || null, event_date: meta.event_date || null, closes_at: closesAtChange(meta.closes_at, w.value?.closes_at), show_claimer_names: meta.show_claimer_names, visibility: meta.visibility }
  if (!await patchList(body)) return
  if (meta.visibility === 'selected') {
    try { await api(`/wishlists/${id}/allowed-users`, { method: 'PUT', body: { user_ids: allowed.value } }) }
    catch (e) { msg.value = ''; err.value = errMsg(e) }
  }
}
async function publish() {
  pubBusy.value = true; pubErrs.value = []
  if (await patchList({ status: 'active' }, t('lists.edit.published'))) showShare.value = true
  pubBusy.value = false
}
async function remove(force = false) {
  delBusy.value = true; delErr.value = ''
  try { await api(`/items/${del.value.id}`, { method: 'DELETE', query: force ? { force: true } : {} }); del.value = null; await load(true) }
  catch (e: any) {
    if (e.code === 'ITEM_HAS_CLAIMS' && !force) delErr.value = t('lists.edit.errHasClaims')
    else delErr.value = errMsg(e)
    if (e.code === 'ITEM_HAS_CLAIMS') del.value.force = true
  } finally { delBusy.value = false }
}
async function move(i: number, d: number) {
  const a = [...items.value]; const j = i + d
  if (j < 0 || j >= a.length) return
  ;[a[i], a[j]] = [a[j], a[i]]
  items.value = a
  try { await api(`/wishlists/${id}/items/reorder`, { method: 'POST', body: { item_ids: a.map(x => x.id) } }) }
  catch (e) { err.value = errMsg(e); await load(true) }
}
async function copy() {
  try { await copyText(shareUrl.value); copied.value = true; setTimeout(() => (copied.value = false), 2000) }
  catch { (document.getElementById('c-share-input') as HTMLInputElement)?.select() }
}
const lineHref = computed(() => `https://line.me/R/share?text=${encodeURIComponent(`${w.value?.title ?? ''} ${shareUrl.value}`)}`)
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
      <div v-if="hidden" class="c-card bad c-err" role="alert">{{ $t('lists.edit.hidden', { reason: w.moderation_reason ? $t('lists.common.reasonSuffix', { reason: w.moderation_reason }) : '' }) }}</div>
      <p v-if="closed" class="c-mute">{{ $t('lists.edit.closedMsg', { state: w.status === 'closed' ? $t('lists.edit.stateClosed') : $t('lists.edit.stateArchived') }) }} <button v-if="w.status === 'archived'" class="c-btn" @click="patchList({ status: 'closed' })">{{ $t('lists.edit.unarchive') }}</button></p>
      <ul v-if="pubErrs.length" class="c-err"><li v-for="x in pubErrs" :key="x">{{ x }}</li></ul>

      <div class="c-split"><div>
      <p v-if="!items.length && !loading" class="c-center c-mute">{{ $t('lists.edit.noItems') }}</p>
      <div class="c-grid">
      <article v-for="(i, k) in items" :key="i.id" class="c-card">
        <div class="c-row">
          <img v-if="i.image_url" class="c-thumb" :src="i.image_url" alt="">
          <div v-else class="c-thumb">{{ i.image_status === 'pending' ? $t('lists.edit.processing') : $t('creator.image.none') }}</div>
          <div class="c-grow">
            <strong>{{ i.title }}</strong>
            <div v-if="i.funding_mode === 'crowdfund'" class="c-mute">{{ $t('lists.edit.crowdLine', { pledged: fmtPts(i.pledged_points), target: fmtPts(i.target_points) }) }}<template v-if="i.funding_deadline">{{ $t('lists.edit.until', { time: fmtTime(i.funding_deadline) }) }}</template>{{ $t('lists.edit.priority', { p: prio(i.priority) }) }}</div>
            <div v-else class="c-mute">{{ $t('lists.edit.qtyLine', { n: i.qty_needed }) }}{{ $t('lists.edit.priority', { p: prio(i.priority) }) }}</div>
            <CreatorStatusBadge v-if="i.image_status === 'pending' || i.image_status === 'rejected'" :v="i.image_status" />
            <div v-if="i.image_status === 'rejected'" class="c-err">{{ $t('lists.edit.imgRejected') }}</div>
          </div>
        </div>
        <div v-if="!closed" class="c-row wrap c-mt">
          <button class="c-btn" :disabled="k === 0" :aria-label="$t('lists.edit.up')" @click="move(k, -1)">↑</button>
          <button class="c-btn" :disabled="k === items.length - 1" :aria-label="$t('lists.edit.down')" @click="move(k, 1)">↓</button>
          <button class="c-btn c-grow" @click="openItem(i)">{{ $t('lists.common.edit') }}</button>
          <button class="c-btn danger" @click="del = { ...i }; delErr = ''">{{ $t('lists.edit.delete') }}</button>
        </div>
      </article>
      </div>
      <button v-if="!closed" class="c-btn block c-mb16" @click="openItem()">{{ $t('lists.edit.addItem') }}</button>
      </div>
      <aside>

      <details ref="addrEl" class="c-card" :open="addrOpen" @toggle="addrOpen = ($event.target as HTMLDetailsElement).open">
        <summary>{{ $t('lists.edit.shipSummary') }}<span v-if="w.has_shipping_address" class="c-badge active">{{ $t('lists.edit.filled') }}</span></summary>
        <CreatorShippingForm :wishlist-id="id" @saved="w.has_shipping_address = !!$event.has_shipping_address" />
      </details>

      <details class="c-card">
        <summary>{{ $t('lists.edit.settings') }}</summary>
        <form class="c-mt12" @submit.prevent="saveMeta">
          <label class="c-field"><span>{{ $t('lists.edit.name') }}</span><input v-model="meta.title" maxlength="100" required></label>
          <label class="c-field"><span>{{ $t('lists.edit.desc') }}</span><textarea v-model="meta.description" rows="2" /></label>
          <label class="c-field"><span>{{ $t('lists.edit.event') }}</span><input v-model="meta.event_date" type="date"></label>
          <label class="c-field"><span>{{ $t('lists.edit.closes') }}</span><input v-model="meta.closes_at" type="datetime-local"></label>
          <label class="c-switch"><input v-model="meta.show_claimer_names" type="checkbox"><span>{{ $t('lists.edit.showNames') }}</span></label>
          <CreatorVisibilityPicker v-model:visibility="meta.visibility" v-model:users="allowed" />
          <p class="c-mute">{{ $t('lists.edit.surprise', { state: w.surprise_mode ? (w.surprise_locked ? $t('lists.edit.surpriseLocked') : $t('lists.edit.surpriseUnlocked')) : $t('lists.edit.surpriseOff') }) }}</p>
          <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
          <button class="c-btn block">{{ $t('lists.edit.saveSettings') }}</button>
        </form>
        <div class="c-row c-mt">
          <button v-if="w.status === 'active'" class="c-btn c-grow" @click="patchList({ status: 'closed' }, $t('lists.edit.ended'))">{{ $t('lists.edit.endList') }}</button>
          <button v-if="w.status === 'closed'" class="c-btn c-grow" @click="patchList({ status: 'active' }, $t('lists.edit.reopened'))">{{ $t('lists.edit.reopen') }}</button>
        </div>
      </details>
      </aside></div>

      <div class="c-bottom"><div>
        <button v-if="w.status === 'draft'" class="c-btn primary c-grow" :disabled="pubBusy || !items.length" @click="publish">{{ pubBusy ? $t('lists.edit.publishing') : $t('lists.edit.publish') }}</button>
        <button v-else class="c-btn primary c-grow" :disabled="hidden" @click="showShare = true">{{ $t('lists.edit.share') }}</button>
      </div></div>
    </template>

    <CreatorItemSheet :open="sheet" :wishlist-id="id" :item="editing" :prefill="prefill" @close="sheet = false" @saved="saved" @reload="saved" @address="openAddress" />
    <CreatorConfirm :open="!!del" :title="$t('lists.edit.delTitle')" :text="$t('lists.edit.delText', { title: del?.title })" :ok="del?.force ? $t('lists.edit.delForce') : $t('lists.edit.delOk')" danger :busy="delBusy" :error="delErr" @close="del = null" @ok="remove(!!del?.force)" />
    <CreatorSheet :open="showShare" :title="$t('lists.edit.shareTitle')" @close="showShare = false">
      <p v-if="w?.status === 'draft'" class="c-err">{{ $t('lists.edit.draftWarn') }}</p>
      <p class="c-mute">{{ visDesc(w?.visibility) }}</p>
      <input id="c-share-input" class="c-input c-mb" :value="shareUrl" readonly @focus="($event.target as HTMLInputElement).select()">
      <label class="c-switch"><input v-model="external" type="checkbox"><span>{{ $t('lists.edit.external') }}</span></label>
      <div class="c-row wrap">
        <button class="c-btn c-grow" @click="copy">{{ copied ? $t('lists.edit.copied') : $t('lists.edit.copy') }}</button>
        <a class="c-btn c-grow" :href="lineHref" target="_blank" rel="noopener">{{ $t('lists.edit.line') }}</a>
        <a class="c-btn c-grow" :href="`sms:?&body=${encodeURIComponent(shareUrl)}`">{{ $t('lists.edit.sms') }}</a>
        <a class="c-btn c-grow" :href="`https://www.facebook.com/sharer/sharer.php?u=${encodeURIComponent(shareUrl)}`" target="_blank" rel="noopener">Facebook</a>
      </div>
      <CreatorQr :text="shareUrl" :size="180" />
      <button class="c-btn block c-mt" @click="showShare = false">{{ $t('lists.common.close') }}</button>
    </CreatorSheet>
  </main>
</template>
