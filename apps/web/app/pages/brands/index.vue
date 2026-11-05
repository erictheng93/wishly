<script setup lang="ts">
// 品牌館：品牌目錄（訪客可看，SSR；API 回應由 CDN 快取，不帶 cookie）
const { public: { apiBase } } = useRuntimeConfig()
const { t } = useI18n()
// 行銷頁內容欄放寬到 1200（頁首跟著 --g-w 對齊）；head 標籤隨頁面卸載，不外洩到其他頁
useSeoMeta({ title: () => t('guest.brands.seoTitle'), description: () => t('guest.brands.seoDesc') })
const { data, error, refresh } = await useAsyncData('brands', () => $fetch<any>(`${apiBase}/api/v1/public/brands`))
const brands = computed<any[]>(() => data.value?.brands ?? [])
</script>

<template>
  <GuestTop />
  <main class="g-wrap g-wide">
    <h1 class="x-h1">{{ $t('guest.brands.heading') }}</h1>
    <div v-if="error" class="g-center"><h1>{{ $t('guest.loadFailed') }}</h1><button class="g-btn" @click="refresh()">{{ $t('guest.reload') }}</button></div>
    <div v-else-if="!brands.length" class="g-center g-mute g-shelf"><strong class="g-shelf-t">{{ $t('guest.brands.soonTitle') }}</strong><br>{{ $t('guest.brands.soonBody') }}</div>
    <div v-else class="x-list">
      <article v-for="b in brands" :key="b.slug" class="x-card">
        <img v-if="b.logo_url" class="x-logo" :src="b.logo_url" alt="" loading="lazy">
        <div v-else class="x-logo x-ph" aria-hidden="true">{{ b.name.slice(0, 1) }}</div>
        <NuxtLink :to="localizedPath(`/brands/${b.slug}`, $i18n.locale)" class="x-title">{{ b.name }}</NuxtLink>
        <UiIcon class="x-go" name="arrow-right" :size="24" />
        <p v-if="b.description" class="g-mute">{{ b.description }}</p>
      </article>
    </div>
  </main>
</template>

<style scoped>
.x-h1{font-size:clamp(52px,9.4vw,136px);font-weight:900;letter-spacing:-.05em;line-height:.98;margin:48px 0 32px}
html:is(:lang(en),:lang(vi),:lang(id)) .x-h1{letter-spacing:-.03em}
html:lang(vi) .x-h1{line-height:1.18}
/* 目錄列：品牌照、名稱、簡介、箭頭；連結以 ::before 撐滿整列 */
.x-list{border-top:1px solid var(--ink-line);margin-top:16px}
.x-card{position:relative;display:grid;grid-template-columns:96px minmax(0,1fr) 24px;gap:4px 20px;align-items:center;padding:20px 0;border-bottom:1px solid var(--line);transition:padding var(--t) var(--ease)}
.x-logo{grid-row:1/3;display:block;width:96px;height:96px;object-fit:cover;border:1px solid var(--line);transition:transform var(--t) var(--ease)}
.x-ph{display:flex;align-items:center;justify-content:center;background:var(--hatch),var(--bg-alt);color:var(--red-text);font-size:34px;font-weight:900}
.x-title{grid-column:2;align-self:end;display:block;font-weight:900;font-size:clamp(28px,4vw,52px);letter-spacing:-.035em;line-height:1.05;color:var(--fg);text-decoration:none;overflow-wrap:anywhere}
html:is(:lang(en),:lang(vi),:lang(id)) .x-title{letter-spacing:-.03em}
html:lang(vi) .x-title{line-height:1.2}
.x-title::before{content:"";position:absolute;inset:0}
.x-card>p{grid-column:2;align-self:start;margin:0;font:400 16px/1.6 var(--font);letter-spacing:0;text-transform:none;color:var(--mute)}
.x-go{grid-column:3;grid-row:1/3;transition:transform var(--t) var(--ease)}
@media (min-width:900px){
  .x-card{grid-template-columns:168px minmax(0,1.1fr) minmax(0,1fr) 28px;gap:0 40px;padding:32px 0}
  .x-logo{width:168px;height:168px;grid-row:1}
  .x-title{grid-row:1;align-self:center}
  .x-card>p{grid-column:3;grid-row:1;align-self:center;font-size:18px;line-height:1.7}
  .x-go{grid-column:4;grid-row:1}
}
@media (hover:hover) and (pointer:fine){
  .x-card:hover{padding-left:16px}
  .x-card:hover .x-title{color:var(--red-text)}
  .x-card:hover .x-go{transform:translateX(6px)}
  .x-card:hover .x-logo{transform:translate(-3px,-3px);box-shadow:5px 5px 0 var(--bg-alt),5px 5px 0 1px var(--line)}
}
.x-title:focus-visible{outline:2px solid var(--red);outline-offset:4px}
</style>
