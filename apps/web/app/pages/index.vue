<script setup lang="ts">
const { user, loaded, fetchMe } = useAuth()
const { t } = useI18n()
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
const faq = computed(() => [0, 1, 2].map(i => ({ q: t(`me.home.faq.${i}.q`), a: t(`me.home.faq.${i}.a`) })))
</script>

<template>
  <div class="l">
    <header class="l-nav">
      <div class="l-in">
        <b class="l-logo">WISHLY</b>
        <nav>
          <NuxtLink to="/explore" class="l-nav-link">{{ $t('me.home.explore') }}</NuxtLink>
          <NuxtLink to="/inspire" class="l-nav-link">{{ $t('me.home.inspire') }}</NuxtLink>
          <NuxtLink to="/brands" class="l-nav-link">{{ $t('me.home.brands') }}</NuxtLink>
          <NuxtLink to="/#scenes" class="l-nav-link">{{ $t('me.home.seeDemo') }}</NuxtLink>
          <LocaleSwitch />
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

    <main>
      <section class="l-hero l-in">
        <div v-if="signedIn" class="l-hero-copy">
          <p class="l-kicker l-on">{{ $t('me.home.signedIn') }}</p>
          <!-- 四語系的 {name} 都在句尾：身分未確認前以佔位條補在後面 -->
          <h1>{{ $t('me.home.welcome', { name: user ? name : '' }) }}<span v-if="!user" class="l-skel l-skel-name" aria-hidden="true" /></h1>
          <p class="l-lead">{{ $t('me.home.todayLead') }}</p>
          <div class="l-stats">
            <NuxtLink to="/dashboard"><b>{{ stats?.lists ?? '–' }}</b>{{ $t('me.home.statLists') }}</NuxtLink>
            <NuxtLink to="/dashboard"><b>{{ stats?.claimed ?? '–' }}</b>{{ $t('me.home.statClaimed') }}</NuxtLink>
            <NuxtLink to="/friends"><b>{{ user ? requests : '–' }}</b>{{ $t('me.home.statRequests') }}</NuxtLink>
          </div>
          <div class="l-cta">
            <NuxtLink to="/dashboard" class="g-btn">{{ $t('me.home.myLists') }}</NuxtLink>
            <NuxtLink to="/explore" class="g-btn ghost">{{ $t('me.home.ctaExplore') }}</NuxtLink>
          </div>
        </div>
        <div v-else class="l-hero-copy">
          <p class="l-kicker">{{ $t('me.home.kicker') }}</p>
          <h1>{{ $t('me.home.h1') }}</h1>
          <p class="l-lead">{{ $t('me.home.lead') }}</p>
          <p class="l-sub">{{ $t('me.home.sub') }}</p>
          <div class="l-cta">
            <NuxtLink :to="cta.to" class="g-btn">{{ cta.text }}</NuxtLink>
            <NuxtLink to="/explore" class="g-btn ghost">{{ $t('me.home.ctaExplore') }}</NuxtLink>
            <NuxtLink to="/#scenes" class="g-btn ghost">{{ $t('me.home.ctaDemo') }}</NuxtLink>
          </div>
        </div>

        <div v-if="signedIn" class="l-mine">
          <div class="l-mine-top"><span>{{ user ? $t('me.home.mineTitle', { name }) : $t('me.home.myLists') }}</span><NuxtLink to="/dashboard">{{ $t('me.home.mineAll') }} →</NuxtLink></div>
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
          <NuxtLink to="/lists/new" class="l-mine-new">{{ $t('lists.dashboard.create') }}</NuxtLink>
        </div>
        <!-- 假卡片：純示意，不取後端資料 -->
        <div v-else class="l-demo" role="img" :aria-label="$t('me.home.demoAria')">
          <div class="g-top l-demo-top"><span>{{ $t('me.home.demoTitle') }}</span><span class="g-mute">{{ $t('me.home.demoCount') }}</span></div>
          <ul>
            <li class="g-card g-item">
              <span class="g-thumb">{{ $t('me.home.demoThumb1') }}</span>
              <div class="g-body">
                <div class="g-title">{{ $t('me.home.demoItem1') }} <span class="g-badge high">{{ $t('me.home.demoTop') }}</span></div>
                <div class="g-bar"><i style="width:100%" /></div>
                <span class="g-mute">{{ $t('me.home.demoBy') }}</span>
              </div>
            </li>
            <li class="g-card g-item">
              <span class="g-thumb">{{ $t('me.home.demoThumb2') }}</span>
              <div class="g-body">
                <div class="g-title">{{ $t('me.home.demoItem2') }}</div>
                <div class="g-bar"><i style="width:60%" /></div>
                <span class="g-num">{{ $t('me.home.demoRaised') }}</span> <span class="g-mute">{{ $t('me.home.demoGoal') }}</span>
              </div>
            </li>
            <li class="g-card g-item">
              <span class="g-thumb">{{ $t('me.home.demoThumb3') }}</span>
              <div class="g-body">
                <div class="g-title">{{ $t('me.home.demoItem3') }}</div>
                <div class="g-bar"><i style="width:0" /></div>
                <span class="g-btn sm l-fake">{{ $t('me.home.demoGive') }}</span>
              </div>
            </li>
          </ul>
        </div>
      </section>

      <section class="l-in l-sec l-origin">
        <h2>{{ $t('me.home.originTitle') }}</h2>
        <p v-for="i in 3" :key="i">{{ $t(`me.home.origin.${i - 1}`) }}</p>
      </section>

      <section id="scenes" class="l-alt" style="scroll-margin-top:var(--head-h)">
        <div class="l-in">
          <h2>{{ $t('me.home.scenesTitle') }}</h2>
          <div class="l-grid l-3">
            <article v-for="s in scenes" :key="s.k" class="l-scene">
              <span class="l-tag">{{ s.k }}</span>
              <h3>{{ s.t }}</h3>
              <p>{{ s.d }}</p>
              <NuxtLink :to="`/s/${s.slug}`" class="l-more">{{ $t('me.home.sceneMore', { k: s.k }) }}</NuxtLink>
            </article>
          </div>
        </div>
      </section>

      <section class="l-in l-sec">
        <h2>{{ $t('me.home.stepsTitle') }}</h2>
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
          <h2>{{ $t('me.home.featsTitle') }}</h2>
          <div class="l-grid l-4">
            <div v-for="f in feats" :key="f.t" class="g-card l-feat">
              <h3>{{ f.t }}</h3>
              <p>{{ f.d }}</p>
            </div>
          </div>
        </div>
      </section>

      <section class="l-in l-sec l-faq">
        <h2>{{ $t('me.home.faqTitle') }}</h2>
        <details v-for="f in faq" :key="f.q">
          <summary>{{ f.q }}</summary>
          <p>{{ f.a }}</p>
        </details>
      </section>

      <section class="l-end">
        <div class="l-in">
          <h2>{{ $t('me.home.endTitle') }}</h2>
          <NuxtLink :to="cta.to" class="g-btn l-end-btn">{{ cta.text }}</NuxtLink>
        </div>
      </section>
    </main>

    <footer class="l-foot l-in">
      <span>© 2026 Eric Vrataski · Wishly</span>
      <span><NuxtLink to="/terms">{{ $t('me.home.terms') }}</NuxtLink>・<NuxtLink to="/privacy">{{ $t('me.home.privacy') }}</NuxtLink></span>
    </footer>
  </div>
