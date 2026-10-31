<script setup lang="ts">
// 設定頁的「此裝置接收推播」區塊（後端 push.rs）。伺服器未啟用或瀏覽器不支援時整區不顯示。
const { status, enabled, denied, busy, error, init, set } = usePush()
onMounted(init)
</script>
<template>
  <section v-if="status === 'ready' || status === 'needs-install'" class="c-card">
    <h2 class="c-h2">{{ $t('settings.push.title') }}</h2>
    <p v-if="status === 'needs-install'" class="c-mute">{{ $t('settings.push.install') }}</p>
    <template v-else>
      <label class="c-switch">
        <input type="checkbox" :checked="enabled" :disabled="busy" @change="set(($event.target as HTMLInputElement).checked)">
        <span>{{ $t('settings.push.label') }}<br><small class="c-mute">{{ $t('settings.push.hint') }}</small></span>
      </label>
      <p v-if="denied" class="c-mute" role="status">{{ $t('settings.push.denied') }}</p>
      <p v-if="error" class="c-err" role="alert">{{ error }}</p>
    </template>
  </section>
</template>
