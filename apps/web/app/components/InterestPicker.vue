<script setup lang="ts">
import { INTEREST_GROUPS } from '~/utils/interests'
// 興趣標籤多選（依分組）；max 不設則不限數量
const props = defineProps<{ modelValue: string[]; max?: number }>()
const emit = defineEmits<{ 'update:modelValue': [string[]] }>()
function toggle(tag: string) {
  const cur = props.modelValue
  if (cur.includes(tag)) emit('update:modelValue', cur.filter(x => x !== tag))
  else if (props.max == null || cur.length < props.max) emit('update:modelValue', [...cur, tag])
}
</script>
<template>
  <div>
    <div v-for="g in INTEREST_GROUPS" :key="g.id" class="ip-group">
      <p class="c-mute c-small">{{ $t(`interests.group.${g.id}`) }}</p>
      <div class="c-row wrap">
        <button v-for="tag in g.tags" :key="tag" type="button" class="c-btn small" :class="{ on: modelValue.includes(tag) }" :aria-pressed="modelValue.includes(tag)" data-chip :disabled="!modelValue.includes(tag) && max != null && modelValue.length >= max" @click="toggle(tag)">{{ $t(`interests.tag.${tag}`) }}</button>
      </div>
    </div>
  </div>
</template>
<style scoped>
.ip-group{margin:0 0 10px}
</style>
