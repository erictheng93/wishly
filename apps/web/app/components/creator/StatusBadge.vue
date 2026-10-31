<script setup lang="ts">
const props = defineProps<{ v: string, label?: string }>() // label：同一狀態值在別的情境用不同文案（例：據點 active＝收件中）
const { t, te } = useI18n()
const text = computed(() => props.label ?? (te(`creator.status.${props.v}`) ? t(`creator.status.${props.v}`) : props.v))
</script>
<template><span :class="['c-badge', v]">{{ text }}</span></template>
