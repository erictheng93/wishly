<script setup lang="ts">
const { user, logout } = useAuth()
const open = ref(false)
const root = ref<HTMLElement>()
const name = computed(() => user.value?.display_name || user.value?.email || '?')
function away(e: Event) { if (!root.value?.contains(e.target as Node)) open.value = false }
onMounted(() => document.addEventListener('click', away))
onBeforeUnmount(() => document.removeEventListener('click', away))
watch(() => useRoute().fullPath, () => { open.value = false })
</script>
<template>
  <div v-if="user" ref="root" class="um" @keydown.esc="open = false">
    <button type="button" class="um-btn" :aria-label="$t('common.accountMenu')" :aria-expanded="open" aria-haspopup="menu" @click="open = !open">{{ name.slice(0, 1).toUpperCase() }}</button>
    <div v-if="open" class="um-pop" role="menu">
      <component :is="user.handle ? 'NuxtLink' : 'div'" :to="user.handle ? `/u/${user.handle}` : undefined" class="um-who">
        <b>{{ user.display_name }}</b><small v-if="user.handle">@{{ user.handle }}</small>
      </component>
      <NuxtLink to="/friends" role="menuitem">{{ $t('common.friends') }}</NuxtLink>
      <NuxtLink to="/me/wallet" role="menuitem">{{ $t('common.points') }}</NuxtLink>
      <NuxtLink to="/settings" role="menuitem">{{ $t('common.settings') }}</NuxtLink>
      <NuxtLink v-if="user.is_staff" to="/admin" role="menuitem">{{ $t('common.admin') }}</NuxtLink>
      <button type="button" role="menuitem" @click="logout">{{ $t('settings.logout') }}</button>
    </div>
  </div>
</template>
<style scoped>
.um{position:relative}
.um-btn{width:44px;height:44px;border:0;background:none;color:var(--red-text);font:inherit;font-weight:700;font-size:16px;cursor:pointer;border-radius:50%}
.um-btn::before{content:"";position:absolute;inset:6px;border:2px solid var(--red);border-radius:50%;pointer-events:none}
.um-pop{position:absolute;right:0;top:100%;min-width:220px;z-index:30;display:flex;flex-direction:column;background:var(--bg,#fff);border:2px solid var(--red);box-shadow:0 6px 20px rgba(0,0,0,.2)}
.um-pop>a,.um-pop>button,.um-who{display:flex;flex-direction:column;justify-content:center;min-height:44px;padding:0 14px;border:0;background:none;color:var(--fg);font:inherit;font-weight:700;text-align:left;cursor:pointer}
.um-who{padding:8px 14px;border-bottom:1px solid var(--red)}
.um-who small{font-weight:400;opacity:.7}
</style>
