<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
definePageMeta({ middleware: 'staff' })
const { t, locale } = useI18n()
useHead({ title: () => t('admin.audit.title') })
const { api } = useApi()
const filters = reactive({ actor_id: '', action: '', entity: '', entity_id: '', from: '', to: '' })
const rows = ref<any[]>([]), cursor = ref<string | null>(null), busy = ref(false), err = ref(''), done = ref(false)
const fields = computed<Record<string, string>>(() => ({ actor_id: t('admin.audit.fields.actorId'), action: t('admin.audit.fields.action'), entity: t('admin.audit.fields.entity'), entity_id: t('admin.audit.fields.entityId'), from: t('admin.audit.fields.from'), to: t('admin.audit.fields.to') }))
const dl = computed(() => (locale.value === 'zh-Hant' ? 'zh-TW' : locale.value))
async function load(more = false) {
  if (busy.value) return
  busy.value = true; err.value = ''
  try {
    const query = Object.fromEntries(Object.entries(filters).map(([k, v]) => [k, v.trim() || undefined]))
    const r = await api('/admin/audit-logs', { query: { ...query, cursor: more ? cursor.value : undefined } })
    rows.value = more ? [...rows.value, ...r.data] : r.data; cursor.value = r.next_cursor; done.value = true
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
watch(filters, () => { cursor.value = null })
onMounted(() => load())
</script>
<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.audit.title')" back="/admin" />
    <form @submit.prevent="load()">
      <div class="audit-filters">
        <label v-for="(name, key) in fields" :key="key" class="c-field"><span>{{ name }}</span><input v-model="filters[key as keyof typeof filters]" :type="key === 'from' || key === 'to' ? 'date' : 'text'" :disabled="busy" :placeholder="key === 'action' ? $t('admin.audit.exAction') : key === 'entity' ? $t('admin.audit.exEntity') : undefined"></label>
      </div>
      <p class="c-mute">{{ $t('admin.audit.note') }}</p>
      <button class="c-btn primary c-mb" :disabled="busy">{{ busy ? $t('admin.audit.querying') : $t('admin.audit.query') }}</button>
    </form>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="done && !rows.length" class="c-mute">{{ $t('admin.audit.empty') }}</p>
    <article v-for="r in rows" :key="r.id" class="c-card audit-row">
      <div class="c-row wrap"><strong class="c-grow">{{ r.action }}</strong><time class="c-mute">{{ new Date(r.created_at).toLocaleString(dl, { timeZone: 'Asia/Taipei' }) }}</time></div>
      <p>{{ $t('admin.audit.actor') }}{{ r.actor_name || r.actor_type }}<span v-if="r.actor_id">（{{ r.actor_id }}）</span></p>
      <p>{{ $t('admin.audit.entity') }}{{ r.entity }}<span v-if="r.entity_id">・{{ r.entity_id }}</span></p>
      <details v-if="r.diff"><summary>{{ $t('admin.audit.diff') }}</summary><pre>{{ JSON.stringify(r.diff, null, 2) }}</pre></details>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="busy" @click="load(true)">{{ $t('admin.audit.loadMore') }}</button>
  </main>
</template>
<style scoped>
.audit-filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:0 12px}
.audit-row{overflow-wrap:anywhere}
.audit-row pre{white-space:pre-wrap;font-size:13px}
</style>
