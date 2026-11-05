<script setup lang="ts">
const { mode, sync, toggle } = useTheme()
onMounted(sync)
const { t } = useI18n()
const view = computed(() => ({ auto: ['auto', t('common.theme.auto')], light: ['sun', t('common.theme.light')], dark: ['moon', t('common.theme.dark')] } as const)[mode.value])
</script>
<template>
  <button type="button" class="theme-toggle" :aria-label="$t('common.theme.label', { mode: view[1] })" :title="$t('common.theme.title', { mode: view[1] })" @click="toggle">
    <UiIcon :name="view[0]" :size="18" />
  </button>
</template>
<style scoped>
.theme-toggle{display:inline-flex;align-items:center;justify-content:center;min-width:44px;min-height:44px;border:0;background:none;color:var(--fg);cursor:pointer;transition:transform .4s var(--ease)}
.theme-toggle:hover{transform:rotate(45deg)}
</style>
