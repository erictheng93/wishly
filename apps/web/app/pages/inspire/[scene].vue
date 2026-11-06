<script setup lang="ts">
import type { GiftPick } from '~/utils/catalog'
// 情境落地頁：接「生日禮物清單」這類搜尋（SSR）；文案在 i18n/locales/*/inspire.json，精選商品沿用商品目錄的場合篩選
const slug = String(useRoute().params.scene)
const scene = sceneBySlug(slug)
if (!scene) throw createError({ statusCode: 404, fatal: true })
const { t, tm, rt, locale } = useI18n()

const k = `inspire.scenes.${slug}`
const pairs = (key: string, a: string, b: string) => (tm(`${k}.${key}`) as any[]).map(x => ({ [a]: rt(x[a]), [b]: rt(x[b]) }))
const c = computed(() => ({
  label: t(`${k}.label`), heading: t(`${k}.heading`), seoDesc: t(`${k}.seoDesc`),
  intro: (tm(`${k}.intro`) as any[]).map(x => rt(x)),
  steps: pairs('steps', 't', 'd'), ideas: pairs('ideas', 't', 'd'), faq: pairs('faq', 'q', 'a') as { q: string, a: string }[],
}))

const { public: { siteUrl, apiBase } } = useRuntimeConfig()
// 行銷頁內容欄放寬到 1200（頁首跟著 --g-w 對齊）；head 標籤隨頁面卸載，不外洩到其他頁
useSeoMeta({ title: () => t(`${k}.seoTitle`), description: () => c.value.seoDesc, ogTitle: () => c.value.heading, ogDescription: () => c.value.seoDesc, ogType: 'article' })
useHead({
  script: computed(() => [ldScript(faqLd(c.value.faq)), ldScript(breadcrumbLd(siteUrl, [[t('guest.inspire.heading'), '/inspire'], [c.value.label, localizedPath(`/inspire/${slug}`, locale.value)]]))]),
})
const { products } = scene.occasion
  ? await useCatalog(`scene-${slug}`, () => ({ recipient: '', occasion: scene.occasion!, budget: '' }))
  : { products: computed(() => []) }
// 幫朋友挑：只在有 occasion 且已登入時出現；選擇存在 ?for=，全在 client 端（SSR 與 CDN 快取不變）
const route = useRoute(), router = useRouter()
const { user, fetchMe } = useAuth()
const friends = ref<{ id: string; display_name: string }[]>([])
const forId = ref(typeof route.query.for === 'string' ? route.query.for : '')
const picks = ref<{ basis: string; products: GiftPick[] } | null>(null)
const forName = computed(() => friends.value.find(f => f.id === forId.value)?.display_name ?? '')
let pickSeq = 0
const occasion = scene.occasion
async function loadPicks() {
  const n = ++pickSeq
  picks.value = null
  if (!forId.value || !occasion) return
  try { const r = await fetchGiftPicks(apiBase, forId.value, { occasion, limit: 50 }); if (n === pickSeq) picks.value = r } catch { /* 失敗就當作沒有指定 */ }
}
function onFor() { router.replace({ query: { ...route.query, for: forId.value || undefined } }); loadPicks() }
onMounted(async () => {
  if (!occasion || !(await fetchMe())) return
  try { friends.value = (await $fetch<any>(`${apiBase}/api/v1/friends`, { credentials: 'include', headers: nativeHeaders() })).friends } catch { return }
  if (!friends.value.some(f => f.id === forId.value)) forId.value = ''
  loadPicks()
})
const shown = computed(() => picks.value?.basis === 'interests' ? mergePicks(picks.value.products, products.value) : products.value)
// 標題在冒號後讓斷行落在語意處（模板不用 v-html，拆字串後插 <wbr>）
const headParts = computed(() => { const m = c.value.heading.match(/^(.*?[：:])(.+)$/); return m ? [m[1]!, m[2]!] : [c.value.heading] })
const others = SCENES.filter(s => s.slug !== slug)
</script>

<template>
  <GuestTop />
  <main class="g-wrap g-wide sc">
    <nav class="x-crumb" aria-label="breadcrumb"><NuxtLink :to="localizedPath('/inspire', locale)">INSPIRE</NuxtLink><span>{{ c.label }}</span></nav>
    <h1 class="x-h1"><template v-for="(part, i) in headParts" :key="i"><wbr v-if="i">{{ part }}</template></h1>
    <p v-for="(p, i) in c.intro" :key="i" class="sc-lead">{{ p }}</p>
    <p class="sc-cta">
      <NuxtLink class="g-btn" to="/register">{{ $t('me.home.start') }}</NuxtLink>
      <NuxtLink v-if="scene!.demo" class="g-btn ghost" :to="localizedPath(`/s/${scene!.demo}`, locale)">{{ $t('me.home.seeDemo') }}</NuxtLink>
    </p>

    <section>
      <h2>{{ $t('inspire.ui.steps') }}</h2>
      <ol class="sc-steps"><li v-for="s in c.steps" :key="s.t"><strong>{{ s.t }}</strong><span>{{ s.d }}</span></li></ol>
    </section>

    <section>
      <h2>{{ $t('inspire.ui.ideas') }}</h2>
      <ul class="sc-ideas"><li v-for="s in c.ideas" :key="s.t"><strong>{{ s.t }}</strong><span>{{ s.d }}</span></li></ul>
    </section>

    <section v-if="products.length">
      <h2>{{ $t('inspire.ui.products') }}</h2>
      <template v-if="user && friends.length">
        <label class="sc-for">{{ $t('guest.inspire.forFriend') }}
          <select v-model="forId" @change="onFor"><option value="">{{ $t('guest.inspire.forNone') }}</option><option v-for="f in friends" :key="f.id" :value="f.id">{{ f.display_name }}</option></select>
        </label>
        <p v-if="picks?.basis === 'fallback'" class="g-mute" role="status">{{ $t('guest.inspire.forFallback', { name: forName }) }}</p>
      </template>
      <GuestCatalogGrid :products="shown" :matched="picks?.basis === 'interests' ? matchedMap(picks.products) : undefined" :matched-name="forName" />
    </section>

    <section>
      <h2>{{ $t('inspire.ui.faq') }}</h2>
      <dl class="sc-faq"><template v-for="f in c.faq" :key="f.q"><dt>{{ f.q }}</dt><dd>{{ f.a }}</dd></template></dl>
    </section>

    <section>
      <h2>{{ $t('inspire.ui.others') }}</h2>
      <nav class="x-others" :aria-label="$t('inspire.ui.others')"><NuxtLink v-for="s in others" :key="s.slug" class="x-scene" :to="localizedPath(`/inspire/${s.slug}`, $i18n.locale)">{{ $t(`inspire.scenes.${s.slug}.heading`) }}<UiIcon name="arrow-right" :size="22" /></NuxtLink></nav>
    </section>
  </main>
