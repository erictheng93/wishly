<script setup lang="ts">
// bottom sheet：原生 <dialog>（焦點鎖定、Esc 關閉、關閉後焦點回觸發元素由瀏覽器處理）
const props = defineProps<{ open: boolean; title: string }>()
const emit = defineEmits<{ close: [] }>()
const el = ref<HTMLDialogElement>()
watch(() => props.open, (v) => {
  if (!el.value) return
  if (v && !el.value.open) el.value.showModal()
  if (!v && el.value.open) el.value.close()
})
</script>
<template>
  <dialog ref="el" class="c-sheet" :aria-label="title" @close="emit('close')">
    <template v-if="open">
      <h2>{{ title }}</h2>
      <slot />
    </template>
  </dialog>
</template>
<style>
dialog.c-sheet h2{padding-bottom:14px;border-bottom:1px solid var(--ink-line);letter-spacing:-.01em}
dialog.c-sheet .c-row:last-child{margin-top:8px}
/* 長表單：操作列黏在面板底部，不必捲到底才找得到「儲存」 */
dialog.c-sheet form>.c-row:last-child{position:sticky;bottom:0;z-index:1;margin-top:8px;padding:12px 0 calc(4px + env(safe-area-inset-bottom));background:var(--card);border-top:1px solid var(--line)}
</style>
