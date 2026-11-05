<script setup lang="ts">
defineProps<{ open: boolean; title: string; text?: string; ok?: string; danger?: boolean; busy?: boolean; disabled?: boolean; error?: string }>()
defineEmits<{ close: []; ok: [] }>()
</script>
<template>
  <CreatorSheet :open="open" :title="title" @close="$emit('close')">
    <p v-if="text">{{ text }}</p>
    <p v-if="error" class="c-err" role="alert">{{ error }}</p>
    <slot />
    <div class="c-row">
      <button type="button" class="c-btn c-grow" autofocus @click="$emit('close')">{{ $t('creator.confirm.cancel') }}</button>
      <button type="button" class="c-btn c-grow" :class="danger ? 'danger' : 'primary'" :disabled="busy || disabled" @click="$emit('ok')">{{ busy ? $t('creator.confirm.busy') : ok || $t('creator.confirm.ok') }}</button>
    </div>
  </CreatorSheet>
</template>
<style>
/* 破壞性確認（封存、刪除）：確認鈕實心，與預設焦點的「取消」拉開層級 */
dialog.c-sheet .c-btn.danger{background:var(--accent);border-color:var(--accent);color:var(--accent-fg)}
</style>
