<script setup lang="ts">
const { locale, locales, setLocale } = useI18n()
const { user, fetchMe } = useAuth()
async function change(code: any) {
  await setLocale(code)
  if (user.value) await fetchMe(true) // 登入中：fetchMe 會把新語言同步到後端
}
const list = computed(() => locales.value.map(l => (typeof l === 'string' ? { code: l, name: l } : l)))
</script>
<template>
  <select class="locale-switch" :value="locale" :aria-label="$t('common.language')" @change="change(($event.target as HTMLSelectElement).value)">
    <option v-for="l in list" :key="l.code" :value="l.code">{{ l.name }}</option>
  </select>
</template>
<style scoped>
.locale-switch{field-sizing:content;min-height:44px;border:0;background:none;color:var(--red-text);font:inherit;font-size:14px;cursor:pointer}
</style>
