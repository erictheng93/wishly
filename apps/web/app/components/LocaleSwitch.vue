<script setup lang="ts">
const { locale, locales, setLocale } = useI18n()
const route = useRoute()
async function change(code: any) {
  await setLocale(code)
  // 只在手動切換時換到該語系網址（放在全域 watch 會攔截 middleware 切語系中的導覽）
  const to = localizedPath(route.path, code)
  if (isLocalizable(stripLocale(route.path)) && to !== route.path) navigateTo({ path: to, query: route.query, hash: route.hash }, { replace: true })
}
const list = computed(() => locales.value.map(l => (typeof l === 'string' ? { code: l, name: l } : l)))
</script>
<template>
  <select class="locale-switch" :value="locale" :aria-label="$t('common.language')" @change="change(($event.target as HTMLSelectElement).value)">
    <option v-for="l in list" :key="l.code" :value="l.code">{{ l.name }}</option>
  </select>
</template>
<style scoped>
.locale-switch{field-sizing:content;min-height:44px;border:0;background:none;color:var(--fg);font:600 12px var(--mono);letter-spacing:.06em;cursor:pointer}
</style>
