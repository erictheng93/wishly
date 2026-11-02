<script setup lang="ts">
// 儀表板的帳號提示，一次一張。email：沒有 Email 的帳號一旦 LINE / Google 不能用就回不來，也收不到通知信；
// name：驗證碼開戶的顯示名稱取自信箱 @ 前段（後端截 50 字），朋友會看到。關閉後這台裝置不再顯示（依帳號、種類記）
const { user } = useAuth()
const kind = computed(() => {
  const u = user.value
  if (!u) return null
  if (!u.email) return 'email'
  return u.display_name === [...u.email.split('@')[0]].slice(0, 50).join('') ? 'name' : null
})
const key = computed(() => `ws_${kind.value}_nudge_off:${user.value?.id}`)
const off = ref(true) // 讀到偏好前先不顯示，已關閉的人才不會閃一下
onMounted(() => { off.value = !!getPref(key.value) })
function dismiss() { off.value = true; setPref(key.value, '1') }
</script>
<template>
  <section v-if="kind && !off" class="c-card an" :aria-label="$t(`creator.nudge.${kind}.title`)">
    <p class="c-m0"><strong>{{ $t(`creator.nudge.${kind}.title`) }}</strong></p>
    <p class="c-mute an-body">{{ $t(`creator.nudge.${kind}.body`) }}</p>
    <div class="c-row">
      <NuxtLink to="/settings" class="c-btn primary small">{{ $t(`creator.nudge.${kind}.link`) }}</NuxtLink>
      <button type="button" class="c-btn small" @click="dismiss">{{ $t('creator.nudge.later') }}</button>
    </div>
  </section>
</template>
<style scoped>
.an{margin:0 0 16px}
.an-body{margin:4px 0 12px}
</style>
