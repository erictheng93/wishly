<script setup lang="ts">
// Q12 購買期限：claim_ttl_hours 168 / 336 / 720 / null（不限）
const ttl = defineModel<number | null>({ required: true })
</script>
<template>
  <label class="c-field"><span>{{ $t('lists.common.ttl') }}</span>
    <select v-model="ttl" name="claim_ttl_hours">
      <option :value="168">{{ $t('lists.common.ttl7') }}</option>
      <option :value="336">{{ $t('lists.common.ttl14') }}</option>
      <option :value="720">{{ $t('lists.common.ttl30') }}</option>
      <option :value="null">{{ $t('lists.common.ttlNone') }}</option>
    </select>
    <small class="c-mute">{{ $t('lists.common.ttlHint') }}</small>
  </label>
</template>