</template>

<style scoped>
.x-crumb{display:flex;justify-content:flex-start;align-items:center;gap:12px;margin:40px 0 0;font:700 12px/1 var(--mono);letter-spacing:.12em}
.x-crumb a{display:inline-flex;align-items:center;min-height:44px;color:var(--red-text)}
.x-crumb span{display:inline-flex;align-items:center;gap:12px;color:var(--mute)}
.x-crumb span::before{content:"";width:20px;height:1.5px;background:currentColor}
.x-h1{margin:16px 0 40px;font-size:clamp(38px,6vw,88px);font-weight:900;letter-spacing:-.03em;line-height:1.1;word-break:keep-all}
html:lang(vi) .x-h1{line-height:1.2}
.sc-lead{max-width:38em;font-size:18px;line-height:1.9}
.sc-for{display:flex;align-items:center;gap:12px;margin:0 0 16px;font-weight:700}
.sc-for select{min-height:44px;padding:0 12px;font:inherit;border:1px solid var(--ink-line);background:var(--bg);color:var(--fg)}
.sc-cta{display:flex;flex-wrap:wrap;gap:8px;margin:32px 0 0}
.sc-cta .g-btn{min-height:56px;font-size:16px;padding:0 32px}
section{margin-top:72px;padding-top:32px;border-top:1px solid var(--ink-line)}
h2{margin:0 0 28px;font-size:clamp(28px,3vw,42px);font-weight:900;letter-spacing:-.03em;line-height:1.1}
html:lang(vi) h2{line-height:1.2}
.sc-steps,.sc-ideas{list-style:none;margin:0;padding:0;display:grid;gap:0}
.sc-steps{counter-reset:s}
.sc-steps li,.sc-ideas li{display:grid;gap:4px;padding:20px 0;border-bottom:1px solid var(--line)}
/* 步驟有順序，所以保留編號 */
.sc-steps li{grid-template-columns:72px 1fr;gap:4px 24px;padding:24px 0}
.sc-steps li::before{counter-increment:s;content:"0" counter(s);grid-row:1/3;font:900 44px/1 var(--font);letter-spacing:-.05em;color:var(--fg)}
.sc-steps strong{font-size:21px;align-self:end}
.sc-ideas strong{font-size:18px}
.sc-steps span,.sc-ideas span,.sc-faq dd{font-size:16.5px;color:var(--mute);line-height:1.7}
.sc-faq dt{margin-top:28px;font-size:20px;font-weight:700}
.sc-faq dt:first-child{margin-top:0}
.sc-faq dd{margin:6px 0 0}
.x-others{border-bottom:1px solid var(--ink-line)}
.x-scene{display:flex;align-items:center;justify-content:space-between;gap:24px;min-height:76px;padding:14px 0;border-top:1px solid var(--line);color:var(--fg);font-size:clamp(17px,1.7vw,22px);font-weight:800;line-height:1.4;transition:padding var(--t) var(--ease),color var(--t) var(--ease)}
.x-scene:first-child{border-top-color:var(--ink-line)}
.x-scene .ui-icon{transition:transform var(--t) var(--ease)}
@media (hover:hover) and (pointer:fine){.x-scene:hover{color:var(--red-text);padding-left:14px}.x-scene:hover .ui-icon{transform:translateX(6px)}}
@media (min-width:768px){.sc-ideas{grid-template-columns:1fr 1fr;column-gap:32px}}
/* 桌面：導言並成三欄雜誌式短文；段落標題（sticky）在左、內容在右 */
@media (min-width:900px){
  section{display:grid;grid-template-columns:4fr 8fr;column-gap:56px;align-items:start}
  section>h2{grid-column:1;position:sticky;top:calc(var(--head-h) + 24px);margin:0}
  section>:not(h2){grid-column:2}
}
@media (min-width:1100px){
  .sc{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));column-gap:48px}
  .sc>.x-crumb,.sc>.x-h1,.sc>.sc-cta,.sc>section{grid-column:1/-1}
  .sc>.sc-lead{max-width:none;margin:0;font-size:17px}
  .sc>section{margin-top:96px}
}
</style>
