<script setup lang="ts">
defineProps<{ n: number }>()
</script>
<template>
  <span v-if="n > 0" class="cb" role="img" :aria-label="$t('common.friendRequests', { n })">{{ n > 99 ? '99+' : n }}</span>
</template>
<style scoped>
.cb{display:inline-flex;align-items:center;justify-content:center;min-width:20px;height:20px;padding:0 5px;margin-left:8px;background:var(--red);color:#fff;font-size:12px;font-weight:700;letter-spacing:0;line-height:1}
</style>
