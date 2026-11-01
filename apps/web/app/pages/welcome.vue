<script setup lang="ts">
import '~/assets/creator-pages.css'
// 新手導覽總覽（D66）：第 1 步個人資料就地填寫，之後的步驟帶到各自的頁面，頁面上方由 OnboardingGuide 接手說明
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('onboarding.welcome.pageTitle') })
const { api } = useApi()
const { user } = useAuth()
const { state, pending, refresh, finish } = useOnboarding()
const loading = ref(true), loadErr = ref(false), busy = ref(false), err = ref('')
const cur = computed(() => state.value?.current ?? null)
const stepDone = (k: TourStep) => !!state.value?.steps.find(s => s.key === k)?.done
const pad = (n: number) => String(n).padStart(2, '0')

// 第 1 步：個人資料（欄位與設定頁相同，至少填一項才算完成）
const name = ref(''), handle = ref(''), bio = ref(''), bMonth = ref(''), bDay = ref('')
const bDays = computed(() => daysInMonth(bMonth.value))
const hBad = computed(() => !!handle.value && !isValidHandle(handle.value))
async function saveProfile() {
  err.value = ''
  if (hBad.value) { err.value = t('settings.social.handleBadErr'); return }
  const birthday = toBirthday(bMonth.value, bDay.value)
  if (birthday === undefined) { err.value = t('settings.social.birthdayErr'); return }
  if (!handle.value && !bio.value.trim() && !birthday) { err.value = t('onboarding.welcome.profileNeedOne'); return }
  busy.value = true
  try {
    user.value = await api('/me', { method: 'PATCH', body: { display_name: name.value.trim() || undefined, handle: handle.value || null, bio: bio.value.trim() || null, birthday } })
    await refresh()
    // 填完個人資料直接帶到建立清單（使用者要求的「一步一步」）
    if (cur.value === 'wishlist') await navigateTo(tourTarget('wishlist'))
  } catch (e: any) { err.value = apiErrMsg(e) } finally { busy.value = false }
}

