<script setup lang="ts">
// 新手導覽條：放在創建者頁標題下、訪客頁頁首下。導覽進行中才出現；在該步驟的頁面說明「怎麼做」，其他頁給「前往」
const route = useRoute()
const { user, fetchMe } = useAuth()
const { state, active, refresh, ensure, finish } = useOnboarding()
const busy = ref(false)
const hidden = computed(() => route.path === '/welcome' || route.path.startsWith('/admin'))
const cur = computed(() => state.value?.current ?? null)
const total = TOUR_STEPS.length
const n = computed(() => cur.value ? TOUR_STEPS.indexOf(cur.value) + 1 : total)
const here = computed(() => onTourPage(cur.value, stripLocale(route.path), state.value?.wishlist_id))
const target = computed(() => tourTarget(cur.value, state.value?.wishlist_id))
async function close() {
  busy.value = true
  try { await finish(true) } catch {} finally { busy.value = false }
}
// 訪客頁（SSR）不一定抓過 /me：有「可能已登入」旗標才抓，免得每位訪客都打一次 API
onMounted(async () => { if (!user.value && useAuthHint().value) await fetchMe(); ensure() })
watch(() => user.value?.id, () => refresh()) // 換帳號才重抓；換頁不抓
</script>
<template>
  <aside v-if="active && !hidden && state" class="tour" :aria-label="$t('onboarding.guide.aria')">
    <div class="tour-top">
      <span class="tour-tag">{{ $t('onboarding.guide.tag', { n, total }) }}</span>
      <span class="tour-pips" aria-hidden="true"><i v-for="s in state.steps" :key="s.key" :class="{ on: s.done, cur: s.key === cur }" /></span>
    </div>
    <template v-if="cur">
      <strong class="tour-title">{{ $t(`onboarding.steps.${cur}.title`) }}</strong>
      <p class="tour-text">{{ here ? $t(`onboarding.steps.${cur}.here`) : $t(`onboarding.steps.${cur}.body`) }}</p>
      <p v-if="cur === 'give'" class="tour-text"><NuxtLink to="/friends">{{ $t('onboarding.guide.friendsLink') }}<UiIcon name="arrow-right" :size="14" /></NuxtLink></p>
    </template>
    <template v-else>
      <strong class="tour-title">{{ $t('onboarding.done.title') }}</strong>
      <p class="tour-text">{{ $t('onboarding.done.body') }}</p>
    </template>
    <div class="tour-act">
      <NuxtLink v-if="cur && !here" :to="localizedPath(target, $i18n.locale)" class="c-btn small primary">{{ $t(`onboarding.steps.${cur}.cta`) }}</NuxtLink>
      <button v-if="!cur" type="button" class="c-btn small primary" :disabled="busy" @click="close">{{ $t('onboarding.done.finish') }}</button>
      <NuxtLink to="/welcome" class="c-btn small">{{ $t('onboarding.guide.overview') }}</NuxtLink>
      <button v-if="cur" type="button" class="c-btn small plain tour-skip" :disabled="busy" @click="close">{{ $t('onboarding.skip') }}</button>
    </div>
  </aside>
</template>
<style scoped>
/* 安靜的細線條帶：不用彩色側條；桌面文字在左、動作在右，高度約減半 */
.tour{display:grid;grid-template-columns:minmax(0,1fr);gap:2px;margin:0 0 20px;padding:14px 16px;border:1px solid var(--line);background:var(--bg-alt)}
.tour-top{display:flex;align-items:center;gap:14px;margin-bottom:2px}
.tour-tag{font:600 11px/1.4 var(--mono);letter-spacing:.12em;color:var(--red-text);text-transform:uppercase}
.tour-pips{display:flex;gap:4px}
.tour-pips i{width:18px;height:6px;background:var(--hatch),var(--bg-alt);outline:1px solid var(--line);outline-offset:-1px}
.tour-pips i.on{background:var(--fg);outline-color:var(--fg)}
.tour-pips i.cur{background:var(--stripe);outline-color:var(--red)}
.tour-title{display:block;font-size:15px;line-height:1.4}
.tour-text{margin:0;font-size:13px;line-height:1.6;color:var(--mute)}
.tour-text a{display:inline-flex;align-items:center;gap:6px;min-height:44px;color:var(--fg);text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:3px}
.tour-act{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin-top:6px}
.tour-skip{margin-left:auto;font-size:13px;color:var(--mute)}
@media (min-width:768px){
  .tour{grid-template-columns:minmax(0,1fr) auto;column-gap:32px;align-items:center}
  .tour>.tour-act{grid-column:2;grid-row:1/span 4;margin:0}
  .tour-skip{margin-left:0}
}
</style>
