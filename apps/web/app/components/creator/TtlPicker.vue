<script setup lang="ts">
// Q12 購買期限：claim_ttl_hours 168 / 336 / 720 / null（不限）
const ttl = defineModel<number | null>({ required: true })
// 經 API 設定的非預設值（如 relief 48 小時）照樣列出，避免選單空白
const custom = computed(() => ttl.value != null && ![168, 336, 720].includes(ttl.value) ? ttl.value : null)
</script>
<template>
  <label class="c-field"><span>{{ $t('lists.common.ttl') }}</span>
    <select v-model="ttl" name="claim_ttl_hours">
      <option :value="168">{{ $t('lists.common.ttl7') }}</option>
      <option :value="336">{{ $t('lists.common.ttl14') }}</option>
      <option :value="720">{{ $t('lists.common.ttl30') }}</option>
      <option v-if="custom" :value="custom">{{ $t('lists.common.ttlHours', { n: custom }) }}</option>
      <option :value="null">{{ $t('lists.common.ttlNone') }}</option>
    </select>
    <small class="c-mute">{{ $t('lists.common.ttlHint') }}</small>
  </label>
</template>
