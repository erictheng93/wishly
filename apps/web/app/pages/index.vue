<script setup lang="ts">
const { user, loaded, fetchMe } = useAuth()
const { t, locale } = useI18n()
// SSR 拿不到 ws_session（在 API 網域），先依 ws_hint 猜：猜已登入就直接畫個人版骨架，/me 回來再確認
const hint = useAuthHint()
const signedIn = computed(() => loaded.value ? !!user.value : !!hint.value)
onMounted(() => {
  fetchMe()
  if (hint.value && !user.value) loadMine() // 與 /me 並行，不必等身分確認
})
// 已登入：Hero 換成歡迎詞＋數字摘要，示意卡換成自己的清單；好友申請數也給頭像紅點用
const { count: requests, refresh: refreshRequests } = useFriendRequests()
const { api } = useApi()
const mine = ref<{ lists: any[], more: boolean } | null>(null)
const mineErr = ref(false)
let mineBusy = false
async function loadMine() {
  mineErr.value = false; mineBusy = true
  // shortcut: 只抓前 100 份算份數與已認領，超過顯示「+」；真有人超過再加後端摘要端點
  try { const r = await api('/wishlists', { query: { limit: 100, status: 'draft,active,closed' }, noRedirect: true }); mine.value = { lists: r.data, more: !!r.next_cursor } }
  catch { mineErr.value = true } finally { mineBusy = false }
}
watch(() => user.value?.id, (id, old) => {
  if (!id || old) mine.value = null // 登出或換帳號：不留前一位的清單
  if (!id) return
  refreshRequests()
  if (!mine.value && !mineBusy) loadMine()
}, { immediate: true })
const plus = computed(() => mine.value?.more ? '+' : '')
const stats = computed(() => mine.value && { lists: `${mine.value.lists.length}${plus.value}`, claimed: `${mine.value.lists.reduce((n, w) => n + (w.completion?.fulfilled_count ?? 0), 0)}${plus.value}` })
const visLabel = (v: string) => VIS_OPTIONS.find(o => o.v === v)?.label ?? ''
const name = computed(() => user.value?.display_name || user.value?.email || '')
const cta = computed(() => signedIn.value ? { to: '/dashboard', text: t('me.home.myLists') } : { to: '/register', text: t('me.home.start') })

useSeoMeta({
  title: () => t('me.home.seoTitle'),
  description: () => t('me.home.seoDesc'),
  ogTitle: () => t('me.home.ogTitle'),
  ogDescription: () => t('me.home.ogDesc'),
})

const scenes = computed(() => (['gift', 'birthday', 'wedding'] as const).map(k => ({ k: t(`me.home.scenes.${k}.k`), slug: { gift: 'demogift00', birthday: 'demobirth0', wedding: 'demowedd00' }[k], t: t(`me.home.scenes.${k}.t`), d: t(`me.home.scenes.${k}.d`) })))
const steps = computed(() => [1, 2, 3].map(i => ({ n: `0${i}`, t: t(`me.home.steps.${i}.t`), d: t(`me.home.steps.${i}.d`) }))) // 語系檔的步驟 key 為 1–3
const feats = computed(() => [0, 1, 2, 3].map(i => ({ t: t(`me.home.feats.${i}.t`), d: t(`me.home.feats.${i}.d`) })))
const lp = (p: string) => { const [path = '/', hash] = p.split('#'); return localizedPath(path, locale.value) + (hash ? '#' + hash : '') }
const links = [['/explore', 'explore'], ['/inspire', 'inspire'], ['/brands', 'brands'], ['/#scenes', 'seeDemo']] as const
const faq = computed(() => [0, 1, 2].map(i => ({ q: t(`me.home.faq.${i}.q`), a: t(`me.home.faq.${i}.a`) })))
const { public: { siteUrl } } = useRuntimeConfig()
useHead({ script: () => homeLd(siteUrl, faq.value).map(ldScript) })
</script>

