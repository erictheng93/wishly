<script setup lang="ts">
// 03 §5.8：捐助成功畫面的「加入主畫面」提示。環境判斷在 usePwaContext；line / standalone 不顯示（LINE 的外開引導另有橫幅）。
import { sendPwaEvent } from '~/composables/usePwaContext'

const props = defineProps<{ context: 'ios' | 'android' | 'line' | 'standalone', source: 'claim' | 'fund' }>()
const emit = defineEmits<{ dismiss: [permanent: boolean], install: [] }>()
const show = props.context === 'ios' || props.context === 'android'
// 提示實際顯示時才送一次
onMounted(() => { if (show) sendPwaEvent({ event: 'pwa_install_prompted', platform: props.context as 'ios' | 'android', context: props.source }) })
</script>

<template>
  <div v-if="show" class="g-banner js-install-hint" role="region" :aria-label="$t('guest.slug.installTitle')">
    <strong>{{ $t('guest.slug.installTitle') }}</strong>
    <p>{{ context === 'ios' ? $t('guest.slug.installIos') : $t('guest.slug.installAndroid') }}</p>
    <button v-if="context === 'android'" class="g-btn sm" @click="emit('install')">{{ $t('guest.slug.installAdd') }}</button>
    <div>
      <button class="g-link" @click="emit('dismiss', false)">{{ $t('guest.slug.installLater') }}</button>
      <button class="g-link" @click="emit('dismiss', true)">{{ $t('guest.slug.installNever') }}</button>
    </div>
  </div>
</template>
