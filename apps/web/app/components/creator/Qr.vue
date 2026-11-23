<script setup lang="ts">
import { encode, renderSVG } from 'uqr'
// download=false：內容是機密（如 TOTP 金鑰）時不提供存檔
const props = withDefaults(defineProps<{ text: string, size?: number, label?: string, download?: boolean }>(), { size: 200, download: true })
const svg = computed(() => props.text ? renderSVG(props.text, { ecc: 'M', border: 2 }) : '')
// 存成 PNG：SVG 在手機相簿、LINE 等多半打不開；QR 只有黑白，PNG 無損且比 JPG 銳利
function save() {
  const { size, data } = encode(props.text, { ecc: 'M', border: 4 })
  const px = Math.ceil(1024 / size)
  const canvas = Object.assign(document.createElement('canvas'), { width: size * px, height: size * px })
  const ctx = canvas.getContext('2d')!
  ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, canvas.width, canvas.height); ctx.fillStyle = '#000'
  data.forEach((row, y) => row.forEach((on, x) => on && ctx.fillRect(x * px, y * px, px, px)))
  canvas.toBlob((blob) => {
    if (!blob) return
    const url = URL.createObjectURL(blob)
    Object.assign(document.createElement('a'), { href: url, download: 'wishly-qr.png' }).click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }, 'image/png')
}
</script>
<template>
  <div class="c-qr">
    <!-- eslint-disable-next-line vue/no-v-html -- uqr 自行產生的 SVG，非使用者輸入 -->
    <div role="img" :aria-label="label || $t('creator.qr.label')" :style="{ width: size + 'px', height: size + 'px' }" v-html="svg" />
    <button v-if="download" type="button" class="c-btn" :disabled="!svg" @click="save">{{ $t('creator.qr.download') }}</button>
  </div>
</template>
<style scoped>
.c-qr{display:flex;flex-direction:column;align-items:center;gap:8px;margin:12px 0}
.c-qr :deep(svg){width:100%;height:100%;display:block;background:#fff;border:1px solid var(--line);padding:6px;box-sizing:border-box}
.c-qr>div[role=img]{position:relative;padding:8px;border:1px solid var(--ink-line);box-sizing:content-box;box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line);margin-bottom:6px}
.c-qr>div[role=img]::before{content:"";position:absolute;top:-5px;left:-5px;width:12px;height:12px;border-top:2px solid var(--red);border-left:2px solid var(--red)}
</style>
