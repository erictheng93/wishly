<script setup lang="ts">
import { safeRedirect } from '~/utils/redirect'
const { user } = useAuth()
const { unread, latest, readOne, readAll } = useInbox()
const { t } = useI18n()
const route = useRoute()
const open = ref(false)
const root = ref<HTMLElement>()
const trigger = ref<HTMLButtonElement>()
const panelId = useId()
const label = computed(() => unread.value > 0 ? `${t('common.notifications')}（${t('common.unreadCount', { n: unread.value })}）` : t('common.notifications'))
function away(e: Event) { if (!root.value?.contains(e.target as Node)) open.value = false }
function dismiss(e: KeyboardEvent) {
  if (!open.value) return
  e.preventDefault(); e.stopPropagation()
  open.value = false
  trigger.value?.focus()
}
function focusout(e: FocusEvent) {
  // WebKit 點按鈕不給焦點，relatedTarget 為 null；點外面交給 away 處理
  if (e.relatedTarget && !root.value?.contains(e.relatedTarget as Node)) open.value = false
}
async function go(n: any) {
  open.value = false
  await readOne(n)
  const to = safeRedirect(n.link, '')
  if (to) await navigateTo(to)
}
onMounted(() => document.addEventListener('click', away))
onBeforeUnmount(() => document.removeEventListener('click', away))
watch(() => route.fullPath, () => { open.value = false })
</script>
<template>
  <div v-if="user" ref="root" class="c-bell" @keydown.esc="dismiss" @focusout="focusout">
    <button ref="trigger" type="button" class="c-bell-btn" :aria-label="label" :aria-expanded="open" :aria-controls="panelId" @click="open = !open">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linejoin="miter" aria-hidden="true"><path d="M6 17V11a6 6 0 0 1 12 0v6l2 2H4l2-2zM10 21h4" /></svg>
      <b v-if="unread > 0" class="c-bell-n" aria-hidden="true">{{ unread > 99 ? '99+' : unread }}</b>
    </button>
    <section v-show="open" :id="panelId" class="c-bell-pop" :aria-label="$t('common.notifications')">
      <p v-if="!latest.length" class="c-bell-empty">{{ $t('me.notifications.empty') }}</p>
      <ul v-else>
        <li v-for="n in latest" :key="n.id" :class="{ unread: !n.read_at }">
          <button type="button" @click="go(n)"><span v-if="!n.read_at" class="sr-only">{{ $t('me.notifications.unread') }}：</span><b>{{ n.title }}</b><span v-if="n.body" class="c-bell-body">{{ n.body }}</span></button>
        </li>
      </ul>
      <footer>
        <button type="button" :disabled="!unread" @click="readAll">{{ $t('me.notifications.readAll') }}</button>
        <NuxtLink to="/me/notifications">{{ $t('me.notifications.viewAll') }}</NuxtLink>
      </footer>
    </section>
  </div>
</template>
<style>
/* 站內通知鈴鐺：方形、細線；未讀徽章用訊號紅。創建者頁（c-head）與訪客頁（g-top）共用，樣式隨元件載入 */
.c-bell{position:relative;display:inline-block}
.c-head .c-bell{margin-left:auto}.c-head .c-bell~.um{margin-left:0}
.c-bell .c-bell-btn{position:relative;display:inline-grid;place-items:center;width:44px;height:44px;padding:0;border:0;background:none;color:var(--fg);cursor:pointer}
.c-bell-n{position:absolute;top:6px;right:2px;min-width:16px;height:16px;padding:0 3px;box-sizing:border-box;background:var(--red);color:#fff;font:700 10px/16px var(--mono);text-align:center}
.c-bell-pop{position:absolute;right:0;top:calc(100% + 8px);z-index:30;width:340px;max-width:calc(100vw - 16px);background:var(--card);border:1px solid var(--ink-line);box-shadow:6px 6px 0 var(--bg-alt),6px 6px 0 1px var(--line);animation:rise .22s var(--ease);white-space:normal}
.c-bell-pop ul{list-style:none;margin:0;padding:0}
.c-bell-pop li+li{border-top:1px solid var(--line)}
.c-bell-pop li button{display:flex;flex-direction:column;gap:2px;width:100%;min-height:44px;padding:10px 16px 10px 20px;border:0;background:none;color:var(--fg);font:inherit;font-size:14px;text-align:left;cursor:pointer;position:relative}
.c-bell-pop li.unread button::before{content:"";position:absolute;left:8px;top:16px;width:6px;height:6px;background:var(--red)}
.c-bell-pop li.unread b{font-weight:800}.c-bell-pop li b{font-weight:600;overflow-wrap:anywhere}
.c-bell-body{color:var(--mute);font-size:13px;line-height:1.5;display:-webkit-box;-webkit-line-clamp:2;line-clamp:2;-webkit-box-orient:vertical;overflow:hidden;overflow-wrap:anywhere}
.c-bell-empty{margin:0;padding:16px;color:var(--mute);font-size:14px}
.c-bell-pop footer{display:flex;justify-content:space-between;align-items:center;border-top:1px solid var(--ink-line)}
.c-bell-pop footer>*{display:inline-flex;align-items:center;min-height:44px;padding:0 16px;border:0;background:none;color:var(--fg);font:600 12px/1 var(--mono);letter-spacing:.06em;cursor:pointer}
.c-bell-pop footer button:disabled{color:var(--mute);cursor:default}
@media (max-width:479px){.c-bell{position:static}.c-bell-pop{left:8px;right:8px;width:auto;max-width:none;top:100%}}
</style>
