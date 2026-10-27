<script setup lang="ts">
const props = defineProps<{ open: boolean; wishlistId: string; item: any | null; prefill?: Record<string, any>; relief?: boolean }>()
const emit = defineEmits<{ close: []; saved: []; reload: []; address: [] }>()
const { api } = useApi()
const { t: tr } = useI18n()
const f = reactive<any>({})
const imageKey = ref<string | null>(null)
const version = ref<string | null>(null)
const upBusy = ref(false), busy = ref(false), err = ref(''), stale = ref(false), needAddr = ref(false)
// 已有捐助 / 認捐的品項不能改取得方式；眾籌達標後不能改目標與期限（後端同樣會擋）
const modeLocked = computed(() => !!props.item && (props.item.qty_claimed > 0 || props.item.pledged_points > 0))
const fundLocked = computed(() => !!props.item && props.item.funding_mode === 'crowdfund' && props.item.funding_status !== 'open')
const crowd = computed(() => f.funding_mode === 'crowdfund')
watch(() => props.open, (v) => {
  if (!v) return
  const i = props.item
  const p = i ? {} : props.prefill ?? {} // 預填只用於新增
  Object.assign(f, { title: i?.title ?? p.title ?? '', brand: i?.brand ?? p.brand ?? '', spec: i?.spec ?? p.spec ?? '', product_url: i?.product_url ?? p.product_url ?? '', qty_needed: i?.qty_needed ?? 1, unit_price_amount: i?.unit_price_amount ?? p.unit_price_amount ?? '', priority: i?.priority ?? 'medium', category: i?.category ?? '', urgency: i?.urgency ?? '',
    funding_mode: i?.funding_mode === 'crowdfund' ? 'crowdfund' : 'quantity', target_points: i?.target_points ?? '', funding_deadline: toTaipeiInput(i?.funding_deadline), price_snapshot_amount: i?.price_snapshot_amount ?? i?.unit_price_amount ?? '' })
  imageKey.value = null; err.value = ''; stale.value = false; needAddr.value = false; version.value = i?.updated_at ?? null
})
async function save() {
  err.value = ''; needAddr.value = false
  if (!f.title.trim()) { err.value = tr('creator.itemSheet.errName'); return }
  if (props.relief && (!f.category || !f.urgency)) { err.value = tr('org.relief.errRelief'); return }
  const body: any = { title: f.title.trim(), brand: f.brand || null, spec: f.spec || null, product_url: f.product_url || null, priority: f.priority }
  if (props.relief) Object.assign(body, { category: f.category, urgency: f.urgency })
  if (crowd.value) {
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
  if (!props.item || f.funding_mode !== (props.item.funding_mode ?? 'quantity')) body.funding_mode = f.funding_mode
  if (imageKey.value) body.image_key = imageKey.value
  if (!props.item && props.prefill?.catalog_ref) body.catalog_ref = props.prefill.catalog_ref // 僅新增
  busy.value = true
  try {
    if (props.item) {
      if (version.value) body.expected_updated_at = version.value
      const r = await api(`/items/${props.item.id}`, { method: 'PATCH', body })
      version.value = r?.updated_at ?? version.value // 成功後更新本地版本
    }
    else await api(`/wishlists/${props.wishlistId}/items`, { method: 'POST', body })
    emit('saved')
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
      <CreatorImageUploader v-model="imageKey" purpose="item" :current-url="item?.image_url" @busy="upBusy = $event" />
      <label class="c-field"><span>{{ $t('creator.itemSheet.name') }}</span><input v-model="f.title" maxlength="120" :placeholder="$t('creator.itemSheet.namePh')" required></label>
      <label class="c-field"><span>{{ $t('creator.itemSheet.brand') }}</span><input v-model="f.brand" maxlength="100"></label>
      <label class="c-field"><span>{{ $t('creator.itemSheet.spec') }}</span><input v-model="f.spec" maxlength="200"></label>
      <label class="c-field"><span>{{ $t('creator.itemSheet.url') }}</span><input v-model="f.product_url" type="url" maxlength="2000" placeholder="https://"></label>
      <template v-if="relief">
        <label class="c-field"><span>{{ $t('org.relief.category') }}</span>
          <select v-model="f.category" name="category" required><option value="" disabled>{{ $t('org.relief.pick') }}</option><option v-for="c in CATEGORIES" :key="c" :value="c">{{ $t(`org.relief.category.${c}`) }}</option></select></label>
        <label class="c-field"><span>{{ $t('org.relief.urgency') }}</span>
          <select v-model="f.urgency" name="urgency" required><option value="" disabled>{{ $t('org.relief.pick') }}</option><option v-for="u in URGENCIES" :key="u" :value="u">{{ $t(`org.relief.urgency.${u}`) }}</option></select></label>
      </template>
      <div class="c-field"><span>{{ $t('creator.itemSheet.mode') }}</span>
        <div class="c-seg" style="margin:0"><label v-for="m in [['quantity', $t('creator.itemSheet.modeQty')], ['crowdfund', $t('creator.itemSheet.modeCrowd')]]" :key="m[0]"><input v-model="f.funding_mode" type="radio" name="funding_mode" :value="m[0]" :disabled="modeLocked"><span>{{ m[1] }}</span></label></div>
        <p v-if="modeLocked" class="c-mute c-small">{{ $t('creator.itemSheet.modeLocked') }}</p>
      </div>
      <template v-if="crowd">
        <label class="c-field"><span>{{ $t('creator.itemSheet.targetPoints') }}{{ item?.pledged_points ? $t('creator.itemSheet.raisedSuffix', { n: item.pledged_points }) : '' }}</span><input v-model="f.target_points" type="number" inputmode="numeric" :min="(item?.pledged_points || 0) + 1" max="10000000" :disabled="fundLocked" required></label>
        <label class="c-field"><span>{{ $t('creator.itemSheet.deadline') }}</span><input v-model="f.funding_deadline" type="datetime-local" :disabled="fundLocked"><small class="c-mute">{{ $t('creator.itemSheet.deadlineHint') }}</small></label>
        <label class="c-field"><span>{{ $t('creator.itemSheet.refPrice') }}</span><input v-model="f.price_snapshot_amount" type="number" inputmode="numeric" min="0"></label>
        <p class="c-mute">{{ $t('creator.itemSheet.crowdNote') }}</p>
        <p v-if="fundLocked" class="c-mute">{{ $t('creator.itemSheet.fundLocked') }}</p>
      </template>
      <template v-else>
        <label class="c-field"><span>{{ $t('creator.itemSheet.qty') }}{{ item?.qty_claimed ? $t('creator.itemSheet.claimedSuffix', { n: item.qty_claimed }) : '' }}</span><input v-model="f.qty_needed" type="number" inputmode="numeric" :min="item?.qty_claimed || 1" max="99"></label>
        <label class="c-field"><span>{{ $t('creator.itemSheet.unitPrice') }}</span><input v-model="f.unit_price_amount" type="number" inputmode="numeric" min="0"></label>
      </template>
      <div class="c-field"><span>{{ $t('creator.itemSheet.priority') }}</span>
        <div class="c-seg"><label v-for="p in [['high', $t('creator.itemSheet.high')], ['medium', $t('creator.itemSheet.medium')], ['low', $t('creator.itemSheet.low')]]" :key="p[0]"><input v-model="f.priority" type="radio" :value="p[0]"><span>{{ p[1] }}</span></label></div>
      </div>
      <p v-if="err" class="c-err" role="alert">{{ err }} <button v-if="stale" type="button" class="c-btn" @click="emit('reload')">{{ $t('creator.itemSheet.reload') }}</button><button v-if="needAddr" type="button" class="c-btn" @click="emit('address')">{{ $t('creator.itemSheet.fillAddress') }}</button></p>
      <div class="c-row">
        <button type="button" class="c-btn c-grow" @click="emit('close')">{{ $t('creator.itemSheet.cancel') }}</button>
        <button class="c-btn primary c-grow" :disabled="busy || upBusy">{{ busy ? $t('creator.itemSheet.saving') : $t('creator.itemSheet.save') }}</button>
      </div>
    </form>
  </CreatorSheet>
</template>
