<script setup lang="ts">
// 情境落地頁：接「生日禮物清單」這類搜尋（SSR）；文案在 utils/scenes.ts，精選商品沿用商品目錄的場合篩選
const slug = String(useRoute().params.scene)
const scene = sceneBySlug(slug)
if (!scene) throw createError({ statusCode: 404, fatal: true })
const { public: { siteUrl } } = useRuntimeConfig()
useSeoMeta({ title: scene.seoTitle, description: scene.seoDesc, ogTitle: scene.heading, ogDescription: scene.seoDesc, ogType: 'article' })
useHead({ script: [ldScript(faqLd(scene.faq)), ldScript(breadcrumbLd(siteUrl, [['選物靈感', '/inspire'], [scene.label, `/inspire/${slug}`]]))] })
const { products } = scene.occasion
  ? await useCatalog(`scene-${slug}`, () => ({ recipient: '', occasion: scene.occasion!, budget: '' }))
  : { products: computed(() => []) }
const others = SCENES.filter(s => s.slug !== slug)
</script>

<template>
  <GuestTop />
  <main class="g-wrap sc">
    <nav class="x-idx" aria-label="breadcrumb"><NuxtLink to="/inspire">INSPIRE</NuxtLink><span>{{ scene!.label }}</span></nav>
    <h1>{{ scene!.heading }}</h1>
    <p v-for="(p, i) in scene!.intro" :key="i" class="sc-lead">{{ p }}</p>
    <p class="sc-cta">
      <NuxtLink class="g-btn" to="/register">{{ $t('me.home.start') }}</NuxtLink>
      <NuxtLink v-if="scene!.demo" class="g-btn ghost" :to="`/s/${scene!.demo}`">{{ $t('me.home.seeDemo') }}</NuxtLink>
    </p>

    <section>
      <h2>三步驟做好清單</h2>
      <ol class="sc-steps"><li v-for="s in scene!.steps" :key="s.t"><strong>{{ s.t }}</strong><span>{{ s.d }}</span></li></ol>
    </section>

    <section>
      <h2>可以放進清單的禮物</h2>
      <ul class="sc-ideas"><li v-for="s in scene!.ideas" :key="s.t"><strong>{{ s.t }}</strong><span>{{ s.d }}</span></li></ul>
    </section>

    <section v-if="products.length">
      <h2>精選商品</h2>
      <GuestCatalogGrid :products="products" />
    </section>

    <section>
      <h2>常見問題</h2>
      <dl class="sc-faq"><template v-for="f in scene!.faq" :key="f.q"><dt>{{ f.q }}</dt><dd>{{ f.a }}</dd></template></dl>
    </section>

    <section>
      <h2>其他送禮情境</h2>
      <p class="g-chips"><NuxtLink v-for="s in others" :key="s.slug" class="g-btn ghost sm" :to="`/inspire/${s.slug}`">{{ s.heading }}</NuxtLink></p>
    </section>
  </main>
</template>

<style scoped>
.x-idx{display:flex;justify-content:space-between;gap:16px;margin:28px 0 0;padding-bottom:12px;border-bottom:1px solid var(--ink-line);font:600 11px/1.4 var(--mono);letter-spacing:.16em;color:var(--mute)}
.x-idx a{color:var(--red-text)}
h1{margin:20px 0 20px;font-size:clamp(32px,6vw,64px);letter-spacing:-.03em;font-weight:900;line-height:1.1}
h2{margin:48px 0 16px;padding-top:12px;border-top:1px solid var(--ink-line);font-size:22px}
.sc-lead{max-width:42em;line-height:1.8}
.sc-cta{display:flex;flex-wrap:wrap;gap:8px;margin-top:24px}
.sc-steps,.sc-ideas{list-style:none;margin:0;padding:0;display:grid;gap:0}
.sc-steps{counter-reset:s}
.sc-steps li,.sc-ideas li{display:grid;gap:4px;padding:14px 0;border-bottom:1px solid var(--line)}
.sc-steps li::before{counter-increment:s;content:"0" counter(s);font:700 11px/1 var(--mono);letter-spacing:.16em;color:var(--red-text)}
.sc-steps span,.sc-ideas span{color:var(--mute);line-height:1.7}
@media (min-width:768px){.sc-ideas{grid-template-columns:1fr 1fr;column-gap:32px}}
.sc-faq dt{margin-top:18px;font-weight:700}
.sc-faq dd{margin:6px 0 0;color:var(--mute);line-height:1.7}
.g-chips{display:flex;flex-wrap:wrap;gap:8px}
</style>
