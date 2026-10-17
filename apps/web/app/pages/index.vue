<script setup lang="ts">
const { user, fetchMe } = useAuth()
onMounted(() => fetchMe())
const cta = computed(() => user.value ? { to: '/dashboard', text: '我的清單' } : { to: '/login', text: '開始建立' })

useSeoMeta({
  title: 'Wishly 心願同步｜心願清單與心願分享',
  description: '建立心願清單，用一個連結分享。送禮、生日、婚禮，朋友打開連結就能瀏覽，登入會員後即可捐助，不重複、不超賣。',
  ogTitle: 'Wishly 心願同步',
  ogDescription: '列出你想要的，讓想送的人不再猜。',
})

const scenes = [
  { k: '送禮', slug: 'gift', t: '節日與回禮', d: '收禮的人列清單，送禮的人照著挑，不再猜尺寸、顏色和喜好。' },
  { k: '生日', slug: 'birthday', t: '一個連結收齊心意', d: '貼進群組就好。大家各認一樣，也能湊錢一起買大禮。' },
  { k: '婚禮', slug: 'wedding', t: '新家用品一次備齊', d: '家電、餐具、生活用品都列上去，賓客捐助後品項立刻鎖定。' },
]
const steps = [
  { n: '01', t: '建立心願清單', d: '加入想要的品項、價格與連結，十分鐘就能完成。' },
  { n: '02', t: '分享心願連結', d: '用 LINE、FB 或任何管道傳出去，預覽圖會自動帶上。' },
  { n: '03', t: '朋友登入後捐助', d: '點開連結就能看，登入會員即可捐助品項，捐助後其他人不能再選。' },
]
const feats = [
  { t: '防超賣', d: '同一個品項只會被一個人捐助，同時點擊也不會重複。' },
  { t: '免帳號瀏覽', d: '朋友不用建帳號就能看清單，要認捐時再登入會員。' },
  { t: '即時更新', d: '捐助與進度即時同步，頁面不用重新整理。' },
  { t: '心願眾籌', d: '大件品項可以多人湊，進度條一眼看到還差多少。' },
]
const faq = [
  { q: '朋友需要註冊嗎？', a: '瀏覽清單不需要。要捐助（認捐）品項時，需要先登入會員。' },
  { q: '會不會有兩個人送到同一樣？', a: '不會。品項一旦被捐助就會鎖定，逾期未完成的捐助會自動釋放。' },
  { q: '清單可以只給特定的人看嗎？', a: '可以。清單可設為公開、僅知道連結的人，或限定對象。' },
]
</script>

<template>
  <div class="l">
    <header class="l-nav">
      <div class="l-in">
        <b class="l-logo">WISHLY</b>
        <nav>
          <NuxtLink to="/s/demo" class="l-nav-link">看範例</NuxtLink>
          <NuxtLink :to="cta.to" class="g-btn sm">{{ cta.text }}</NuxtLink>
        </nav>
      </div>
    </header>

    <main>
      <section class="l-hero l-in">
        <div class="l-hero-copy">
          <p class="l-kicker">心願清單 × 心願分享</p>
          <h1>Wishly 心願同步</h1>
          <p class="l-lead">列出你想要的，讓想送的人不再猜。</p>
          <p class="l-sub">一個連結分享整份心願清單。朋友打開連結就能瀏覽，登入會員後即可捐助，不重複、不超賣。</p>
          <div class="l-cta">
            <NuxtLink :to="cta.to" class="g-btn">{{ cta.text }}</NuxtLink>
            <NuxtLink to="/s/demo" class="g-btn ghost">看範例清單</NuxtLink>
          </div>
        </div>

        <!-- 假卡片：純示意，不取後端資料 -->
        <div class="l-demo" role="img" aria-label="心願清單範例：三個品項，其中一項已被捐助">
          <div class="g-top l-demo-top"><span>小安的生日心願</span><span class="g-mute">3 項</span></div>
          <ul>
            <li class="g-card g-item">
              <span class="g-thumb">咖啡機</span>
              <div class="g-body">
                <div class="g-title">手沖咖啡組 <span class="g-badge high">最想要</span></div>
                <div class="g-bar"><i style="width:100%" /></div>
                <span class="g-mute">已被 阿凱 捐助</span>
              </div>
            </li>
            <li class="g-card g-item">
              <span class="g-thumb">耳機</span>
              <div class="g-body">
                <div class="g-title">降噪耳機</div>
                <div class="g-bar"><i style="width:60%" /></div>
                <span class="g-num">已湊 $4,800</span> <span class="g-mute">/ $8,000</span>
              </div>
            </li>
            <li class="g-card g-item">
              <span class="g-thumb">書</span>
              <div class="g-body">
                <div class="g-title">設計的法則</div>
                <div class="g-bar"><i style="width:0" /></div>
                <span class="g-btn sm l-fake">我來送</span>
              </div>
            </li>
          </ul>
        </div>
      </section>

      <section class="l-alt">
        <div class="l-in">
          <h2>三種情境，都用同一份清單</h2>
          <div class="l-grid l-3">
            <article v-for="s in scenes" :key="s.k" class="l-scene">
              <span class="l-tag">{{ s.k }}</span>
              <h3>{{ s.t }}</h3>
              <p>{{ s.d }}</p>
              <NuxtLink :to="`/s/${s.slug}`" class="l-more">看{{ s.k }}範例 →</NuxtLink>
            </article>
          </div>
        </div>
      </section>

      <section class="l-in l-sec">
        <h2>三步完成心願分享</h2>
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
          <h2>讓心願清單更好用</h2>
          <div class="l-grid l-4">
            <div v-for="f in feats" :key="f.t" class="g-card l-feat">
              <h3>{{ f.t }}</h3>
              <p>{{ f.d }}</p>
            </div>
          </div>
        </div>
      </section>

      <section class="l-in l-sec l-faq">
        <h2>常見問題</h2>
        <details v-for="f in faq" :key="f.q">
          <summary>{{ f.q }}</summary>
          <p>{{ f.a }}</p>
        </details>
      </section>

      <section class="l-end">
        <div class="l-in">
          <h2>現在就列出你的心願</h2>
          <NuxtLink :to="cta.to" class="g-btn l-end-btn">{{ cta.text }}</NuxtLink>
        </div>
      </section>
    </main>

    <footer class="l-foot l-in">
      <span>© 2026 Eric Vrataski · Wishly</span>
      <span><NuxtLink to="/terms">服務條款</NuxtLink>・<NuxtLink to="/privacy">隱私權政策</NuxtLink></span>
    </footer>
  </div>
</template>

<style>
/* 首頁 landing（l- 前綴）：沿用 tokens.css，單欄 → 平板 → 桌面；樣式不放進 creator.css 避免每頁都載入 */
.l{--pad:20px}
.l-in{max-width:1080px;margin:0 auto;padding:0 var(--pad)}
.l-nav{position:sticky;top:0;z-index:20;background:var(--glass);border-bottom:3px solid var(--red);padding-top:env(safe-area-inset-top)}
.l-nav .l-in{display:flex;justify-content:space-between;align-items:center;min-height:52px}
.l-nav nav{display:flex;align-items:center;gap:8px}
.l-logo{color:var(--red-text);letter-spacing:.2em;font-size:16px}
.l-nav-link{display:inline-flex;align-items:center;min-height:44px;padding:0 12px;font-weight:700;font-size:14px;letter-spacing:.1em}
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
.l-faq{max-width:760px}
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
  .l-demo{margin:0 0 0 auto}
  .l-4{grid-template-columns:repeat(4,1fr)}
}
</style>
