<script setup lang="ts">
import '~/assets/guest.css'
const { t } = useI18n()
useSeoMeta({ title: () => t('legal.terms.title') })
</script>
<template>
  <GuestTop />
  <main class="g-wrap g-doc">
    <h1>{{ $t('legal.terms.title') }}</h1>
    <p class="g-mute x-draft">{{ $t('legal.terms.draft') }}</p>
    <h2>{{ $t('legal.terms.s1h') }}</h2>
    <p>{{ $t('legal.terms.s1p') }}</p>
    <h2>{{ $t('legal.terms.s2h') }}</h2>
    <ul><li>{{ $t('legal.terms.s2l1') }}</li><li>{{ $t('legal.terms.s2l2') }}</li><li>{{ $t('legal.terms.s2l3') }}</li></ul>
    <h2>{{ $t('legal.terms.s3h') }}</h2>
    <p>{{ $t('legal.terms.s3p') }}</p>
    <h2>{{ $t('legal.terms.s4h') }}</h2>
    <p>{{ $t('legal.terms.s4p') }}</p>
    <h2>{{ $t('legal.terms.s5h') }}</h2>
    <p>{{ $t('legal.terms.s5p') }}</p>
    <h2>{{ $t('legal.terms.s6h') }}</h2>
    <p>{{ $t('legal.terms.s6p') }} <SupportLink /></p>
  </main>
</template>
<style scoped>
.x-draft{display:block;width:max-content;max-width:100%;margin:0 0 24px;padding:6px 12px;border:1px solid var(--line);background:var(--bg-alt);font:500 12px/1.6 var(--mono);letter-spacing:.02em;color:var(--mute)}
.g-doc{max-width:760px}
.g-doc h1{margin:40px 0 12px;font-size:clamp(34px,6vw,52px);font-weight:900;letter-spacing:-.03em}
html:lang(vi) .g-doc h1{line-height:1.22}
.g-doc h2{margin:44px 0 12px;padding-top:20px;border-top:1px solid var(--line);font-size:clamp(19px,2.6vw,22px);font-weight:800;letter-spacing:-.005em}
.g-doc h2:first-of-type{border-top-color:var(--ink-line)}
.g-doc p,.g-doc li{max-width:36em;font-size:16px;line-height:1.9;color:var(--fg)}
.g-doc p.x-draft{font-size:12px;line-height:1.6;color:var(--mute)}
.g-doc ul{padding-left:0;list-style:none}
.g-doc li{position:relative;margin:8px 0;padding-left:20px}
.g-doc li::before{content:"";position:absolute;left:0;top:.95em;width:5px;height:5px;background:var(--mute)}
.g-doc a{text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:4px;text-decoration-thickness:1px;overflow-wrap:anywhere}
/* 桌面：小標移到左欄、內文固定在 36em 右欄，行長落在 35–40 字 */
@media (min-width:1000px){
  .g-doc{max-width:none;display:grid;grid-template-columns:200px minmax(0,36em);justify-content:center}
  .g-doc>h1,.g-doc>.x-draft{grid-column:1/-1}
  .g-doc>h2{grid-column:1;margin:0;padding:28px 48px 0 0;border-top:1px solid var(--line);font-size:17px}
  .g-doc>h2+p,.g-doc>h2+ul{margin:0 0 20px;padding-top:28px;border-top:1px solid var(--line)}
  .g-doc>h2:first-of-type,.g-doc>h2:first-of-type+p,.g-doc>h2:first-of-type+ul{border-top-color:var(--ink-line)}
  .g-doc>p,.g-doc>ul{grid-column:2}
}
</style>
