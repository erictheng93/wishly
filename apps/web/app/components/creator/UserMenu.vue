<script setup lang="ts">
const { user, logout } = useAuth()
const { count } = useFriendRequests()
const open = ref(false)
const root = ref<HTMLElement>()
const trigger = ref<HTMLButtonElement>()
const panelId = useId()
const name = computed(() => user.value?.display_name || user.value?.email || '?')
function away(e: Event) { if (!root.value?.contains(e.target as Node)) open.value = false }
function dismiss(e: KeyboardEvent) {
  if (!open.value) return
  e.preventDefault()
  e.stopPropagation()
  open.value = false
  trigger.value?.focus()
}
function focusout(e: FocusEvent) {
  // WebKit 點按鈕不給焦點，relatedTarget 為 null；點外面交給 away 處理
  if (e.relatedTarget && !root.value?.contains(e.relatedTarget as Node)) open.value = false
}
onMounted(() => document.addEventListener('click', away))
onBeforeUnmount(() => document.removeEventListener('click', away))
watch(() => useRoute().fullPath, () => { open.value = false })
</script>
<template>
  <div v-if="user" ref="root" class="um" @keydown.esc="dismiss" @focusout="focusout">
    <button ref="trigger" type="button" class="um-btn" :aria-label="count > 0 ? `${$t('common.accountMenu')}（${$t('common.friendRequests', { n: count })}）` : $t('common.accountMenu')" :aria-expanded="open" :aria-controls="panelId" @click="open = !open">{{ name.slice(0, 1).toUpperCase() }}<i v-if="count > 0" class="um-dot" aria-hidden="true" /></button>
    <nav v-show="open" :id="panelId" class="um-pop" :aria-label="$t('common.accountMenu')">
      <component :is="user.handle ? 'NuxtLink' : 'div'" :to="user.handle ? `/u/${user.handle}` : undefined" class="um-who">
        <b>{{ user.display_name }}</b><small v-if="user.handle">@{{ user.handle }}</small>
      </component>
      <NuxtLink to="/friends"><span>{{ $t('common.friends') }}<CreatorCountBadge :n="count" /></span></NuxtLink>
      <NuxtLink to="/me/wallet">{{ $t('common.points') }}</NuxtLink>
      <NuxtLink to="/settings">{{ $t('common.settings') }}</NuxtLink>
      <NuxtLink v-if="user.is_staff" to="/admin">{{ $t('common.admin') }}</NuxtLink>
      <button type="button" @click="logout()">{{ $t('settings.logout') }}</button>
    </nav>
  </div>
</template>
<style scoped>
.um{position:relative}
.um-btn{position:relative;width:44px;height:44px;border:0;background:none;color:var(--red-text);font:inherit;font-weight:700;font-size:16px;cursor:pointer;border-radius:50%}
.um-btn::before{content:"";position:absolute;inset:6px;border:2px solid var(--red);border-radius:50%;pointer-events:none}
.um-dot{position:absolute;top:6px;right:6px;width:10px;height:10px;border-radius:50%;background:var(--red);border:2px solid var(--bg,#fff)}
.um-pop{position:absolute;right:0;top:100%;min-width:220px;z-index:30;display:flex;flex-direction:column;background:var(--bg,#fff);border:2px solid var(--red);box-shadow:0 6px 20px rgba(0,0,0,.2)}
.um-pop>a,.um-pop>button,.um-who{display:flex;flex-direction:column;justify-content:center;min-height:44px;padding:0 14px;border:0;background:none;color:var(--fg);font:inherit;font-weight:700;text-align:left;cursor:pointer}
.um-who{padding:8px 14px;border-bottom:1px solid var(--red)}
.um-who small{font-weight:400;opacity:.7}
</style>
