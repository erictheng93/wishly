<script setup lang="ts">
// 高負載 / 維護橫幅：掛在 app.vue，所有頁面共用；到期後若是唯讀維護就再問一次 /public/status
const s = useSysStatus()
const { locale } = useI18n()
const notices = ref<SysNotice[]>([])
const dismissed = ref<string[]>([])
const shown = computed(() => visibleNotices(notices.value, locale.value, dismissed.value))
function dismiss(id: string) { dismissed.value = [...dismissed.value, id]; writeDismissed(dismissed.value) }
const { public: { apiBase } } = useRuntimeConfig()
async function check() {
  try {
    const r = await fetchPublicStatus(apiBase)
    notices.value = r.notices ?? []
    s.value = r.read_only ? { kind: 'readOnly', until: Date.now() + 60_000 } : null
  } catch (e: any) {
    const before = s.value
    noteUnavailable(e?.statusCode ?? 0, e?.data?.code)
    // 離線或非停機錯誤不會改狀態，watcher 不會再排程：自己延 30 秒再查，否則維護橫幅會一直留著
    if (before && s.value === before) s.value = { ...before, until: Date.now() + 30_000 }
  }
}
let timer: ReturnType<typeof setTimeout> | undefined
watch(s, (v) => {
  clearTimeout(timer)
  if (v && import.meta.client) timer = setTimeout(() => v.kind === 'readOnly' ? check() : (s.value = null), Math.max(v.until - Date.now(), 1000))
}, { immediate: true })
// 公告每次掛載都要抓（fetchPublicStatus 3 秒內共用請求）
onMounted(() => { dismissed.value = readDismissed(); check() })
onBeforeUnmount(() => clearTimeout(timer))
</script>
<template>
  <div v-if="s" class="sys-banner" role="status">{{ s.kind === 'readOnly' ? $t('common.sys.readOnly') : $t('common.sys.busy') }}</div>
  <div v-for="n in shown" :key="n.id" class="sys-banner sys-notice" :class="`is-${n.level}`" :role="n.level === 'critical' ? 'alert' : 'status'">
    <span><strong>{{ n.title }}</strong><template v-if="n.body"> {{ n.body }}</template></span>
    <button v-if="n.level !== 'critical'" type="button" class="sys-x" :aria-label="$t('common.sys.dismiss')" @click="dismiss(n.id)"><UiIcon name="x" :size="16" /></button>
  </div>
</template>
<style scoped>
.sys-banner{padding:calc(8px + env(safe-area-inset-top)) 16px 8px;background:var(--accent);color:var(--accent-fg);border-bottom:4px solid transparent;border-image:var(--stripe) 4;font-size:14px;text-align:center;overflow-wrap:anywhere}
.sys-notice{display:flex;justify-content:center;align-items:center;gap:12px}
.sys-notice.is-info,.sys-notice.is-warning{background:var(--bg);color:var(--fg);border-bottom:1px solid var(--ink-line)}
.sys-notice.is-warning{border-bottom-width:2px}
.sys-x{flex:none;background:none;border:0;color:inherit;cursor:pointer;padding:4px;line-height:0}
</style>
