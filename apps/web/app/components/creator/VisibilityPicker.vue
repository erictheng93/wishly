<script setup lang="ts">
// 可見性選擇；摯友名單在設定頁管理（new / edit 共用）
const visibility = defineModel<string>('visibility', { required: true })
const { ids, load } = useCloseFriends()
watch(visibility, v => { if (v === 'selected') load().catch(() => {}) }, { immediate: true })
</script>
<template>
  <fieldset class="c-field c-vis">
    <span>{{ $t('creator.vis.who') }}</span>
    <label v-for="o in VIS_OPTIONS" :key="o.v" class="c-switch">
      <input v-model="visibility" type="radio" name="visibility" :value="o.v">
      <span>{{ o.label }}<br><small class="c-mute">{{ o.desc }}</small></span>
    </label>
  </fieldset>
  <p v-if="visibility === 'selected'" class="c-mute">{{ $t('creator.vis.closeCount', { n: ids.size }) }} <NuxtLink to="/settings#close-friends">{{ $t('creator.vis.manage') }}</NuxtLink></p>
</template>
<style scoped>
.c-vis{border:0;padding:0;margin:0 0 16px;min-width:0}
.c-vis .c-switch{align-items:flex-start;padding:10px 0;margin:0;border-bottom:1px solid var(--line)}
.c-vis .c-switch:has(input:checked) span{font-weight:700}
.c-vis input{margin-top:3px;accent-color:var(--red)}
</style>
