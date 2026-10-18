<script setup lang="ts">
const props = defineProps<{ v: string }>()
const { t, te } = useI18n()
const text = computed(() => te(`creator.status.${props.v}`) ? t(`creator.status.${props.v}`) : props.v)
</script>
<template><span :class="['c-badge', v]">{{ text }}</span></template>
