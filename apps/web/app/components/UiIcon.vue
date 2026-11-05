<script setup lang="ts">
// 全站線條圖示：取代 ‹ ☀ ▾ ＋ → 等 Unicode 字元（各平台字形、粗細不一，螢幕閱讀器還會念出「左箭頭」）。
// 一律 1.5px 線寬、currentColor、裝飾用（aria-hidden）；按鈕的名稱由按鈕自己的文字或 aria-label 提供。
type IconName = 'back' | 'chevron-down' | 'arrow-right' | 'arrow-up' | 'arrow-down' | 'plus' | 'sun' | 'moon' | 'auto'
const props = withDefaults(defineProps<{ name: IconName; size?: number }>(), { size: 20 })
const PATHS: Record<IconName, string> = {
  back: 'M15 5l-7 7 7 7',
  'chevron-down': 'M6 9l6 6 6-6',
  'arrow-right': 'M4 12h16M14 6l6 6-6 6',
  'arrow-up': 'M12 20V4M6 10l6-6 6 6',
  'arrow-down': 'M12 4v16M6 14l6 6 6-6',
  plus: 'M12 5v14M5 12h14',
  sun: 'M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM12 2.5V5M12 19v2.5M2.5 12H5M19 12h2.5M5.3 5.3l1.8 1.8M16.9 16.9l1.8 1.8M5.3 18.7l1.8-1.8M16.9 7.1l1.8-1.8',
  moon: 'M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z',
  // 跟隨系統：半實心圓（實心半邊另以 fill 畫）
  auto: 'M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z',
}
</script>
<template>
  <svg class="ui-icon" :width="props.size" :height="props.size" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="square" stroke-linejoin="miter" aria-hidden="true" focusable="false">
    <path :d="PATHS[props.name]" />
    <path v-if="props.name === 'auto'" d="M12 4a8 8 0 0 0 0 16z" fill="currentColor" stroke="none" />
  </svg>
</template>
<style>
.ui-icon{flex:none;display:block}
</style>