async function skip() {
  busy.value = true; err.value = ''
  try { await finish(true); await navigateTo('/dashboard') } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
async function restart() {
  busy.value = true; err.value = ''
  try { await finish(false) } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}

async function load() {
  loading.value = true; loadErr.value = false
  if (pending.value && !(await refresh())) loadErr.value = true
  loading.value = false
}
onMounted(async () => {
  // 儀表板只自動帶來一次（每個分頁工作階段）
  try { sessionStorage.setItem('ws_tour_welcomed', '1') } catch {}
  name.value = user.value?.display_name ?? ''; handle.value = user.value?.handle ?? ''; bio.value = user.value?.bio ?? ''
  ;[bMonth.value = '', bDay.value = ''] = user.value?.birthday?.split('-') ?? []
  await load()
})
</script>
<template>
  <main class="c-page wl">
    <CreatorHeader :title="$t('onboarding.welcome.pageTitle')" />
    <p v-if="loading" class="c-mute" role="status">{{ $t('lists.common.loading') }}</p>
    <!-- 已完成或跳過：可重新開始 -->
    <section v-else-if="!pending" class="c-card">
      <h2 class="c-h2">{{ $t('onboarding.welcome.closedTitle') }}</h2>
      <p class="c-mute">{{ $t('onboarding.welcome.closedBody') }}</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <div class="c-row wrap">
        <button type="button" class="c-btn primary" :disabled="busy" @click="restart">{{ $t('onboarding.welcome.restart') }}</button>
        <NuxtLink to="/dashboard" class="c-btn">{{ $t('onboarding.welcome.toDashboard') }}</NuxtLink>
      </div>
    </section>
    <p v-else-if="loadErr || !state" class="c-err" role="alert">{{ $t('onboarding.welcome.loadFailed') }} <button class="c-btn" @click="load">{{ $t('lists.common.retry') }}</button></p>
    <template v-else>
      <section class="wl-intro">
        <p class="wl-hello">{{ $t('onboarding.welcome.hello', { name: user?.display_name }) }}</p>
        <p class="c-mute">{{ $t('onboarding.welcome.lead') }}</p>
        <div class="wl-roles">
          <div class="wl-role" :class="{ on: stepDone('publish') }">
            <span class="x-tag">ROLE / 01<b v-if="stepDone('publish')">・{{ $t('onboarding.welcome.roleDone') }}</b></span>
            <strong>{{ $t('onboarding.welcome.roleReceiver') }}</strong>
            <small class="c-mute">{{ $t('onboarding.welcome.roleReceiverBody') }}</small>
          </div>
          <div class="wl-role" :class="{ on: stepDone('give') }">
            <span class="x-tag">ROLE / 02<b v-if="stepDone('give')">・{{ $t('onboarding.welcome.roleDone') }}</b></span>
            <strong>{{ $t('onboarding.welcome.roleGiver') }}</strong>
            <small class="c-mute">{{ $t('onboarding.welcome.roleGiverBody') }}</small>
          </div>
        </div>
      </section>

      <ol class="wl-steps">
        <li v-for="(s, i) in state.steps" :key="s.key" class="wl-step" :class="{ done: s.done, cur: s.key === cur }" :aria-current="s.key === cur ? 'step' : undefined">
          <div class="wl-head">
            <span class="wl-no" aria-hidden="true">{{ pad(i + 1) }}</span>
            <strong class="c-grow">{{ $t(`onboarding.steps.${s.key}.title`) }}</strong>
            <span class="wl-st">{{ s.done ? $t('onboarding.welcome.stepDone') : s.key === cur ? $t('onboarding.welcome.stepNow') : $t('onboarding.welcome.stepTodo') }}</span>
          </div>
          <div v-if="s.key === cur" class="wl-body">
            <p class="c-mute">{{ $t(`onboarding.steps.${s.key}.body`) }}</p>
            <form v-if="s.key === 'profile'" @submit.prevent="saveProfile">
              <label class="c-field"><span>{{ $t('settings.displayName') }}</span><input v-model="name" maxlength="50" required><small class="c-mute">{{ $t('onboarding.welcome.nameHint') }}</small></label>
              <label class="c-field"><span>{{ $t('settings.social.handle') }}</span><input v-model="handle" maxlength="30" autocapitalize="none" autocomplete="off" :placeholder="$t('settings.social.handlePh')"></label>
              <p v-if="hBad" class="c-err" role="alert">{{ $t('settings.social.handleBadHint') }}</p>
              <label class="c-field"><span>{{ $t('settings.social.bio') }}</span><textarea v-model="bio" maxlength="160" rows="3" /></label>
              <div class="c-field" role="group" aria-labelledby="wl-bday"><span id="wl-bday">{{ $t('settings.social.birthday') }}</span>
                <div class="c-row">
                  <select v-model="bMonth" :aria-label="$t('settings.social.monthAria')"><option value="">{{ $t('settings.social.monthOpt') }}</option><option v-for="m in 12" :key="m" :value="pad(m)">{{ $t('settings.social.monthN', { n: m }) }}</option></select>
                  <select v-model="bDay" :aria-label="$t('settings.social.dayAria')"><option value="">{{ $t('settings.social.dayOpt') }}</option><option v-for="d in bDays" :key="d" :value="pad(d)">{{ $t('settings.social.dayN', { n: d }) }}</option></select>
                </div>
              </div>
              <p class="c-mute">{{ $t('onboarding.welcome.profileHint') }}</p>
              <p v-if="err" class="c-err" role="alert">{{ err }}</p>
              <button class="c-btn primary block" :disabled="busy || hBad">{{ busy ? $t('onboarding.welcome.profileSaving') : $t('onboarding.welcome.profileSave') }}</button>
            </form>
            <NuxtLink v-else :to="tourTarget(s.key, state.wishlist_id)" class="c-btn primary block">{{ $t(`onboarding.steps.${s.key}.cta`) }}</NuxtLink>
          </div>
        </li>
      </ol>

      <section v-if="!cur" class="c-card wl-fin">
        <h2 class="c-h2">{{ $t('onboarding.done.title') }}</h2>
        <p class="c-mute">{{ $t('onboarding.done.body') }}</p>
        <p v-if="err" class="c-err" role="alert">{{ err }}</p>
        <button type="button" class="c-btn primary block" :disabled="busy" @click="skip">{{ $t('onboarding.done.finish') }}</button>
      </section>
      <p v-else class="wl-skip c-mute">{{ $t('onboarding.welcome.skipLead') }}
        <button type="button" class="c-btn small plain" :disabled="busy" @click="skip">{{ $t('onboarding.welcome.skipBtn') }}</button>
      </p>
      <p v-if="err && cur && cur !== 'profile'" class="c-err" role="alert">{{ err }}</p>
    </template>
  </main>
</template>
<style scoped>
.wl-intro{margin:0 0 20px;animation:rise .5s var(--ease) both}
.wl-hello{margin:0 0 6px;font-size:20px;font-weight:800}
.wl-roles{display:grid;grid-template-columns:1fr 1fr;gap:10px;margin-top:14px}
.wl-role{display:grid;gap:4px;padding:12px;border:1px solid var(--line);background:var(--card)}
.wl-role.on{border-color:var(--ink-line)}
.wl-role .x-tag b{color:var(--ok)}
.wl-role strong{font-size:16px}
.wl-steps{list-style:none;margin:0 0 16px;padding:0;border-top:1px solid var(--ink-line)}
.wl-step{border-bottom:1px solid var(--line)}
.wl-head{display:flex;align-items:center;gap:12px;padding:14px 0}
.wl-no{font:900 22px/1 var(--font);letter-spacing:-.04em;color:transparent;-webkit-text-stroke:1.5px var(--ink-line);font-variant-numeric:tabular-nums;min-width:30px}
.wl-step.done .wl-no{color:var(--fg);-webkit-text-stroke:0}
.wl-step.cur .wl-no{color:var(--red-text);-webkit-text-stroke:0}
.wl-step.done strong{color:var(--mute);text-decoration:line-through;text-decoration-thickness:1px}
.wl-st{font:600 11px/1.4 var(--mono);letter-spacing:.1em;text-transform:uppercase;color:var(--mute)}
.wl-step.cur .wl-st{color:var(--red-text)}
.wl-step.done .wl-st{color:var(--ok)}
.wl-body{padding:0 0 18px 42px}
.wl-body > .c-mute:first-child{margin-top:0}
.wl-skip{display:flex;flex-wrap:wrap;align-items:center;gap:4px;margin:8px 0 24px}
.wl-fin{margin-bottom:24px}
@media (max-width:480px){.wl-body{padding-left:0}.wl-roles{grid-template-columns:1fr}}
</style>
