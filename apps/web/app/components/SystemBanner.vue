<script setup lang="ts">
// 高負載 / 維護橫幅：掛在 app.vue，所有頁面共用；到期後若是唯讀維護就再問一次 /public/status
const s = useSysStatus()
const { public: { apiBase } } = useRuntimeConfig()
async function check() {
  try {
    const r = await $fetch<{ read_only: boolean }>(`${apiBase}/api/v1/public/status`)
    s.value = r.read_only ? { kind: 'readOnly', until: Date.now() + 60_000 } : null
  } catch (e: any) { noteUnavailable(e?.statusCode ?? 0, e?.data?.code) }
}
let timer: ReturnType<typeof setTimeout> | undefined
watch(s, (v) => {
  clearTimeout(timer)
  if (v && import.meta.client) timer = setTimeout(() => v.kind === 'readOnly' ? check() : (s.value = null), Math.max(v.until - Date.now(), 1000))
}, { immediate: true })
onMounted(() => { if (!s.value) check() })
onBeforeUnmount(() => clearTimeout(timer))
</script>
<template>
  <div v-if="s" class="sys-banner" role="status">{{ s.kind === 'readOnly' ? $t('common.sys.readOnly') : $t('common.sys.busy') }}</div>
</template>
<style scoped>
.sys-banner{padding:calc(8px + env(safe-area-inset-top)) 16px 8px;background:var(--accent);color:var(--accent-fg);border-bottom:4px solid transparent;border-image:var(--stripe) 4;font-size:14px;text-align:center;overflow-wrap:anywhere}
</style>
