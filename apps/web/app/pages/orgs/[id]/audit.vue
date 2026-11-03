<script setup lang="ts">
// 組織稽核紀錄（owner / coordinator 唯讀）；游標分頁，動作文案缺譯時顯示原始代碼
definePageMeta({ middleware: 'auth' })
const { t, te } = useI18n()
useHead({ title: () => t('org.audit.title') })
const { api } = useApi()
const id = useRoute().params.id as string
const rows = ref<any[]>([])
const next = ref<string | null>(null)
const err = ref(''), busy = ref(false)
// i18n 路徑以點分層，動作代碼的點改底線才查得到
const label = (a: string) => { const k = `org.audit.a.${a.replace(/\./g, '_')}`; return te(k) ? t(k) : a }
const who = (r: any) => r.actor?.type === 'staff' ? t('org.audit.staff') : r.actor?.name || t('org.audit.system')
async function load() {
  busy.value = true; err.value = ''
  try {
    const r = await api<{ data: any[], next_cursor: string | null }>(`/orgs/${id}/audit-logs`, { query: next.value ? { cursor: next.value } : {} })
    rows.value.push(...r.data); next.value = r.next_cursor
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
onMounted(load)
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="$t('org.audit.title')" :back="`/orgs/${id}`" />
    <p v-if="err" class="c-err c-errbox" role="alert">{{ err }}</p>
    <p v-if="!rows.length && !busy && !err" class="c-mute">{{ $t('org.audit.empty') }}</p>
    <div v-else class="au-wrap">
      <table class="au-t">
        <thead><tr><th>{{ $t('org.audit.time') }}</th><th>{{ $t('org.audit.actor') }}</th><th>{{ $t('org.audit.action') }}</th><th>{{ $t('org.audit.entity') }}</th></tr></thead>
        <tbody>
          <tr v-for="r in rows" :key="r.id">
            <td>{{ new Date(r.created_at).toLocaleString() }}</td>
            <td>{{ who(r) }}</td>
            <td>{{ label(r.action) }}</td>
            <td class="c-mute">{{ r.entity }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <button v-if="next" type="button" class="c-btn" :disabled="busy" @click="load">{{ $t('org.audit.more') }}</button>
  </main>
</template>

<style scoped>
.au-wrap{overflow-x:auto;margin-bottom:16px}
.au-t{width:100%;border-collapse:collapse;font-size:14px}
.au-t th{font:600 11px var(--mono);letter-spacing:.1em;text-transform:uppercase;text-align:left;padding:10px 12px;border-bottom:2px solid var(--ink-line);background:var(--bg-alt)}
.au-t td{padding:10px 12px;border-bottom:1px solid var(--line);white-space:nowrap}
</style>
