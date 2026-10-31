<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 救災事件管理（極簡）：沒有 admin 列表端點，列表用公開 GET /public/relief-events；可合併到另一事件、隱藏 / 取消隱藏
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.events.title') })
const { api } = useApi()
const rows = ref<any[]>([]), next = ref<number | null>(null), loading = ref(false)
const q = ref(''), err = ref(''), msg = ref('')
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const shown = computed(() => rows.value.filter(e => !q.value.trim() || e.id === q.value.trim() || e.slug === q.value.trim()))
// 公開列表不含已隱藏事件：輸入列表外的 UUID 時，合成一列讓 staff 直接操作
const direct = computed(() => (UUID.test(q.value.trim()) && !shown.value.length ? { id: q.value.trim(), name: q.value.trim(), slug: '', counties: [], moderation_status: 'ok', merged_into: null } : null))

async function load(more = false) {
  loading.value = true; err.value = ''
  try {
    const r = await api('/public/relief-events', { query: { page: more ? next.value : undefined } })
    rows.value = more ? [...rows.value, ...r.events] : r.events; next.value = r.next_page ?? null
  } catch (e) { err.value = errMsg(e) } finally { loading.value = false }
}
onMounted(() => load())

type Act = { row: any, kind: 'merge' | 'hide' | 'unhide' }
const act = ref<Act | null>(null)
const into = ref(''), reason = ref(''), busy = ref(false), aerr = ref('')
const title = computed(() => (act.value ? t(`admin.events.${act.value.kind}`) : ''))
const text = computed(() => act.value ? t(`admin.events.${act.value.kind === 'merge' ? 'mergeText' : act.value.kind + 'Text'}`, { name: act.value.row.name }) : '')
function open(row: any, kind: Act['kind']) { act.value = { row, kind }; into.value = ''; reason.value = ''; aerr.value = '' }
async function submit() {
  const { row, kind } = act.value!
  aerr.value = ''
  if (kind === 'hide' && !reason.value.trim()) { aerr.value = t('admin.events.reasonRequired'); return } // 後端下架原因必填
  busy.value = true
  try {
    if (kind === 'merge') {
      const v = into.value.trim()
      const target = UUID.test(v) ? v : rows.value.find(e => e.slug === v)?.id // slug 只能從已載入的列表解析
      if (!target) { aerr.value = t('admin.events.targetNotFound'); return }
      if (target === row.id) { aerr.value = t('admin.events.selfMerge'); return }
      await api(`/admin/relief-events/${row.id}/merge`, { method: 'POST', body: { into: target } })
      row.merged_into = target
    } else {
      const moderation_status = kind === 'hide' ? 'hidden' : 'ok'
      await api(`/admin/relief-events/${row.id}`, { method: 'PATCH', body: { moderation_status, moderation_reason: reason.value.trim() || null } })
      row.moderation_status = moderation_status
    }
    msg.value = t('admin.events.done'); act.value = null
  } catch (e) { aerr.value = errMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.events.title')" back="/admin" />
    <label class="c-field c-narrow"><span>{{ $t('admin.events.filterLabel') }}</span><input v-model="q"></label>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn" @click="load()">{{ $t('admin.events.retry') }}</button></p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
    <p v-if="loading && !rows.length" class="c-mute" role="status">{{ $t('admin.events.loading') }}</p>
    <p v-else-if="!shown.length && !direct && !err" class="c-center c-mute">{{ $t('admin.events.empty') }}</p>
    <article v-for="e in (direct ? [direct] : shown)" :key="e.id" class="c-card" :data-event="e.id">
      <div class="c-row"><strong class="c-grow">{{ e.name }}</strong><span v-if="e.moderation_status === 'hidden'" class="c-badge">{{ $t('admin.events.hiddenBadge') }}</span></div>
      <div v-if="e.slug" class="c-mute">{{ e.slug }} · {{ e.hazard_type }} · {{ (e.counties || []).join('、') }} · {{ e.occurred_on }}</div>
      <div class="c-mute c-small">{{ e.id }}</div>
      <p v-if="e.merged_into" class="c-mute">{{ $t('admin.events.merged', { into: e.merged_into }) }}</p>
      <div class="c-row wrap c-mt">
        <!-- 直接以 UUID 操作時不知道目前是否已隱藏，兩個方向都給 -->
        <button v-if="e === direct" class="c-btn c-grow" @click="open(e, 'unhide')">{{ $t('admin.events.unhide') }}</button>
        <button v-if="!e.merged_into" class="c-btn c-grow" @click="open(e, 'merge')">{{ $t('admin.events.merge') }}</button>
        <button class="c-btn c-grow" :class="{ danger: e.moderation_status !== 'hidden' }" @click="open(e, e.moderation_status === 'hidden' ? 'unhide' : 'hide')">{{ $t(`admin.events.${e.moderation_status === 'hidden' ? 'unhide' : 'hide'}`) }}</button>
      </div>
    </article>
    <button v-if="next" class="c-btn block" :disabled="loading" @click="load(true)">{{ $t('admin.events.loadMore') }}</button>

    <CreatorConfirm :open="!!act" :title="title" :text="text" :ok="title" :danger="act?.kind === 'hide'" :busy="busy" :error="aerr" @close="act = null" @ok="submit">
      <label v-if="act?.kind === 'merge'" class="c-field"><span>{{ $t('admin.events.mergeIntoLabel') }}</span><input v-model="into" name="into"></label>
      <label v-else class="c-field"><span>{{ $t('admin.events.reasonLabel') }}</span><input v-model="reason" maxlength="200" name="reason" :required="act?.kind === 'hide'"></label>
    </CreatorConfirm>
  </main>
</template>
