<script setup lang="ts">
definePageMeta({ middleware: 'staff' })
useHead({ title: '稽核紀錄' })
const { api } = useApi()
const filters = reactive({ actor_id: '', action: '', entity: '', entity_id: '', from: '', to: '' })
const rows = ref<any[]>([]), cursor = ref<string | null>(null), busy = ref(false), err = ref(''), done = ref(false)
const fields: Record<string, string> = { actor_id: '操作者 UUID', action: '動作', entity: '對象類型', entity_id: '對象 UUID', from: '開始日期', to: '結束日期' }
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
    <CreatorHeader title="稽核紀錄" back="/admin" />
    <form @submit.prevent="load()">
      <div class="audit-filters">
        <label v-for="(name, key) in fields" :key="key" class="c-field"><span>{{ name }}</span><input v-model="filters[key as keyof typeof filters]" :type="key === 'from' || key === 'to' ? 'date' : 'text'" :disabled="busy" :placeholder="key === 'action' ? '例如 user.update' : key === 'entity' ? '例如 users' : undefined"></label>
      </div>
      <p class="c-mute">日期以台北時區計算，包含結束日；動作與對象類型須完整匹配。</p>
      <button class="c-btn primary c-mb" :disabled="busy">{{ busy ? '查詢中…' : '查詢' }}</button>
    </form>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="done && !rows.length" class="c-mute">沒有符合的稽核紀錄</p>
    <article v-for="r in rows" :key="r.id" class="c-card audit-row">
      <div class="c-row wrap"><strong class="c-grow">{{ r.action }}</strong><time class="c-mute">{{ new Date(r.created_at).toLocaleString('zh-TW', { timeZone: 'Asia/Taipei' }) }}</time></div>
      <p>操作者：{{ r.actor_name || r.actor_type }}<span v-if="r.actor_id">（{{ r.actor_id }}）</span></p>
      <p>對象：{{ r.entity }}<span v-if="r.entity_id">・{{ r.entity_id }}</span></p>
      <details v-if="r.diff"><summary>操作內容</summary><pre>{{ JSON.stringify(r.diff, null, 2) }}</pre></details>
    </article>
    <button v-if="cursor" class="c-btn block" :disabled="busy" @click="load(true)">載入更多稽核紀錄</button>
  </main>
</template>
<style scoped>
.audit-filters{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:0 12px}
.audit-row{overflow-wrap:anywhere}
.audit-row pre{white-space:pre-wrap;font-size:13px}
</style>
