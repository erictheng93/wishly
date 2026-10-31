<script setup lang="ts">
import '~/assets/guest.css'
// 訪客頁會被 CDN 快取，SSR 一律畫匿名版；登入者的帳號選單（錢包、設定、登出）只在瀏覽器端補上。
// 只在有「可能已登入」旗標時才打 /me，避免一般訪客每頁多一次 401。
const { fetchMe } = useAuth()
const hint = useAuthHint()
onMounted(() => { if (hint.value) fetchMe().catch(() => {}) })
</script>
<template>
  <!-- 品牌字標在左、導覽在中、語系與主題在右；導覽以等寬小字呈現，目前頁面紅色底線 -->
  <header class="g-top"><NuxtLink to="/" class="g-brand">Wishly</NuxtLink><nav class="g-nav"><NuxtLink to="/explore">{{ $t('common.explore') }}</NuxtLink><NuxtLink to="/inspire">{{ $t('common.inspire') }}</NuxtLink><NuxtLink to="/brands">{{ $t('common.brands') }}</NuxtLink><NuxtLink to="/me/claims">{{ $t('common.myClaims') }}</NuxtLink></nav><span class="g-tools"><LocaleSwitch /><ThemeToggle /><ClientOnly><CreatorUserMenu /></ClientOnly></span></header>
  <!-- 登入且新手導覽進行中才出現（訪客頁為 SSR，導覽只在 client 判斷） -->
  <ClientOnly><div class="g-tour"><OnboardingGuide /></div></ClientOnly>
</template>
