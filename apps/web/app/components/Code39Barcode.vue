<script setup lang="ts">
// 可掃描的 Code 39 條碼（內嵌 SVG）；值含不合法字元時不渲染（文字與複製鈕仍在）
const props = defineProps<{ value: string }>()
const QUIET = 10 // 左右留白單位（≥ 10 倍窄條）
const enc = computed(() => { try { return props.value ? code39(props.value) : null } catch { return null } })
</script>
<template>
  <svg v-if="enc" role="img" :aria-label="value" :viewBox="`0 0 ${enc.width + QUIET * 2} 60`" preserveAspectRatio="none" shape-rendering="crispEdges" :style="{ display: 'block', width: '100%', maxWidth: `${(enc.width + QUIET * 2) * 2}px`, height: '60px', background: '#fff' }">
    <rect :width="enc.width + QUIET * 2" height="60" fill="#fff" />
    <rect v-for="b in enc.bars" :key="b.x" :x="b.x + QUIET" y="0" :width="b.w" height="60" fill="#000" />
  </svg>
</template>
