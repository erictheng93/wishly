<script setup lang="ts">
// 新手導覽條：放在創建者頁標題下、訪客頁頁首下。導覽進行中才出現；在該步驟的頁面說明「怎麼做」，其他頁給「前往」
const route = useRoute()
const { user, fetchMe } = useAuth()
const { state, active, refresh, finish } = useOnboarding()
const busy = ref(false)
const hidden = computed(() => route.path === '/welcome' || route.path.startsWith('/admin'))
const cur = computed(() => state.value?.current ?? null)
const total = TOUR_STEPS.length
const n = computed(() => cur.value ? TOUR_STEPS.indexOf(cur.value) + 1 : total)
const here = computed(() => onTourPage(cur.value, route.path, state.value?.wishlist_id))
const target = computed(() => tourTarget(cur.value, state.value?.wishlist_id))
async function close() {
  busy.value = true
  try { await finish(true) } catch {} finally { busy.value = false }
}
// 訪客頁（SSR）不一定抓過 /me：有「可能已登入」旗標才抓，免得每位訪客都打一次 API
onMounted(async () => { if (!user.value && useAuthHint().value) await fetchMe(); refresh() })
watch(() => route.fullPath, () => refresh())
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
      <p v-if="cur === 'give'" class="tour-text"><NuxtLink to="/friends">{{ $t('onboarding.guide.friendsLink') }}</NuxtLink></p>
    </template>
    <template v-else>
      <strong class="tour-title">{{ $t('onboarding.done.title') }}</strong>
      <p class="tour-text">{{ $t('onboarding.done.body') }}</p>
    </template>
    <div class="tour-act">
      <NuxtLink v-if="cur && !here" :to="target" class="c-btn small primary">{{ $t(`onboarding.steps.${cur}.cta`) }}</NuxtLink>
      <button v-if="!cur" type="button" class="c-btn small primary" :disabled="busy" @click="close">{{ $t('onboarding.done.finish') }}</button>
      <NuxtLink to="/welcome" class="c-btn small">{{ $t('onboarding.guide.overview') }}</NuxtLink>
      <button v-if="cur" type="button" class="c-btn small plain tour-skip" :disabled="busy" @click="close">{{ $t('onboarding.skip') }}</button>
    </div>
  </aside>
</template>
<style scoped>
/* 標本籤樣式：墨色細框、左側紅色訊號條、等寬小標 */
.tour{position:relative;margin:0 0 16px;padding:12px 14px 12px 18px;border:1px solid var(--ink-line);background:var(--card);animation:rise .4s var(--ease) both}
.tour::before{content:'';position:absolute;inset:-1px auto -1px -1px;width:4px;background:var(--stripe)}
.tour-top{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:6px}
.tour-tag{font:600 11px/1.4 var(--mono);letter-spacing:.12em;color:var(--red-text);text-transform:uppercase}
.tour-pips{display:flex;gap:4px}
.tour-pips i{width:18px;height:6px;background:var(--hatch),var(--bg-alt);outline:1px solid var(--line);outline-offset:-1px}
.tour-pips i.on{background:var(--fg);outline-color:var(--fg)}
.tour-pips i.cur{background:var(--stripe);outline-color:var(--red)}
.tour-title{display:block;font-size:16px;line-height:1.4}
.tour-text{margin:4px 0 0;font-size:14px;line-height:1.6;color:var(--mute)}
.tour-act{display:flex;flex-wrap:wrap;align-items:center;gap:8px;margin-top:10px}
.tour-skip{margin-left:auto;font-size:13px;color:var(--mute)}
</style>
