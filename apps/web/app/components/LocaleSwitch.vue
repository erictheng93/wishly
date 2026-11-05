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
// 窄手機頁首放不下「Tiếng Việt」「Bahasa Indonesia」：可見文字改短代碼，原生 select 透明覆蓋其上（鍵盤、讀屏、選單行為不變）
const SHORT: Record<string, string> = { 'zh-Hant': '中', en: 'EN', vi: 'VI', id: 'ID' }
const cur = computed(() => list.value.find(l => l.code === locale.value) ?? { code: locale.value, name: locale.value })
</script>
<template>
  <label class="locale-switch">
    <span class="ls-full" aria-hidden="true">{{ cur.name }}</span><span class="ls-code" aria-hidden="true">{{ SHORT[cur.code] ?? cur.code }}</span>
    <UiIcon name="chevron-down" :size="10" />
    <select :value="locale" :aria-label="$t('common.language')" @change="change(($event.target as HTMLSelectElement).value)">
      <option v-for="l in list" :key="l.code" :value="l.code">{{ l.name }}</option>
    </select>
  </label>
</template>
<style scoped>
.locale-switch{position:relative;display:inline-flex;align-items:center;gap:5px;min-height:44px;padding:0 6px;color:var(--fg);font:600 12px var(--mono);letter-spacing:.06em;white-space:nowrap;cursor:pointer}
.locale-switch select{position:absolute;inset:0;width:100%;opacity:0;cursor:pointer;font-size:16px}
.locale-switch:focus-within{outline:2px solid var(--red);outline-offset:-2px}
.ls-code{display:none}
@media (max-width:479px){.ls-full{display:none}.ls-code{display:inline}}
</style>