</template>

<style>
/* 首頁 landing（l- 前綴）：沿用 tokens.css，單欄 → 平板 → 桌面；樣式不放進 creator.css 避免每頁都載入 */
.l{--pad:20px}
.l-in{max-width:1080px;margin:0 auto;padding:0 var(--pad)}
.l-nav{position:sticky;top:0;z-index:20;background:var(--glass);border-bottom:3px solid var(--red);padding-top:env(safe-area-inset-top)}
.l-nav .l-in{display:flex;justify-content:space-between;align-items:center;min-height:52px}
.l-nav .l-in>nav{display:flex;align-items:center;gap:8px}
.l-logo{color:var(--red-text);letter-spacing:.2em;font-size:16px}
.l-nav-link{display:inline-flex;align-items:center;min-height:44px;padding:0 12px;font-weight:700;font-size:14px;letter-spacing:.1em}
/* 已登入：Hero 歡迎詞＋數字摘要、右側自己的清單（取代示意卡） */
.l-auth-wait{display:inline-block;width:132px;height:44px}
.l-chip-wait{width:96px;box-sizing:border-box;border:2px solid var(--line)}
.l-skel{display:inline-block;width:60%;height:1em;background:var(--bg-alt);vertical-align:middle}
.l-skel-name{width:3em;height:.8em}
.l-on::before{content:"";display:inline-block;width:8px;height:8px;margin-right:8px;border-radius:50%;background:var(--red);vertical-align:2px}
.l-stats{display:grid;grid-template-columns:repeat(3,1fr);margin-top:24px;border:1px solid var(--line);text-align:left}
.l-stats a{display:flex;flex-direction:column;min-height:44px;padding:10px 12px;border-right:1px solid var(--line);color:var(--mute);font-size:13px;letter-spacing:.06em}
.l-stats a:last-child{border-right:0}
.l-stats b{font-size:26px;line-height:1.2;color:var(--red-text);letter-spacing:0}
.l-mine{box-sizing:border-box;max-width:420px;width:100%;margin:0 auto;border:2px solid var(--red);background:var(--bg)}
.l-mine-top{display:flex;justify-content:space-between;align-items:center;gap:12px;min-height:44px;padding:0 16px;background:var(--red);color:#fff;font-weight:800;font-size:15px;letter-spacing:.04em}
.l-mine-top span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.l-mine-top a{flex:none;display:inline-flex;align-items:center;min-height:44px;color:#fff;font-size:13px;font-weight:700}
.l-mine ul{list-style:none;margin:0;padding:4px 16px}
.l-mine li{padding:10px 0;border-bottom:1px solid var(--line)}
.l-mine li:last-child{border-bottom:0}
.l-mine-r{display:flex;align-items:center;gap:12px}
.l-mine-t{flex:1;min-width:0;color:var(--fg);font-weight:800;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.l-mine .g-bar{margin:6px 0 0}
.l-mine-vis{flex:none;font-size:12px;padding:0 6px;border:1px solid var(--line);color:var(--mute);letter-spacing:.1em}
.l-mine-msg{padding:20px 16px;color:var(--mute);text-align:center}
.l-mine-new{display:flex;justify-content:center;align-items:center;min-height:48px;border-top:1px dashed var(--red);font-weight:700;font-size:14px;letter-spacing:.1em}
/* 窄螢幕：頁首只留 Logo、語言切換與登入鈕／身分晶片（開始建立、我的清單在 Hero 已有），收緊字距與間距讓 CTA 不折行 */
@media (max-width:479px){.l-nav-link,.l-wide{display:none}.l-auth-wait{width:60px}.l-chip-wait{width:72px}.l-logo{letter-spacing:.08em}.l-nav .l-in>nav{gap:4px}.l-nav .g-btn{white-space:nowrap;padding-left:12px;padding-right:12px}}
.l h2{font-size:24px;margin:0 0 24px;text-align:center}
.l h3{font-size:17px;margin:8px 0}
.l p{margin:0}
.l-hero{display:grid;gap:40px;padding-top:40px;padding-bottom:56px}
.l-hero-copy{text-align:center}
.l-kicker{color:var(--red-text);font-weight:700;letter-spacing:.2em;font-size:13px}
.l-hero h1{font-size:clamp(34px,8vw,56px);margin:12px 0}
.l-lead{font-size:clamp(18px,3vw,22px);font-weight:700}
.l-sub{color:var(--mute);margin-top:12px!important}
.l-cta{display:flex;flex-direction:column;gap:12px;margin-top:28px}
.l-demo{max-width:420px;width:100%;margin:0 auto;border:1px solid var(--line);border-top:0;background:var(--bg);pointer-events:none;user-select:none}
.l-demo ul{margin:0;padding:4px 16px 8px}
.l-demo-top{position:static;font-weight:800;font-size:15px}
.l-demo-top>span:first-child{letter-spacing:.04em}
.l-demo .g-card{margin:12px 0}
.l-fake{margin-top:4px}
.l-alt{background:var(--bg-alt);padding:48px 0}
.l-sec{padding-top:48px;padding-bottom:48px}
.l-grid{display:grid;gap:16px}
.l-scene{background:var(--card);border:1px solid var(--line);border-top:4px solid var(--red);padding:20px}
.l-tag{display:inline-block;background:var(--red);color:#fff;font-weight:800;letter-spacing:.2em;padding:2px 10px;font-size:13px}
.l-more{display:inline-flex;align-items:center;min-height:44px;margin-top:8px;font-weight:700;font-size:14px;letter-spacing:.08em}
.l-scene p,.l-feat p,.l-steps p{color:var(--mute);font-size:15px}
.l-steps{list-style:none;margin:0;padding:0}
.l-steps li{border-top:1px solid var(--line);padding-top:12px}
.l-num{font-size:28px;font-weight:800;color:var(--red-text);letter-spacing:.05em}
.l-feat{margin:0}
.l-faq,.l-origin{max-width:760px}
.l-origin p+p{margin-top:12px}
.l-origin p{font-size:16px;line-height:1.8}
.l-faq details{border-bottom:1px solid var(--line)}
.l-faq summary{font-weight:700;min-height:52px}
.l-faq details p{color:var(--mute);padding-bottom:16px}
.l-end{background:var(--stripe);padding:48px 0;text-align:center}
.l-end h2{color:#fff;margin-bottom:20px}
.l-end-btn{background:#fff!important;color:var(--red)!important;border-color:#fff!important;min-width:200px}
.l-foot{display:flex;flex-direction:column;align-items:center;gap:4px;padding-top:24px;padding-bottom:calc(32px + env(safe-area-inset-bottom));font-size:13px;color:var(--mute)}
.l-foot a{color:var(--mute);display:inline-flex;align-items:center;min-height:44px;padding:0 6px}

@media (min-width:768px){ /* 平板 */
  .l{--pad:32px}
  .l-cta{flex-direction:row;justify-content:center}
  .l-3{grid-template-columns:repeat(3,1fr)}
  .l-4{grid-template-columns:repeat(2,1fr)}
  .l-hero{padding-top:64px;padding-bottom:72px}
  .l h2{font-size:30px;margin-bottom:32px}
  .l-alt,.l-end{padding:72px 0}
  .l-sec{padding-top:72px;padding-bottom:72px}
  .l-foot{flex-direction:row;justify-content:space-between}
}
@media (min-width:1100px){ /* 桌面：Hero 左右分欄 */
  .l-hero{grid-template-columns:1.1fr .9fr;align-items:center;gap:64px}
  .l-hero-copy{text-align:left}
  .l-cta{justify-content:flex-start}
  .l-demo,.l-mine{margin:0 0 0 auto}
  .l-4{grid-template-columns:repeat(4,1fr)}
}
</style>