<template>
  <div class="l">
    <header class="l-nav">
      <div class="l-in">
        <b class="l-logo">WISHLY</b>
        <nav>
          <NuxtLink v-for="[to, k] in links" :key="to" :to="lp(to)" class="l-nav-link">{{ $t(`me.home.${k}`) }}</NuxtLink>
          <span class="l-tools"><LocaleSwitch /><ThemeToggle /></span>
          <!-- 登入狀態未確定前先佔位，避免 SSR 的未登入按鈕閃一下又變；有 ws_hint 時只有晶片要等 -->
          <template v-if="signedIn">
            <NuxtLink to="/dashboard" class="g-btn sm l-wide">{{ $t('me.home.myLists') }}</NuxtLink>
            <CreatorUserMenu v-if="user" />
            <span v-else class="l-auth-wait l-chip-wait" aria-hidden="true" />
          </template>
          <span v-else-if="!loaded" class="l-auth-wait" aria-hidden="true" />
          <template v-else>
            <NuxtLink to="/login" class="g-btn sm ghost">{{ $t('me.home.login') }}</NuxtLink>
            <NuxtLink to="/register" class="g-btn sm l-wide">{{ $t('me.home.start') }}</NuxtLink>
          </template>
        </nav>
      </div>
    </header>
    <!-- 窄螢幕的導覽列：頁首放不下時四個入口移到這一列（不 sticky，隨頁面捲走，頁首維持 52px） -->
    <nav class="l-local">
      <div><NuxtLink v-for="[to, k] in links" :key="to" :to="lp(to)">{{ $t(`me.home.${k}`) }}</NuxtLink></div>
    </nav>

    <main>
      <section class="l-hero l-in">
        <div v-if="signedIn" class="l-hero-copy l-hi">
          <!-- 四語系的 {name} 都在句尾：身分未確認前以佔位條補在後面 -->
          <h1>{{ $t('me.home.welcome', { name: user ? name : '' }) }}<span v-if="!user" class="l-skel l-skel-name" aria-hidden="true" /></h1>
          <p class="l-kicker l-on">{{ $t('me.home.signedIn') }}</p>
          <p class="l-lead">{{ $t('me.home.todayLead') }}</p>
          <div class="l-stats">
            <NuxtLink to="/dashboard"><b>{{ stats?.lists ?? '–' }}</b>{{ $t('me.home.statLists') }}</NuxtLink>
            <NuxtLink to="/dashboard"><b>{{ stats?.claimed ?? '–' }}</b>{{ $t('me.home.statClaimed') }}</NuxtLink>
            <NuxtLink to="/friends"><b>{{ user ? requests : '–' }}</b>{{ $t('me.home.statRequests') }}</NuxtLink>
          </div>
          <div class="l-cta">
            <NuxtLink to="/dashboard" class="g-btn">{{ $t('me.home.myLists') }}</NuxtLink>
            <NuxtLink :to="localizedPath('/explore', $i18n.locale)" class="g-btn ghost">{{ $t('me.home.ctaExplore') }}</NuxtLink>
          </div>
        </div>
        <div v-else class="l-hero-copy">
          <h1>{{ $t('me.home.h1') }}</h1>
          <p class="l-kicker">{{ $t('me.home.kicker') }}</p>
          <p class="l-lead">{{ $t('me.home.lead') }}</p>
          <p class="l-sub">{{ $t('me.home.sub') }}</p>
          <div class="l-cta">
            <NuxtLink :to="cta.to" class="g-btn">{{ cta.text }}</NuxtLink>
            <NuxtLink :to="localizedPath('/explore', $i18n.locale)" class="g-btn ghost">{{ $t('me.home.ctaExplore') }}</NuxtLink>
            <NuxtLink :to="localizedPath('/', $i18n.locale) + '#scenes'" class="g-btn ghost">{{ $t('me.home.ctaDemo') }}</NuxtLink>
          </div>
        </div>

        <div v-if="signedIn" class="l-mine l-spec">
          <div class="l-spec-head" aria-hidden="true"><span>MEMBER FILE</span><span>No.0001</span></div>
          <div class="l-mine-top"><span>{{ user ? $t('me.home.mineTitle', { name }) : $t('me.home.myLists') }}</span><NuxtLink to="/dashboard">{{ $t('me.home.mineAll') }}<UiIcon name="arrow-right" :size="16" /></NuxtLink></div>
          <p v-if="mineErr" class="l-mine-msg" role="alert">{{ $t('me.home.mineErr') }} <button type="button" class="g-btn sm ghost" @click="loadMine">{{ $t('lists.common.retry') }}</button></p>
          <ul v-else-if="!mine" role="status" :aria-label="$t('lists.common.loading')">
            <li v-for="i in 3" :key="i" aria-hidden="true"><span class="l-skel" /><div class="g-bar" /></li>
          </ul>
          <p v-else-if="!mine.lists.length" class="l-mine-msg">{{ $t('lists.dashboard.empty') }}</p>
          <ul v-else>
            <li v-for="w in mine.lists.slice(0, 3)" :key="w.id">
              <div class="l-mine-r">
                <NuxtLink :to="`/lists/${w.id}/edit`" class="l-mine-t">{{ w.title }}</NuxtLink>
                <span class="l-mine-vis">{{ visLabel(w.visibility) }}</span>
              </div>
              <div class="g-bar" role="img" :aria-label="`${w.completion?.completion_pct ?? 0}%`"><i :style="{ width: `${w.completion?.completion_pct ?? 0}%` }" /></div>
            </li>
          </ul>
          <NuxtLink to="/lists/new" class="l-mine-new"><UiIcon name="plus" :size="14" />{{ $t('lists.dashboard.create') }}</NuxtLink>
          <div class="l-bc" aria-hidden="true"><Code39Barcode value="WISHLY0001" /><span>WISHLY-0001</span></div>
        </div>
        <!-- 假卡片：純示意，不取後端資料；做成「標本籤」：吊孔、編號、條碼 -->
        <div v-else class="l-demo l-spec" role="img" :aria-label="$t('me.home.demoAria')">
          <div class="l-spec-head" aria-hidden="true"><span>SPECIMEN LABEL</span><span>No.0427</span></div>
          <div class="l-demo-top"><span>{{ $t('me.home.demoTitle') }}</span><span class="g-mute">{{ $t('me.home.demoCount') }}</span></div>
          <ul>
            <li class="g-card g-item">
              <span class="g-thumb is-empty" aria-hidden="true" />
              <div class="g-body">
                <div class="g-title">{{ $t('me.home.demoItem1') }} <span class="g-badge high">{{ $t('me.home.demoTop') }}</span></div>
                <div class="g-bar"><i style="width:100%" /></div>
                <span class="g-mute">{{ $t('me.home.demoBy') }}</span>
              </div>
            </li>
            <li class="g-card g-item">
              <span class="g-thumb is-empty" aria-hidden="true" />
              <div class="g-body">
                <div class="g-title">{{ $t('me.home.demoItem2') }}</div>
                <div class="g-bar"><i style="width:60%" /></div>
                <span class="g-num">{{ $t('me.home.demoRaised') }}</span> <span class="g-mute">{{ $t('me.home.demoGoal') }}</span>
              </div>
            </li>
            <li class="g-card g-item">
              <span class="g-thumb is-empty" aria-hidden="true" />
              <div class="g-body">
                <div class="g-title">{{ $t('me.home.demoItem3') }}</div>
                <div class="g-bar"><i /></div>
                <span class="g-btn sm l-fake">{{ $t('me.home.demoGive') }}</span>
              </div>
            </li>
          </ul>
          <div class="l-bc" aria-hidden="true"><Code39Barcode value="WISHLY0427" /><span>WISHLY-0427</span></div>
        </div>
      </section>

      <!-- 警示斜紋跑馬燈：純裝飾 -->
      <div class="l-tick" aria-hidden="true">
        <div><span v-for="i in 8" :key="i">WISHLY<i /> NO OVERSELL<i /> ONE LINK<i /> REAL-TIME<i /> </span></div>
      </div>

      <section class="l-in l-sec l-origin l-split">
        <div class="l-sh">
          <h2>{{ $t('me.home.originTitle') }}</h2>
        </div>
        <div class="l-origin-body">
          <p v-for="i in 3" :key="i"><span v-for="(line, j) in $t(`me.home.origin.${i - 1}`).split('\n')" :key="j">{{ line }}</span></p>
        </div>
      </section>

      <section id="scenes" class="l-alt" style="scroll-margin-top:var(--head-h)">
        <div class="l-in">
          <div class="l-sh">
            <h2>{{ $t('me.home.scenesTitle') }}</h2>
          </div>
          <div class="l-grid l-3">
            <article v-for="s in scenes" :key="s.k" class="l-scene">
              <span class="l-tag">{{ s.k }}</span>
              <h3>{{ s.t }}</h3>
              <p>{{ s.d }}</p>
              <NuxtLink :to="localizedPath(`/s/${s.slug}`, $i18n.locale)" class="l-more">{{ $t('me.home.sceneMore', { k: s.k }) }}<UiIcon name="arrow-right" :size="18" /></NuxtLink>
            </article>
          </div>
        </div>
      </section>

      <section class="l-in l-sec">
        <div class="l-sh">
          <h2>{{ $t('me.home.stepsTitle') }}</h2>
        </div>
        <ol class="l-grid l-3 l-steps">
          <li v-for="s in steps" :key="s.n">
            <span class="l-num">{{ s.n }}</span>
            <h3>{{ s.t }}</h3>
            <p>{{ s.d }}</p>
          </li>
        </ol>
      </section>

      <section class="l-alt">
        <div class="l-in">
          <div class="l-sh">
            <h2>{{ $t('me.home.featsTitle') }}</h2>
          </div>
          <div class="l-grid l-4 l-feats">
            <div v-for="f in feats" :key="f.t" class="l-feat">
              <h3>{{ f.t }}</h3>
              <p>{{ f.d }}</p>
            </div>
          </div>
        </div>
      </section>

      <section class="l-in l-sec l-faq l-split">
        <div class="l-sh">
          <h2>{{ $t('me.home.faqTitle') }}</h2>
        </div>
        <div class="l-faq-body">
          <details v-for="f in faq" :key="f.q">
            <summary>{{ f.q }}</summary>
            <p>{{ f.a }}</p>
          </details>
        </div>
      </section>

      <section class="l-end">
        <div class="l-end-stripe" aria-hidden="true" />
        <div class="l-in">
          <h2>{{ $t('me.home.endTitle') }}</h2>
          <NuxtLink :to="cta.to" class="g-btn l-end-btn">{{ cta.text }}</NuxtLink>
        </div>
      </section>
    </main>

    <footer class="l-foot">
      <div class="l-in l-foot-row">
        <span>© 2026 Eric Vrataski · Wishly</span>
        <span><NuxtLink :to="localizedPath('/terms', $i18n.locale)">{{ $t('me.home.terms') }}</NuxtLink>・<NuxtLink :to="localizedPath('/privacy', $i18n.locale)">{{ $t('me.home.privacy') }}</NuxtLink>・<SupportLink /></span>
      </div>
      <div class="l-word" aria-hidden="true">WISHLY</div>
    </footer>
  </div>
