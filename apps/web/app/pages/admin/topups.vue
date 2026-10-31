<script setup lang="ts">
// 舊路徑：交易明細已併入金流後台
definePageMeta({ middleware: 'staff' })
await navigateTo('/admin/finance?tab=transactions', { replace: true })
</script>
<template><div /></template>
