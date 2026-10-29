<script setup lang="ts">import '~/assets/guest.css'</script>
<template>
  <!-- 品牌字標在左、導覽在中、語系與主題在右；導覽以等寬小字呈現，目前頁面紅色底線 -->
  <header class="g-top"><NuxtLink to="/" class="g-brand">Wishly</NuxtLink><nav class="g-nav"><NuxtLink to="/explore">{{ $t('common.explore') }}</NuxtLink><NuxtLink to="/inspire">{{ $t('common.inspire') }}</NuxtLink><NuxtLink to="/brands">{{ $t('common.brands') }}</NuxtLink><NuxtLink to="/me/claims">{{ $t('common.myClaims') }}</NuxtLink></nav><span class="g-tools"><LocaleSwitch /><ThemeToggle /></span></header>
</template>