</template>

<style>
/* 首頁 landing（l- 前綴）：沿用 tokens.css，單欄 → 平板 → 桌面；樣式不放進 creator.css 避免每頁都載入 */
.l{--pad:20px;overflow-x:clip}
.l-in{max-width:1200px;margin:0 auto;padding:0 var(--pad)}
/* 頁首：與 .g-top 同做法（頂端紅線＋毛玻璃＋border-image 滿版） */
.l-nav{position:sticky;top:0;z-index:20;padding-top:env(safe-area-inset-top);-webkit-backdrop-filter:saturate(1.6) blur(14px);backdrop-filter:saturate(1.6) blur(14px);border-bottom:1px solid transparent;border-image:var(--bar-bg) fill 0/0/0 100vmax}
.l-nav .l-in{display:flex;justify-content:space-between;align-items:center;min-height:56px}
.l-nav .l-in>nav{display:flex;align-items:center;gap:8px}
.l-logo{display:inline-flex;align-items:center;gap:8px;font-weight:900;font-size:15px;letter-spacing:.22em;color:var(--fg)}
.l-logo::before{content:"";width:10px;height:10px;background:var(--red)}
.l-nav-link{position:relative;display:inline-flex;align-items:center;min-height:44px;padding:0 12px;font-weight:700;font-size:14px;letter-spacing:.04em}
.l-nav-link::after{content:"";position:absolute;left:12px;right:12px;bottom:8px;height:2px;background:var(--red);transform:scaleX(0);transform-origin:left;transition:transform var(--t) var(--ease)}
.l-tools{display:flex;align-items:center;margin-left:8px;padding-left:8px;border-left:1px solid var(--line)}
/* 已登入：Hero 歡迎詞＋數字摘要、右側自己的清單（取代示意卡） */
.l-auth-wait{display:inline-block;width:132px;height:44px}
.l-chip-wait{width:96px;box-sizing:border-box;border:2px solid var(--line)}
.l-skel{display:inline-block;width:60%;height:1em;background:var(--bg-alt);vertical-align:middle}
.l-skel-name{width:3em;height:.8em}
.l-on::before{content:"";display:inline-block;width:8px;height:8px;margin-right:8px;background:var(--red);vertical-align:1px}
.l-stats{align-self:stretch;display:grid;grid-template-columns:repeat(3,1fr);margin-top:28px;border:1px solid var(--ink-line);text-align:left}
.l-stats a{display:flex;flex-direction:column;min-height:44px;padding:12px 14px;border-right:1px solid var(--line);color:var(--mute);font:600 11px/1.4 var(--mono);letter-spacing:.1em;text-transform:uppercase;transition:background var(--t) var(--ease)}
.l-stats a:last-child{border-right:0}
.l-stats b{font:900 clamp(30px,4vw,44px)/1.1 var(--font);color:var(--fg);letter-spacing:-.02em;margin-bottom:4px}
.l-stats a:hover{background:var(--bg-alt)}
.l-local{display:none}
/* 標本籤：吊孔、編號列、條碼 */
.l-spec{position:relative;box-sizing:border-box;width:100%;max-width:470px;margin:0 auto;border:1px solid var(--ink-line);background:var(--card);box-shadow:12px 12px 0 var(--bg-alt),12px 12px 0 1px var(--line);animation:rise .7s .2s var(--ease) both}
.l-spec::before{content:"";position:absolute;left:50%;top:-1px;width:44px;height:14px;margin-left:-22px;background:var(--bg);border:1px solid var(--ink-line);border-top:0;border-radius:0 0 22px 22px!important;z-index:1}
.l-spec-head{display:flex;justify-content:space-between;align-items:center;padding:26px 20px 12px;font:700 12px/1 var(--mono);letter-spacing:.16em;color:var(--mute);background:var(--hatch)}
.l-spec-head span:last-child{color:var(--red-text)}
.l-bc{padding:14px 20px 18px;border-top:1px solid var(--ink-line);text-align:center}
.l-bc svg{height:44px!important;margin:0 auto;border:1px solid var(--line)}
.l-bc span{display:block;margin-top:6px;font:600 11px/1 var(--mono);letter-spacing:.3em;color:var(--mute)}
.l-demo{pointer-events:none;user-select:none}
.l-demo ul{margin:0;padding:0 20px 6px;list-style:none}
.l-demo-top{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:16px 20px;border-bottom:1px solid var(--ink-line);font-weight:800;font-size:19px;letter-spacing:.02em}
.l-demo .g-card{margin:0;padding:18px 0;border:0;border-bottom:1px solid var(--line);background:none;animation:none}
.l-demo .g-card:last-child{border-bottom:0}
.l-demo .g-bar{margin:8px 0 6px;height:10px}
.l-demo .g-thumb{width:64px;height:64px;flex:0 0 64px}
.l-demo .g-title{font-size:18px}
.l-demo .g-mute,.l-demo .g-num{font-size:14px}
/* 動態時刻：「我來送」被按下 → 進度補滿 → 按鈕變灰並畫上刪除線（防超賣）。全域 reduced-motion 規則會讓 both 填充直接停在終態 */
.l-fake{position:relative;margin-top:4px;min-width:96px;animation:mk-claim 3.4s 1.4s var(--ease) both}
.l-fake::after{content:"";position:absolute;left:50%;top:50%;width:44px;height:1.5px;margin-left:-22px;background:var(--fg);transform-origin:left;animation:mk-strike .35s 3.3s var(--ease) both}
.l-demo li:last-child .g-bar>i{width:0;animation:mk-fill .9s 2s var(--ease) both}
@keyframes mk-fill{to{width:100%}}
@keyframes mk-claim{
  0%,8%{transform:none}
  18%{transform:translate(2px,2px)}
  30%{transform:none}
  58%{background:var(--accent);color:var(--accent-fg);border-color:var(--accent)}
  59%,100%{background:var(--bg-alt);color:var(--mute);border-color:var(--line)}
}
@keyframes mk-strike{from{transform:scaleX(0)}}
.l-mine-top{display:flex;justify-content:space-between;align-items:center;gap:12px;min-height:44px;padding:0 16px;border-bottom:1px solid var(--ink-line);font-weight:800;font-size:15px}
.l-mine-top span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.l-mine-top a{flex:none;display:inline-flex;align-items:center;gap:4px;min-height:44px;color:var(--red-text);font-size:13px;font-weight:700}
.l-mine ul{list-style:none;margin:0;padding:4px 16px}
.l-mine li{padding:12px 0;border-bottom:1px solid var(--line)}
.l-mine li:last-child{border-bottom:0}
.l-mine-r{display:flex;align-items:center;gap:12px}
.l-mine-t{flex:1;min-width:0;color:var(--fg);font-weight:800;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.l-mine .g-bar{margin:8px 0 0}
.l-mine-vis{flex:none;font:600 11px/20px var(--mono);padding:0 6px;border:1px solid var(--line);color:var(--mute);letter-spacing:.1em}
.l-mine-msg{padding:20px 16px;color:var(--mute);text-align:center}
/* 窄螢幕：頁首只留 Logo、語言切換與登入鈕／身分晶片（開始建立、我的清單在 Hero 已有；四個入口移到 .l-local），收緊字距與間距讓 CTA 不折行。
   完整導覽實測需 ~800px（id 最長、zh-Hant ~580px），斷點留約 100px 給登入後的名稱晶片與字型差異；導覽或文案變長要重量 */
@media (max-width:899px){.l-nav-link,.l-wide{display:none}.l-auth-wait{width:60px}.l-chip-wait{width:72px}.l-logo{letter-spacing:.12em}.l-nav .l-in>nav{gap:4px}.l-tools{margin-left:0;padding-left:4px}.l-nav .g-btn{white-space:nowrap;padding-left:12px;padding-right:12px}
  /* 與 Logo 左緣對齊；放不下時橫向捲動，右緣淡出提示還有，末端留白讓最後一項捲到底時不被淡出 */
  .l-local{display:block;border-bottom:1px solid var(--line);background:var(--bg)}
  .l-local>div{display:flex;overflow-x:auto;scrollbar-width:none;padding:0 32px 0 var(--pad);-webkit-mask-image:linear-gradient(90deg,#000 calc(100% - 32px),transparent);mask-image:linear-gradient(90deg,#000 calc(100% - 32px),transparent)}
  .l-local>div::-webkit-scrollbar{display:none}
  .l-local a{flex:none;display:inline-flex;align-items:center;min-height:44px;padding:0 12px;color:var(--fg);font:700 13px var(--mono);letter-spacing:.08em;white-space:nowrap}
  .l-local a:first-child{padding-left:0}}
.l h2{font-size:clamp(28px,5vw,48px);line-height:1.1;letter-spacing:-.025em;margin:0;text-align:left}
.l h3{font-size:18px;margin:8px 0}
.l p{margin:0}
.l-hero{display:grid;gap:48px;padding-top:36px;padding-bottom:64px}
.l-hero-copy{display:flex;flex-direction:column;align-items:flex-start;text-align:left}
/* 標題下的警示斜紋與標籤之間留 24px；選擇器要贏過 .l p{margin:0} */
.l-hero .l-kicker{margin-top:24px}
.l-hero-copy>*{animation:rise .6s var(--ease) both}
.l-hero-copy>:nth-child(2){animation-delay:.05s}.l-hero-copy>:nth-child(3){animation-delay:.1s}.l-hero-copy>:nth-child(4){animation-delay:.15s}.l-hero-copy>:nth-child(n+5){animation-delay:.2s}
.l-kicker{display:inline-block;color:var(--accent-fg);background:var(--red);font:700 12px/24px var(--mono);letter-spacing:.16em;padding:0 10px;text-transform:uppercase}
.l-kicker.l-on{color:var(--red-text);background:none;padding:0}
.l-hero h1{font-size:clamp(72px,9.8vw,142px);line-height:1.04;letter-spacing:-.055em;font-weight:900;margin:0;word-break:keep-all;text-wrap:balance}
.l-hero h1::after{content:"";display:block;width:min(180px,40%);height:16px;margin-top:22px;background:var(--stripe);background-size:17px 17px;transform-origin:left;animation:fill .9s .5s var(--ease) both}
.l-hi h1{font-size:clamp(40px,9vw,92px);line-height:1.05;word-break:normal}
.l-lead{font-size:clamp(22px,2.6vw,32px);line-height:1.4;font-weight:800;margin-top:20px!important;max-width:20em}
.l-sub{color:var(--mute);font-size:18px;line-height:1.75;margin-top:12px!important;max-width:34em}
.l-cta{display:flex;flex-direction:column;gap:12px;margin-top:36px;align-self:stretch}
.l-cta .g-btn:first-child{min-height:56px;font-size:16px;padding:0 32px}
.l-alt{background:var(--bg-alt);padding:56px 0;border-top:1px solid var(--ink-line);border-bottom:1px solid var(--ink-line)}
.l-sec{padding-top:56px;padding-bottom:56px}
/* 區塊頭 */
.l-sh{margin-bottom:40px}
.l-sh h2{font-size:clamp(34px,4.6vw,64px);font-weight:900;letter-spacing:-.035em;line-height:1.05;margin:0}
.l-grid{display:grid;gap:16px}
#scenes{background:var(--fg);color:var(--bg);border:0;padding:88px 0 72px}
#scenes h2{color:var(--bg)}
#scenes .l-grid{display:block}
.l-scene{display:grid;grid-template-columns:1fr;gap:10px 40px;align-items:start;border-top:1px solid color-mix(in srgb,var(--bg) 32%,transparent);padding:32px 0;transition:padding var(--t) var(--ease),background var(--t) var(--ease)}
.l-scene:last-child{border-bottom:1px solid color-mix(in srgb,var(--bg) 32%,transparent)}
.l-scene h3{margin:0;font-size:clamp(28px,3.6vw,48px);font-weight:900;letter-spacing:-.03em;line-height:1.1;color:var(--bg)}
.l-tag{display:inline-block;justify-self:start;background:var(--accent);color:var(--accent-fg);font:700 12px/22px var(--mono);letter-spacing:.2em;padding:0 10px}
.l-more{display:inline-flex;align-items:center;gap:10px;min-height:44px;color:var(--bg);font-weight:700;font-size:14px;letter-spacing:.06em;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:5px}
.l-more .ui-icon{transition:transform var(--t) var(--ease)}
.l-scene p{margin:0;color:color-mix(in srgb,var(--bg) 74%,var(--fg));font-size:17px;line-height:1.7}
.l-feat p,.l-steps p{color:var(--mute);font-size:17px;line-height:1.7}
.l-steps{list-style:none;margin:0;padding:0;gap:0}
.l-steps li{position:relative;padding:34px 24px 12px 0}
.l-steps li::before{content:"";position:absolute;left:0;right:24px;top:0;height:8px;background:var(--hatch),var(--bg-alt)}
.l-steps li::after{content:"";position:absolute;left:0;top:0;height:8px;width:calc((100% - 24px) * var(--p,1));background:var(--stripe)}
.l-steps li:nth-child(1){--p:.34}.l-steps li:nth-child(2){--p:.67}.l-steps li:nth-child(3){--p:1}
.l-steps h3{font-size:24px;margin:0 0 8px}
.l-num{display:block;font:900 clamp(96px,13vw,200px)/.95 var(--font);letter-spacing:-.07em;color:var(--fg);margin-bottom:20px}

.l-feats{gap:0}
.l-feat{padding:28px 0;border-top:1px solid var(--ink-line)}
.l-feat h3{font-size:22px;margin:0 0 6px}
.l-feat p{margin:0}
.l-feat:first-child{display:flex;flex-direction:column;padding:36px 0 0}
.l-feat:first-child h3{font-size:clamp(40px,5vw,72px);font-weight:900;letter-spacing:-.04em;line-height:1.05;margin-bottom:16px}
.l-feat:first-child p{font-size:clamp(18px,1.6vw,22px);max-width:22em}
.l-feat:first-child::after{content:"";display:block;margin-top:40px;height:18px;background:var(--stripe)}


.l-origin-body{display:contents;font-size:clamp(18px,1.5vw,21px);line-height:1.9}
.l-origin .l-sh{margin-bottom:32px}
.l-origin .l-sh h2{font-size:clamp(64px,10vw,150px);letter-spacing:-.06em;line-height:.9}
.l-origin-body span{display:block;text-wrap:balance}
.l-origin-body:not(:lang(zh)) p:not(:last-child) span+span{margin-top:.5em}
.l-origin-body p+p{margin-top:16px}
.l-origin-body p:last-child{margin-top:56px;font-size:clamp(36px,8vw,104px);font-weight:900;letter-spacing:-.045em;line-height:1.12}
.l-origin-body p:last-child span:last-child{display:inline-block;padding-bottom:6px;background:var(--stripe) 0 100%/100% 8px no-repeat}
.l-faq-body{border-top:1px solid var(--ink-line)}
.l-faq details{border-bottom:1px solid var(--line)}
.l-faq summary{display:flex;align-items:center;justify-content:space-between;gap:16px;font-weight:800;font-size:clamp(18px,1.6vw,22px);min-height:76px;list-style:none;cursor:pointer}
.l-faq summary::-webkit-details-marker{display:none}
.l-faq summary::after{content:"";flex:none;width:40px;height:40px;border:1px solid var(--ink-line);background:linear-gradient(currentColor,currentColor) center/14px 1.5px no-repeat,linear-gradient(currentColor,currentColor) center/1.5px 14px no-repeat;transition:transform var(--t) var(--ease),background-color var(--t) var(--ease),color var(--t) var(--ease)}
.l-faq details[open] summary::after{transform:rotate(45deg);background-color:var(--red);border-color:var(--red);color:var(--accent-fg)}
.l-faq details p{color:var(--mute);padding-bottom:20px;font-size:17px;line-height:1.8;max-width:38em}
/* 跑馬燈 */
.l-tick{overflow:hidden;background:var(--fg);color:var(--bg);border-top:1px solid var(--ink-line);border-bottom:1px solid var(--ink-line)}
.l-tick>div{display:flex;width:max-content;padding:11px 0;font:700 12px/1 var(--mono);letter-spacing:.24em;white-space:nowrap;animation:tick 40s linear infinite}
.l-tick span{display:inline-flex;align-items:center}
.l-tick i{display:inline-block;width:8px;height:8px;margin:0 22px;background:var(--red)}
@keyframes tick{to{transform:translateX(-12.5%)}}
@media (prefers-reduced-motion:reduce){.l-tick>div{animation:none!important}}
.l-end{position:relative;background:var(--red);color:#fff;padding:0 0 64px;overflow:hidden}
.l-end-stripe{height:28px;background:var(--stripe);background-size:17px 17px;border-bottom:1px solid rgba(0,0,0,.25);margin-bottom:56px;filter:brightness(.78)}
.l-end h2{color:#fff;font-size:clamp(48px,10vw,152px);line-height:1;letter-spacing:-.05em;font-weight:900;margin:0 0 36px;max-width:12em}
.l-end-btn{background:#fff!important;color:var(--red)!important;border-color:#fff!important;min-width:220px;min-height:64px;font-size:18px}
.l-end-btn:hover{box-shadow:4px 4px 0 #111!important}
.l-foot{padding-top:20px;overflow:hidden}
.l-foot-row{display:flex;flex-direction:column;align-items:center;gap:4px;font:500 12px/1.6 var(--mono);letter-spacing:.08em;color:var(--mute)}
.l-foot a{color:var(--mute);display:inline-flex;align-items:center;min-height:44px;padding:0 6px}
.l-foot a:hover{color:var(--red-text)}
.l-word{font:900 clamp(72px,24vw,340px)/.8 var(--font);letter-spacing:-.05em;color:transparent;-webkit-text-stroke:1.5px color-mix(in srgb,var(--fg) 20%,transparent);text-align:center;margin:32px 0 -.11em;user-select:none;white-space:nowrap}

/* 手機 CTA：主要鈕滿寬，兩個次要鈕等寬並排 */
@media (max-width:767px){
  .l-cta{flex-direction:row;flex-wrap:wrap}
  .l-cta .g-btn:first-child{flex:1 1 100%}
  .l-cta .g-btn.ghost{flex:1 1 0;min-width:0;padding:6px 8px;font-size:14px;line-height:1.3;text-align:center}
}
/* 拉丁語系：顯示級字距不緊於 -.03em（為中文調的 -.05em 級會讓字母互撞）；越南文行高放寬給聲調 */
html:is(:lang(en),:lang(vi),:lang(id)) .l :is(h1,h2,h3,.l-origin-body p:last-child){letter-spacing:-.03em}
html:is(:lang(en),:lang(vi),:lang(id)) .l .l-hero h1{font-size:clamp(52px,8vw,112px);word-break:normal;overflow-wrap:break-word}
html:lang(vi) .l :is(h1,h2){line-height:1.18}
html:lang(vi) .l .l-origin-body p:last-child{line-height:1.25}
html:lang(vi) .l :is(.l-scene h3,.l-feat h3){line-height:1.2}

@media (min-width:768px){ /* 平板 */
  .l{--pad:32px}
  .l-cta{flex-direction:row;flex-wrap:wrap}
  .l-3{grid-template-columns:repeat(3,1fr)}
  .l-hero{padding-top:56px;padding-bottom:88px}
  .l-alt{padding:88px 0}
  .l-sec{padding-top:88px;padding-bottom:88px}
  .l-sh{margin-bottom:64px}
  .l-steps li+li{padding-left:24px}
  .l-foot-row{flex-direction:row;justify-content:space-between}
}
@media (min-width:1100px){ /* 桌面：Hero 左右分欄；原由／問答左標題右內文 */
  .l-hero{grid-template-columns:minmax(0,1fr) 470px;align-items:center;gap:56px;padding-top:72px;padding-bottom:104px}
  .l-spec{margin:0 0 0 auto;rotate:1.6deg}
  .l-split{display:grid;grid-template-columns:5fr 7fr;gap:72px;align-items:start}
  .l-split .l-sh{margin-bottom:0;position:sticky;top:calc(var(--head-h) + 24px)}
  .l-origin{row-gap:28px}
  .l-origin .l-sh{grid-row:1/3;position:static;margin:0}
  .l-origin-body p:nth-child(1){grid-column:2;grid-row:1;margin:0}
  .l-origin-body p:nth-child(2){grid-column:2;grid-row:2;margin:0}
  .l-origin .l-origin-body p:last-child{grid-column:1/-1;grid-row:3;margin-top:88px;font-size:clamp(64px,7.4vw,112px)}
  .l-scene{grid-template-columns:96px 1.15fr 1fr auto;align-items:center;padding:44px 0}
  .l-scene:hover{padding-left:20px;background:color-mix(in srgb,var(--bg) 7%,transparent)}
  .l-scene:hover .l-more .ui-icon{transform:translateX(6px)}
  .l-feats{display:grid;grid-template-columns:1.2fr 1fr;grid-template-rows:repeat(3,auto);column-gap:80px}
  .l-feat:first-child{grid-row:1/4;grid-column:1}
  .l-feat:not(:first-child){grid-column:2;display:grid;grid-template-columns:150px 1fr;gap:24px;align-items:baseline}
  .l-feat:not(:first-child) h3{margin:0}
  .l-end .l-in{display:grid;grid-template-columns:1fr auto;gap:56px;align-items:end}
  .l-end h2{margin:0}
  .l-end-btn{margin-bottom:14px}
}
@media (hover:hover) and (pointer:fine){.l-nav-link:hover::after{transform:none}}
</style>
