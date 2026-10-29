<script setup lang="ts">
const { user, logout } = useAuth()
const { count } = useFriendRequests()
const { current, orgs, switchTo } = useOrgMode()
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
    <button ref="trigger" type="button" class="um-btn" :aria-label="`${name}，${$t('common.accountMenu')}${count > 0 ? `（${$t('common.friendRequests', { n: count })}）` : ''}`" :aria-expanded="open" :aria-controls="panelId" @click="open = !open"><span class="um-av">{{ name.slice(0, 1).toUpperCase() }}<i v-if="count > 0" class="um-dot" aria-hidden="true" /></span><span class="um-name">{{ name }}</span><span class="um-caret" aria-hidden="true">▾</span></button>
    <nav v-show="open" :id="panelId" class="um-pop" :aria-label="$t('common.accountMenu')">
      <component :is="user.handle ? 'NuxtLink' : 'div'" :to="user.handle ? `/u/${user.handle}` : undefined" class="um-who">
        <b>{{ user.display_name }}</b><small v-if="user.handle">@{{ user.handle }}</small>
      </component>
      <template v-if="orgs.length">
        <button type="button" :aria-pressed="current === null" @click="switchTo(null)">{{ $t('org.personal') }}</button>
        <button v-for="o in orgs" :key="o.id" type="button" :aria-pressed="current === o.id" @click="switchTo(o.id)">{{ o.name }}</button>
      </template>
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
/* 身分晶片：低調墨線——方形首字、細線框，與頁首其他無框按鈕同一語氣；有待辦才出現紅點 */
.um-btn{display:inline-flex;align-items:center;gap:8px;height:44px;padding:0 8px 0 6px;border:1px solid var(--line);background:transparent;color:var(--fg);font:inherit;font-weight:700;font-size:13px;letter-spacing:.02em;cursor:pointer;transition:border-color var(--t) var(--ease),background var(--t) var(--ease),box-shadow var(--t) var(--ease)}
.um-btn[aria-expanded=true]{border-color:var(--ink-line);background:var(--bg-alt)}
.um-av{position:relative;flex:none;display:grid;place-items:center;width:28px;height:28px;border:1px solid var(--ink-line);background:var(--card);color:var(--fg);font:700 13px/1 var(--mono)}
.um-name{max-width:8em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.um-caret{color:var(--mute);font-size:10px;transition:transform var(--t) var(--ease)}
.um-btn[aria-expanded=true] .um-caret{transform:rotate(180deg)}
@media (max-width:479px){.um-name{max-width:3em}.um-caret{display:none}.c-head .um .um-btn{padding:0 7px}}
.um-dot{position:absolute;top:-4px;right:-4px;width:9px;height:9px;background:var(--red);border:2px solid var(--bg,#fff);box-sizing:content-box}
.um-pop{position:absolute;right:0;top:calc(100% + 8px);min-width:224px;z-index:30;display:flex;flex-direction:column;background:var(--card);border:1px solid var(--ink-line);box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line);animation:rise .22s var(--ease)}
.um-pop>a,.um-pop>button,.um-who{display:flex;flex-direction:column;justify-content:center;min-height:44px;padding:0 16px;border:0;border-top:1px solid var(--line);background:none;color:var(--fg);font:inherit;font-size:14px;font-weight:700;text-align:left;cursor:pointer;transition:background var(--t) var(--ease),padding var(--t) var(--ease)}
.um-pop>a>span{display:inline-flex;align-items:center}
.um-who{padding:10px 16px;border-top:0;border-bottom:1px solid var(--ink-line);background:var(--bg-alt)}
.um-who b{font-size:15px}
.um-pop>button[aria-pressed=true]{color:var(--red-text);box-shadow:inset 3px 0 var(--red)}
.um-pop>button:last-child{color:var(--mute);font-weight:600}
.um-who small{font:500 11px/1.4 var(--mono);letter-spacing:.06em;color:var(--mute)}
@media (hover:hover) and (pointer:fine){
  .um-btn:hover{border-color:var(--ink-line)}
  .um-pop>a:hover,.um-pop>button:hover{background:var(--bg-alt);padding-left:20px}
}
</style>
