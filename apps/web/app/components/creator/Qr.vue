<script setup lang="ts">
import { renderSVG } from 'uqr'
// download=false：內容是機密（如 TOTP 金鑰）時不提供存檔
const props = withDefaults(defineProps<{ text: string, size?: number, label?: string, download?: boolean }>(), { size: 200, label: '分享連結 QR code', download: true })
const svg = computed(() => props.text ? renderSVG(props.text, { ecc: 'M', border: 2 }) : '')
function save() {
  const url = URL.createObjectURL(new Blob([svg.value], { type: 'image/svg+xml' }))
  const a = Object.assign(document.createElement('a'), { href: url, download: 'wishly-qr.svg' })
  a.click(); URL.revokeObjectURL(url)
}
</script>
<template>
  <div class="c-qr">
    <!-- eslint-disable-next-line vue/no-v-html -- uqr 自行產生的 SVG，非使用者輸入 -->
    <div role="img" :aria-label="label" :style="{ width: size + 'px', height: size + 'px' }" v-html="svg" />
    <button v-if="download" type="button" class="c-btn" :disabled="!svg" @click="save">下載 QR</button>
  </div>
</template>
<style scoped>
.c-qr{display:flex;flex-direction:column;align-items:center;gap:8px;margin:12px 0}
.c-qr :deep(svg){width:100%;height:100%;display:block;background:#fff;border:1px solid var(--line)}
</style>
