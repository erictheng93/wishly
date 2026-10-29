<script setup lang="ts">
import { renderSVG } from 'uqr'
// download=false：內容是機密（如 TOTP 金鑰）時不提供存檔
const props = withDefaults(defineProps<{ text: string, size?: number, label?: string, download?: boolean }>(), { size: 200, download: true })
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
