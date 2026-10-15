<script setup lang="ts">
const { mode, sync, toggle } = useTheme()
onMounted(sync)
const view = computed(() => ({ auto: ['◐', '自動'], light: ['☀', '淺色'], dark: ['☾', '深色'] })[mode.value])
</script>
<template>
  <button type="button" class="theme-toggle" :aria-label="`主題：${view[1]}（點擊切換）`" :title="`主題：${view[1]}`" @click="toggle">
    {{ view[0] }}
  </button>
</template>
<style scoped>
.theme-toggle{display:inline-flex;align-items:center;justify-content:center;min-width:44px;min-height:44px;border:0;background:none;color:var(--red-text);font:inherit;font-size:18px;cursor:pointer}
</style>
