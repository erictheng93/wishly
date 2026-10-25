<script setup lang="ts">
// 封面：預設圖（radio）或自己上傳；兩者互斥。preset / imageKey 由父層送出時依互斥規則帶欄位
defineProps<{ currentUrl?: string | null }>()
const preset = defineModel<string | null>('preset', { required: true })
const imageKey = defineModel<string | null>('imageKey', { required: true })
const emit = defineEmits<{ busy: [boolean] }>()
const reset = ref(0) // 點預設圖時重掛上傳元件，清掉它的預覽
const uid = useId()
function choose(id: string) { preset.value = id; imageKey.value = null; reset.value++ }
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
.cp-grid{display:grid;grid-template-columns:repeat(4,1fr);gap:8px;margin:8px 0 12px}
.cp-opt{position:relative;display:flex;flex-direction:column;align-items:center;gap:4px;font-size:12px;cursor:pointer;border:2px solid var(--line);padding:4px}
.cp-opt input{position:absolute;opacity:0;width:1px;height:1px}
.cp-opt img{width:100%;height:auto;aspect-ratio:1;display:block}
.cp-opt:has(input:checked){border-color:var(--red);box-shadow:inset 0 0 0 2px var(--red)}
.cp-opt:has(input:checked) span{color:var(--red-text);font-weight:700}
.cp-opt:has(input:focus-visible){outline:2px solid var(--red);outline-offset:2px}
</style>
