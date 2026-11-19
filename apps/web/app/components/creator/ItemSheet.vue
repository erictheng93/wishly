<script setup lang="ts">
const props = defineProps<{ open: boolean; wishlistId: string; item: any | null; prefill?: Record<string, any>; relief?: boolean; eventDate?: string | null; hasAddress?: boolean }>()
const emit = defineEmits<{ close: []; saved: [keepOpen: boolean]; reload: []; address: [] }>()
const { api } = useApi()
const { t: tr } = useI18n()
const f = reactive<any>({})
const imageKey = ref<string | null>(null)
const version = ref<string | null>(null)
const upBusy = ref(false), busy = ref(false), err = ref(''), stale = ref(false), needAddr = ref(false)
const errEl = ref<HTMLElement>(), nameEl = ref<HTMLInputElement>()
// 錯誤訊息在表單底部，長表單會被黏底按鈕列蓋住：出現就捲到可視範圍中央
watch(err, v => { if (v) nextTick(() => errEl.value?.scrollIntoView({ block: 'center' })) })
const more = ref(false), again = ref(false), formKey = ref(0), catalogRef = ref<string | null>(null)
// 去填收件資訊時保留新增中的草稿；下次開啟新增就還原，不重新帶預填
let keepDraft = false
// 已有捐助 / 認捐的品項不能改取得方式；眾籌達標後不能改目標與期限（後端同樣會擋）
const modeLocked = computed(() => !!props.item && (props.item.qty_claimed > 0 || props.item.pledged_points > 0))
const fundLocked = computed(() => !!props.item && props.item.funding_mode === 'crowdfund' && props.item.funding_status !== 'open')
const crowd = computed(() => f.funding_mode === 'crowdfund')
const support = computed(() => f.funding_mode === 'support')
// 救災據點建立時就以據點地址寫入收件資訊（org_sites.rs），只有一般清單需要前置提示
const addrHint = computed(() => crowd.value && !props.relief && !props.hasAddress)
const deadlineRequired = computed(() => !!props.relief || !props.eventDate)
function toAddress() { keepDraft = !props.item; emit('address') }
// 支持品項的目標：新增時由貼上的連結帶出標題；失敗就停用儲存
const supLink = ref(''), supSlug = ref<string | null>(null), supTitle = ref('')
async function checkTarget() {
  supSlug.value = null; supTitle.value = ''
  const slug = parseListSlug(supLink.value)
  if (!slug) { err.value = supLink.value.trim() ? tr('creator.itemSheet.supportBad') : ''; return }
  try {
    const r = await api(`/public/wishlists/${slug}`)
    if (!isCurrentSlug(supLink.value, slug)) return // 輸入已改，丟棄過期回應
    supSlug.value = slug; supTitle.value = r?.title ?? ''; err.value = ''
    if (!f.title.trim() && r?.title) f.title = r.title
  } catch { if (isCurrentSlug(supLink.value, slug)) err.value = tr('creator.itemSheet.supportBad') }
}
function fill(i: any, p: Record<string, any>) {
  Object.assign(f, { title: i?.title ?? p.title ?? '', brand: i?.brand ?? p.brand ?? '', spec: i?.spec ?? p.spec ?? '', product_url: i?.product_url ?? p.product_url ?? '', qty_needed: i?.qty_needed ?? 1, unit_price_amount: i?.unit_price_amount ?? p.unit_price_amount ?? '', priority: i?.priority ?? 'medium', category: i?.category ?? '', urgency: i?.urgency ?? '',
    funding_mode: ['crowdfund', 'support'].includes(i?.funding_mode) ? i.funding_mode : 'quantity', target_points: i?.target_points ?? '', funding_deadline: toTaipeiInput(i?.funding_deadline), price_snapshot_amount: i?.price_snapshot_amount ?? i?.unit_price_amount ?? '' })
  supLink.value = ''; supSlug.value = null; supTitle.value = ''; catalogRef.value = p.catalog_ref ?? null
  imageKey.value = null; err.value = ''; stale.value = false; needAddr.value = false; version.value = i?.updated_at ?? null
  more.value = !!(f.brand || f.spec || f.priority !== 'medium') // 次要欄位有非預設值就展開
  formKey.value++ // 重建圖片元件，清掉上一筆的預覽
}
watch(() => props.open, (v) => {
  if (!v) return
  const draft = keepDraft && !props.item
  keepDraft = false
  if (draft) { err.value = ''; needAddr.value = false; return }
  fill(props.item, props.item ? {} : props.prefill ?? {}) // 預填只用於新增
})
async function save() {
  err.value = ''; needAddr.value = false
  if (!f.title.trim()) { err.value = tr('creator.itemSheet.errName'); return }
  if (props.relief && (!f.category || !f.urgency)) { err.value = tr('org.relief.errRelief'); return }
  if (support.value && !props.item && !supSlug.value) { err.value = tr('creator.itemSheet.supportBad'); return }
  const body: any = { title: f.title.trim(), brand: f.brand || null, spec: f.spec || null, product_url: f.product_url || null, priority: f.priority }
  if (props.relief) Object.assign(body, { category: f.category, urgency: f.urgency })
  if (support.value) {
    for (const k of ['brand', 'spec', 'product_url']) delete body[k]
    if (!props.item) Object.assign(body, { funding_mode: 'support', support_slug: supSlug.value })
  } else if (crowd.value) {
    const t = Number(f.target_points)
    if (!Number.isInteger(t) || t < 1) { err.value = tr('creator.itemSheet.errTarget'); return }
    if (!fundLocked.value) {
      body.target_points = t
      if (f.funding_deadline) body.funding_deadline = fromTaipeiInput(f.funding_deadline) // 留空：後端用活動日 23:59（台北）
    }
    body.price_snapshot_amount = f.price_snapshot_amount === '' ? null : Number(f.price_snapshot_amount)
  } else {
    body.qty_needed = Number(f.qty_needed) || 1
    body.unit_price_amount = f.unit_price_amount === '' ? null : Number(f.unit_price_amount)
  }
  if (!support.value && (!props.item || f.funding_mode !== (props.item.funding_mode ?? 'quantity'))) body.funding_mode = f.funding_mode
  if (imageKey.value && !support.value) body.image_key = imageKey.value
  if (!props.item && catalogRef.value) body.catalog_ref = catalogRef.value // 僅新增
  busy.value = true
  try {
    if (props.item) {
      if (version.value) body.expected_updated_at = version.value
      const r = await api(`/items/${props.item.id}`, { method: 'PATCH', body })
      version.value = r?.updated_at ?? version.value // 成功後更新本地版本
    }
    else await api(`/wishlists/${props.wishlistId}/items`, { method: 'POST', body })
    const next = !props.item && again.value
    if (next) { // 連續新增：清空表單但保留取得方式，焦點回名稱
      const mode = f.funding_mode
      fill(null, {}); f.funding_mode = mode
      nextTick(() => nameEl.value?.focus())
    }
    emit('saved', next)
  } catch (e: any) {
    if (e.code === 'STALE_VERSION') { stale.value = true; err.value = tr('creator.itemSheet.errStale') }
    else if (e.code === 'SHIPPING_ADDRESS_REQUIRED') { needAddr.value = true; err.value = tr('creator.itemSheet.errAddr') }
    else err.value = errMsg(e)
  } finally { busy.value = false }
}
</script>
<template>
  <CreatorSheet :open="open" :title="item ? $t('creator.itemSheet.titleEdit') : $t('creator.itemSheet.titleNew')" @close="emit('close')">
    <form @submit.prevent="save">
      <label class="c-field"><span>{{ $t('creator.itemSheet.name') }}</span><input ref="nameEl" v-model="f.title" maxlength="120" :placeholder="$t('creator.itemSheet.namePh')" required></label>
      <template v-if="!support">
      <label class="c-field"><span>{{ $t('creator.itemSheet.url') }}</span><input v-model="f.product_url" type="url" maxlength="2000" placeholder="https://"></label>
      <CreatorImageUploader :key="formKey" v-model="imageKey" purpose="item" :current-url="item?.image_url" @busy="upBusy = $event" />
      </template>
      <template v-if="relief">
        <label class="c-field"><span>{{ $t('org.relief.categoryLabel') }}</span>
          <select v-model="f.category" name="category" required><option value="" disabled>{{ $t('org.relief.pick') }}</option><option v-for="c in CATEGORIES" :key="c" :value="c">{{ $t(`org.relief.category.${c}`) }}</option></select></label>
        <label class="c-field"><span>{{ $t('org.relief.urgencyLabel') }}</span>
          <select v-model="f.urgency" name="urgency" required><option value="" disabled>{{ $t('org.relief.pick') }}</option><option v-for="u in URGENCIES" :key="u" :value="u">{{ $t(`org.relief.urgency.${u}`) }}</option></select></label>
      </template>
      <div class="c-field"><span>{{ $t('creator.itemSheet.mode') }}</span>
        <div class="c-seg" style="margin:0" role="radiogroup" :aria-label="$t('creator.itemSheet.mode')"><label v-for="m in [['quantity', $t('creator.itemSheet.modeQty')], ['crowdfund', $t('creator.itemSheet.modeCrowd')], ...(item && !support ? [] : [['support', $t('creator.itemSheet.modeSupport')]])]" :key="m[0]"><input v-model="f.funding_mode" type="radio" name="funding_mode" :value="m[0]" :disabled="modeLocked || (!!item && support)"><span>{{ m[1] }}</span></label></div>
        <p v-if="modeLocked" class="c-mute c-small">{{ $t('creator.itemSheet.modeLocked') }}</p>
        <p v-if="addrHint" class="c-mute c-small">{{ $t('creator.itemSheet.errAddr') }} <button type="button" class="c-btn" @click="toAddress">{{ $t('creator.itemSheet.fillAddress') }}</button></p>
      </div>
      <template v-if="support">
        <p v-if="item" class="c-mute">{{ $t('creator.itemSheet.supportTarget', { name: item.support?.target?.title ?? $t('creator.itemSheet.supportEnded') }) }}</p>
        <label v-else class="c-field"><span>{{ $t('creator.itemSheet.supportLink') }}</span><input v-model="supLink" maxlength="300" inputmode="url" autocapitalize="none" :spellcheck="false" placeholder="https://wishly.tw/s/…" @input="supSlug = null; supTitle = ''" @keydown.enter.prevent="checkTarget" @blur="checkTarget"></label>
        <p v-if="!item && supTitle" class="c-mute">{{ $t('creator.itemSheet.supportTarget', { name: supTitle }) }}</p>
        <p class="c-mute">{{ $t('creator.itemSheet.supportNote') }}</p>
      </template>
      <template v-else-if="crowd">
        <label class="c-field"><span>{{ $t('creator.itemSheet.targetPoints') }}{{ item?.pledged_points ? $t('creator.itemSheet.raisedSuffix', { n: item.pledged_points }) : '' }}</span><input v-model="f.target_points" type="number" inputmode="numeric" :min="(item?.pledged_points || 0) + 1" max="10000000" :disabled="fundLocked" required></label>
        <!-- 救災據點與沒有活動日的清單，截止時間必填（後端 deadline_of 同樣要求） -->
        <label class="c-field"><span>{{ relief ? $t('org.relief.deadlineLabel') : deadlineRequired ? $t('creator.itemSheet.deadlineRequired') : $t('creator.itemSheet.deadline') }}</span><input v-model="f.funding_deadline" type="datetime-local" :disabled="fundLocked" :required="deadlineRequired && !fundLocked"><small v-if="!relief" class="c-mute">{{ deadlineRequired ? $t('creator.itemSheet.deadlineHintNoEvent') : $t('creator.itemSheet.deadlineHint') }}</small></label>
        <label class="c-field"><span>{{ $t('creator.itemSheet.refPrice') }}</span><input v-model="f.price_snapshot_amount" type="number" inputmode="numeric" min="0"></label>
        <p class="c-mute">{{ $t('creator.itemSheet.crowdNote') }}</p>
        <p v-if="fundLocked" class="c-mute">{{ $t('creator.itemSheet.fundLocked') }}</p>
      </template>
      <template v-else>
        <div class="is-pair">
        <label class="c-field"><span>{{ $t('creator.itemSheet.qty') }}{{ item?.qty_claimed ? $t('creator.itemSheet.claimedSuffix', { n: item.qty_claimed }) : '' }}</span><input v-model="f.qty_needed" type="number" inputmode="numeric" :min="item?.qty_claimed || 1" max="99"></label>
        <label class="c-field"><span>{{ $t('creator.itemSheet.unitPrice') }}</span><input v-model="f.unit_price_amount" type="number" inputmode="numeric" min="0"></label>
        </div>
      </template>
      <details class="is-more" :open="more" @toggle="more = ($event.target as HTMLDetailsElement).open">
        <summary>{{ $t('creator.itemSheet.more') }}<UiIcon class="is-plus" name="plus" :size="18" /></summary>
        <template v-if="!support">
        <label class="c-field"><span>{{ $t('creator.itemSheet.brand') }}</span><input v-model="f.brand" maxlength="100"></label>
        <label class="c-field"><span>{{ $t('creator.itemSheet.spec') }}</span><input v-model="f.spec" maxlength="200"></label>
        </template>
        <div class="c-field"><span>{{ $t('creator.itemSheet.priority') }}</span>
          <div class="c-seg" role="radiogroup" :aria-label="$t('creator.itemSheet.priority')"><label v-for="p in [['high', $t('creator.itemSheet.high')], ['medium', $t('creator.itemSheet.medium')], ['low', $t('creator.itemSheet.low')]]" :key="p[0]"><input v-model="f.priority" type="radio" :value="p[0]"><span>{{ p[1] }}</span></label></div>
        </div>
      </details>
      <!-- 前置提示已有「填寫收件資訊」鈕時不重複 -->
      <p v-if="err" ref="errEl" class="c-err" role="alert">{{ err }} <button v-if="stale" type="button" class="c-btn" @click="emit('reload')">{{ $t('creator.itemSheet.reload') }}</button><button v-if="needAddr && !addrHint" type="button" class="c-btn" @click="toAddress">{{ $t('creator.itemSheet.fillAddress') }}</button></p>
      <label v-if="!item" class="c-switch"><input v-model="again" type="checkbox"><span>{{ $t('creator.itemSheet.again') }}</span></label>
      <div class="c-row">
        <button type="button" class="c-btn c-grow" @click="emit('close')">{{ $t('creator.itemSheet.cancel') }}</button>
        <button class="c-btn primary c-grow" :disabled="busy || upBusy || (support && !item && !supSlug)">{{ busy ? $t('creator.itemSheet.saving') : $t('creator.itemSheet.save') }}</button>
      </div>
    </form>
  </CreatorSheet>
</template>
<style scoped>
.is-pair{display:grid;grid-template-columns:1fr 1fr;gap:0 12px}
.is-more{margin:0 0 12px;border-top:1px solid var(--line)}
.is-more>summary{display:flex;align-items:center;min-height:44px;cursor:pointer;font-weight:700;list-style:none}
.is-more>summary::-webkit-details-marker{display:none}
.is-plus{margin-left:auto;color:var(--red-text);transition:transform var(--t) var(--ease)}
.is-more[open]>summary .is-plus{transform:rotate(45deg)}
</style>
