<script setup lang="ts">
const { mode, sync, toggle } = useTheme()
onMounted(sync)
const { t } = useI18n()
const view = computed(() => ({ auto: ['◐', t('common.theme.auto')], light: ['☀', t('common.theme.light')], dark: ['☾', t('common.theme.dark')] })[mode.value])
</script>
<template>
  <button type="button" class="theme-toggle" :aria-label="$t('common.theme.label', { mode: view[1] })" :title="$t('common.theme.title', { mode: view[1] })" @click="toggle">
    {{ view[0] }}
  </button>
</template>
<style scoped>
.theme-toggle{display:inline-flex;align-items:center;justify-content:center;min-width:44px;min-height:44px;border:0;background:none;color:var(--red-text);font:inherit;font-size:18px;cursor:pointer}
</style>
