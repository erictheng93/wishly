<script setup lang="ts">
import { safeRedirect } from '~/utils/redirect'
// 新通知的全站提示：6 秒消失；在通知頁不顯示（列表自己會更新）
const { toast } = useInbox()
const route = useRoute()
let timer: ReturnType<typeof setTimeout> | undefined
const shown = computed(() => (route.path.startsWith('/me/notifications') ? null : toast.value))
watch(toast, (v) => { clearTimeout(timer); if (v) timer = setTimeout(() => { toast.value = null }, 6000) })
onBeforeUnmount(() => clearTimeout(timer))
const to = computed(() => safeRedirect(shown.value?.link, '/me/notifications'))
</script>
<template>
  <div class="c-toast-host" aria-live="polite">
    <div v-if="shown" class="c-toast">
      <NuxtLink :to="to" @click="toast = null"><b>{{ shown.title }}</b><span v-if="shown.body">{{ shown.body }}</span></NuxtLink>
      <button type="button" :aria-label="$t('common.close')" @click="toast = null">×</button>
    </div>
  </div>
</template>
<style>
.c-toast-host{position:fixed;left:8px;right:8px;bottom:calc(8px + env(safe-area-inset-bottom));z-index:60;display:flex;justify-content:flex-end;pointer-events:none}
.c-toast{pointer-events:auto;display:flex;align-items:flex-start;width:360px;max-width:100%;background:var(--card);border:1px solid var(--ink-line);border-left:4px solid var(--red);box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line);animation:rise .22s var(--ease)}
.c-toast a{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px;padding:10px 4px 10px 14px;color:var(--fg);font-size:14px}
.c-toast a span{color:var(--mute);font-size:13px;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}
.c-toast button{flex:none;width:44px;height:44px;border:0;background:none;color:var(--mute);font-size:20px;cursor:pointer}
</style>
