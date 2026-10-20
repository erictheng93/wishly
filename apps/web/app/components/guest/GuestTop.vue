<script setup lang="ts">import '~/assets/guest.css'</script>
<template>
  <header class="g-top"><NuxtLink to="/">Wishly</NuxtLink><NuxtLink to="/explore">{{ $t('common.explore') }}</NuxtLink><NuxtLink to="/inspire">{{ $t('common.inspire') }}</NuxtLink><NuxtLink to="/brands">{{ $t('common.brands') }}</NuxtLink><NuxtLink to="/me/claims">{{ $t('common.myClaims') }}</NuxtLink><LocaleSwitch /><ThemeToggle /></header>
</template>
