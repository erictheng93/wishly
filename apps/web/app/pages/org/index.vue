<script setup lang="ts">
// 公益組織介紹頁（訪客可看，SSR）；CTA 未登入先走登入，回來直接進申請表
const { t } = useI18n()
useSeoMeta({ title: () => t('org.seoTitle'), description: () => t('org.seoDesc') })
if (!(await useOrgsEnabled()).value) throw createError({ statusCode: 404, fatal: true })
const { user } = useAuth()
const hint = useAuthHint()
const cta = computed(() => (user.value || hint.value ? '/org/apply' : '/login?redirect=/org/apply'))
</script>

<template>
  <GuestTop />
  <main class="g-wrap og">
    <header class="og-hero">
      <p class="og-kicker" aria-hidden="true"><i /> RELIEF / ORGANISATIONS</p>
      <h1>{{ $t('org.heading') }}</h1>
      <div class="og-cta">
        <NuxtLink :to="cta" class="g-btn">{{ $t('org.cta') }}</NuxtLink>
      </div>
      <div class="og-stripe" aria-hidden="true" />
    </header>
    <ul class="og-pts">
      <li v-for="n in 4" :key="n" class="og-pt">
        <span class="og-n" aria-hidden="true">{{ String(n).padStart(2, '0') }}</span>
        <span class="og-t">{{ $t(`org.p${n}`) }}</span>
      </li>
    </ul>
    <section class="og-prep">
      <h2>{{ $t('org.prepTitle') }}</h2>
      <ul>
        <li v-for="n in 3" :key="n"><span aria-hidden="true">{{ String(n).padStart(2, '0') }}</span>{{ $t(`org.prep${n}`) }}</li>
      </ul>
    </section>
  </main>
</template>

<style scoped>
.og-hero{padding:clamp(28px,7vw,72px) 0 clamp(24px,4vw,40px);position:relative}
.og-kicker{display:flex;align-items:center;gap:10px;margin:0 0 18px;font:600 11px/1 var(--mono);letter-spacing:.2em;color:var(--red-text);animation:rise .5s var(--ease) both}
.og-kicker i{width:10px;height:10px;background:var(--red)}
.og-hero h1{font-size:clamp(38px,8.4vw,96px);line-height:1.02;letter-spacing:-.035em;font-weight:900;margin:0 0 clamp(24px,4vw,40px);max-width:11em;animation:rise .6s .05s var(--ease) both}
.og-cta{display:flex;align-items:center;flex-wrap:wrap;gap:20px 28px;animation:rise .6s .12s var(--ease) both}
.og-cta .g-btn{min-height:56px;padding:0 32px;font-size:16px}
.og-stripe{height:10px;margin-top:clamp(32px,6vw,64px);background:var(--stripe);animation:fill 1.1s .2s var(--ease) both;transform-origin:left}
.og-pts{list-style:none;margin:0;padding:0;display:grid;grid-template-columns:1fr;border-top:1px solid var(--ink-line)}
.og-pt{display:flex;gap:16px;align-items:baseline;padding:22px 0;border-bottom:1px solid var(--line);animation:rise .5s var(--ease) both}
.og-pt:nth-child(2){animation-delay:.05s}.og-pt:nth-child(3){animation-delay:.1s}.og-pt:nth-child(4){animation-delay:.15s}
.og-n{font:800 clamp(28px,4vw,44px)/1 var(--mono);color:var(--red-text);letter-spacing:-.04em;flex:none;min-width:2.1em}
.og-t{font-size:clamp(18px,2.2vw,22px);font-weight:800;line-height:1.35}
@media (min-width:768px){
  .og-pts{grid-template-columns:1fr 1fr;border-left:1px solid var(--line)}
  .og-pt{flex-direction:column;gap:14px;padding:24px 22px 28px;border-right:1px solid var(--line);justify-content:flex-start;transition:background var(--t) var(--ease)}
  .og-pt:hover{background:var(--bg-alt)}
}
@media (min-width:1100px){.og-pts{grid-template-columns:repeat(4,1fr)}}
.og-prep{margin:56px 0 0;padding:28px clamp(18px,3vw,32px) 12px;background:var(--bg-alt);border-top:2px solid var(--ink-line)}
.og-prep h2{font-size:22px;margin:0 0 8px}
.og-prep ul{list-style:none;margin:0;padding:0}
.og-prep li{display:flex;align-items:center;gap:16px;min-height:56px;border-bottom:1px solid var(--line);font-size:17px;font-weight:700}
.og-prep li:last-child{border-bottom:0}
.og-prep li span{font:600 11px var(--mono);letter-spacing:.12em;color:var(--red-text);padding:3px 6px;border:1px solid currentColor}
</style>
