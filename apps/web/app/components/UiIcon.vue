<script setup lang="ts">
// 全站線條圖示：取代 ‹ ☀ ▾ ＋ → 等 Unicode 字元（各平台字形、粗細不一，螢幕閱讀器還會念出「左箭頭」）。
// 一律 1.5px 線寬、currentColor、裝飾用（aria-hidden）；按鈕的名稱由按鈕自己的文字或 aria-label 提供。
type IconName = 'back' | 'chevron-down' | 'arrow-right' | 'arrow-up' | 'arrow-down' | 'plus' | 'minus' | 'sun' | 'moon' | 'auto' | 'line' | 'google' | 'mail' | 'refresh' | 'instagram' | 'threads' | 'youtube' | 'facebook' | 'tiktok' | 'x' | 'github' | 'link'
const props = withDefaults(defineProps<{ name: IconName; size?: number }>(), { size: 20 })
const PATHS: Record<IconName, string> = {
  back: 'M15 5l-7 7 7 7',
  'chevron-down': 'M6 9l6 6 6-6',
  'arrow-right': 'M4 12h16M14 6l6 6-6 6',
  'arrow-up': 'M12 20V4M6 10l6-6 6 6',
  'arrow-down': 'M12 4v16M6 14l6 6 6-6',
  plus: 'M12 5v14M5 12h14',
  minus: 'M5 12h14',
  refresh: 'M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4v4h-4',
  // 外部連結的網站示意（單色線條，不用官方標誌）
  instagram: 'M4 4h16v16H4zM12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7zM16.5 7.5h.01',
  threads: 'M17.5 9A5.5 5.5 0 0 0 7 12c0 4 2 7 5 7s5-2 5-4.5S15 11 12 11s-3 1-3 2.5S10.5 16 12 16',
  youtube: 'M3 6.5h18v11H3zM10 9.5v5l4.5-2.5z',
  facebook: 'M14 21v-8h3l.5-3.5H14V7.5c0-1 .5-2 2-2h1.5V2.5H15C12 2.5 10.5 4.5 10.5 7v2.5H7.5V13h3v8',
  tiktok: 'M14 3v11.5a3.5 3.5 0 1 1-3.5-3.5M14 3c0 2.5 2 4.5 5 4.5',
  x: 'M4 4l16 16M20 4L4 20',
  github: 'M9 19c-4 1.2-4-2-6-2.500M15 21v-3.500c0-1 .1-1.600-.5-2.500 2.800-.3 5.500-1.400 5.500-6a4.600 4.600 0 0 0-1.300-3.200 4.300 4.300 0 0 0-.1-3.200s-1.100-.3-3.500 1.300a12 12 0 0 0-6.200 0C6.500 2.800 5.400 3.100 5.400 3.100a4.300 4.300 0 0 0-.1 3.200A4.600 4.600 0 0 0 4 9.500c0 4.600 2.700 5.700 5.500 6-.6.600-.6 1.200-.5 2.500V21',
  link: 'M10 14a4 4 0 0 0 5.700 0l3-3a4 4 0 0 0-5.700-5.700l-1 1M14 10a4 4 0 0 0-5.700 0l-3 3a4 4 0 0 0 5.700 5.700l1-1',
  // 第三方登入：單色線條示意，不用官方彩色標誌
  line: 'M4 5h16v11H10l-4 3.5V16H4z',
  google: 'M19.5 12H12M19.5 12a7.5 7.5 0 1 1-2.2-5.3',
  mail: 'M3.5 6.5h17v11h-17zM3.5 6.5l8.5 6.5 8.5-6.5',
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
