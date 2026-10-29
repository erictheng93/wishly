<script setup lang="ts">
// 封面：預設圖（radio）或自己上傳；兩者互斥。preset / imageKey 由父層送出時依互斥規則帶欄位
defineProps<{ currentUrl?: string | null }>()
const preset = defineModel<string | null>('preset', { required: true })
const imageKey = defineModel<string | null>('imageKey', { required: true })
const emit = defineEmits<{ busy: [boolean] }>()
const reset = ref(0) // 點預設圖時重掛上傳元件，清掉它的預覽
const uid = useId()
// 上傳中被重掛時，舊元件的 busy:false 傳不到這裡，要自己解除，否則父層送出鈕會一直停用
function choose(id: string) { preset.value = id; imageKey.value = null; reset.value++; emit('busy', false) }
function uploaded(k: string | null) { imageKey.value = k; if (k) preset.value = null }
</script>
<template>
  <fieldset class="c-field cp">
    <legend>{{ $t('lists.cover.title') }}</legend>
    <div class="cp-grid" role="radiogroup" :aria-label="$t('lists.cover.title')">
      <label v-for="id in COVER_PRESETS" :key="id" class="cp-opt">
        <input type="radio" :name="`cover-${uid}`" :value="id" :checked="preset === id" @change="choose(id)">
        <img :src="`/covers/${id}.svg`" :alt="$t(`lists.cover.p.${id}`)" width="64" height="64">
        <span>{{ $t(`lists.cover.p.${id}`) }}</span>
      </label>
    </div>
    <CreatorImageUploader :key="reset" :model-value="imageKey" purpose="cover" :current-url="preset ? null : currentUrl" @update:model-value="uploaded" @busy="emit('busy', $event)" />
  </fieldset>
</template>
<style scoped>
.cp{border:0;padding:0;margin:0 0 16px;min-width:0}
.cp legend{padding:0;font:600 11px/1.4 var(--mono);letter-spacing:.12em;text-transform:uppercase;color:var(--mute)}
.cp-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin:8px 0 14px}
.cp-opt{position:relative;display:flex;flex-direction:column;align-items:stretch;gap:6px;font:600 11px/1.3 var(--mono);letter-spacing:.04em;cursor:pointer;border:1px solid var(--line);padding:4px 4px 8px;text-align:center;color:var(--mute);transition:border-color var(--t) var(--ease),transform var(--t) var(--ease)}
.cp-opt input{position:absolute;opacity:0;width:1px;height:1px}
.cp-opt img{width:100%;height:auto;aspect-ratio:1;display:block;background:var(--bg-alt)}
.cp-opt:has(input:checked){border-color:var(--ink-line);box-shadow:4px 4px 0 var(--bg-alt),4px 4px 0 1px var(--ink-line);transform:translate(-2px,-2px);color:var(--fg)}
.cp-opt:has(input:checked)::after{content:"";position:absolute;top:0;right:0;width:12px;height:12px;background:var(--red)}
.cp-opt:has(input:checked) span{font-weight:700}
.cp-opt:has(input:focus-visible){outline:2px solid var(--red);outline-offset:2px}
@media (hover:hover) and (pointer:fine){.cp-opt:hover{border-color:var(--ink-line)}}
@media (max-width:479px){.cp-grid{gap:6px}.cp-opt{font-size:10px;letter-spacing:0}}
</style>
