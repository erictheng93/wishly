<script setup lang="ts">
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('common.friends') })
const { api } = useApi()
const { count: reqCount } = useFriendRequests()
const data = ref<any>({ friends: [], incoming: [], outgoing: [] })
const loading = ref(true), err = ref(''), msg = ref('')
const invite = ref<any>(null), copied = ref(false)
const unfriend = ref<any>(null), ufBusy = ref(false), ufErr = ref('')
const feed = ref<any>(null), feedErr = ref('')

const msgOf = (e: any) => apiErrMsg(e)
async function load() {
  try { data.value = await api('/friends'); reqCount.value = data.value.incoming.length } catch (e) { err.value = msgOf(e) } finally { loading.value = false }
}
// 動態讀取失敗不影響好友管理
async function loadFeed() {
  feedErr.value = ''
  try { feed.value = await api('/friends/feed') } catch (e) { feedErr.value = msgOf(e) }
}
onMounted(() => { load(); loadFeed() })
// 動作後重新載入清單；失敗顯示錯誤
async function act(f: () => Promise<any>) {
  err.value = msg.value = ''
  try { await f(); await load() } catch (e) { err.value = msgOf(e) }
}
// 即時搜尋（typeahead）：停止輸入 300ms 後才查全站（debounce）；同一字串不重查；seq 丟棄過期回應。
// 排序（自己 → 好友 → 共同好友 → 陌生人）與模糊比對都在後端 GET /friends/search。
// 好友申請只對搜尋結果中的使用者以 id 送出，不會拿輸入的文字盲目送申請。
type Hit = { id: string, display_name: string, handle: string | null, avatar_url: string | null, relation: 'self' | 'friend' | 'outgoing' | 'incoming' | 'none', mutual: number }
const SEARCH_DEBOUNCE_MS = 300
const q = ref(''), hits = ref<Hit[] | null>(null), searching = ref(false), rowBusy = ref(''), active = ref(-1)
let seq = 0, lastKey = '', timer: ReturnType<typeof setTimeout> | undefined
const normKey = (v: string) => v.trim().replace(/\s+/g, ' ').toLowerCase()
watch(q, (v) => {
  const key = normKey(v)
  if (key === lastKey && (hits.value || searching.value)) return // 只多打了空白 / 換大小寫
  clearTimeout(timer); const my = ++seq; lastKey = key; active.value = -1
  if (!key) { hits.value = null; searching.value = false; return }
  searching.value = true
  timer = setTimeout(async () => {
    try { const r = await api('/friends/search', { query: { q: key } }); if (my === seq) { hits.value = r.users; err.value = '' } }
    catch (e) { if (my === seq) { hits.value = null; lastKey = ''; err.value = msgOf(e) } }
    finally { if (my === seq) searching.value = false }
  }, SEARCH_DEBOUNCE_MS)
})
onBeforeUnmount(() => clearTimeout(timer))
const relText = (u: Hit) => u.relation === 'none' ? (u.mutual ? t('me.friends.mutual', { n: u.mutual }, u.mutual) : '')
  : t({ self: 'me.friends.relSelf', friend: 'me.friends.relFriend', outgoing: 'me.friends.relOutgoing', incoming: 'me.friends.relIncoming' }[u.relation])
// 比對到的片段加底線（不分大小寫）：整串命中優先，否則標第一個命中的詞；以 index 切開，不用 v-html
function parts(name: string) {
  const k = normKey(q.value), low = name.toLowerCase()
  const cands = k ? [k, ...k.split(' ').map(w => w.replace(/^@+/, '')).filter(Boolean)] : []
  for (const w of cands) {
    const i = low.indexOf(w)
    if (i >= 0) return [name.slice(0, i), name.slice(i, i + w.length), name.slice(i + w.length)]
  }
  return [name, '', '']
}
// 鍵盤操作：↑↓ 移動、Enter 對選中的人加好友 / 接受邀請（其餘關係開個人頁）、Esc 清空。
// 沒有選中任何結果時 Enter 不做事。
function onKey(e: KeyboardEvent) {
  const list = hits.value ?? []
  if (e.key === 'Escape') { q.value = ''; return }
  if (!list.length) return
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault()
    const d = e.key === 'ArrowDown' ? 1 : -1
    active.value = (active.value + d + list.length + (active.value < 0 && d < 0 ? 1 : 0)) % list.length
    document.getElementById(`fr-hit-${active.value}`)?.scrollIntoView({ block: 'nearest' })
  } else if (e.key === 'Enter') {
    const u = list[active.value]
    if (!u || searching.value || rowBusy.value) return
    e.preventDefault()
    if (u.relation === 'none') addHit(u)
    else if (u.relation === 'incoming') acceptHit(u)
    else navigateTo(profilePath(u))
  }
}
async function addHit(u: Hit) {
  rowBusy.value = u.id; err.value = ''
  try {
    const r = await api('/friends/requests', { method: 'POST', body: { to: u.id } })
    u.relation = r?.status === 'friends' ? 'friend' : 'outgoing'
    await load(); if (u.relation === 'friend') loadFeed()
  } catch (e) { err.value = msgOf(e) } finally { rowBusy.value = '' }
}
async function acceptHit(u: Hit) {
  const r = data.value.incoming.find((x: any) => x.user.id === u.id)
  if (!r) return
  rowBusy.value = u.id
  await accept(r); rowBusy.value = ''
  if (!err.value) u.relation = 'friend'
}
const accept = (r: any) => act(async () => { await api(`/friends/requests/${r.id}/accept`, { method: 'POST' }); loadFeed() })
const dropReq = (r: any) => act(() => api(`/friends/requests/${r.id}`, { method: 'DELETE' }))
async function doUnfriend() {
  ufBusy.value = true; ufErr.value = ''
  try { await api(`/friends/${unfriend.value.id}`, { method: 'DELETE' }); unfriend.value = null; await load(); loadFeed() }
  catch (e) { ufErr.value = msgOf(e) } finally { ufBusy.value = false }
}
const makeInvite = () => act(async () => { invite.value = await api('/friends/invites', { method: 'POST' }); copied.value = false })
const revoke = () => act(async () => { await api('/friends/invites', { method: 'DELETE' }); invite.value = null })
async function copy() {
  try { await copyText(invite.value.url); copied.value = true } catch { err.value = t('me.friends.copyFail') }
}

const rtf = computed(() => new Intl.RelativeTimeFormat(intlLocale(), { numeric: 'auto' }))
function ago(iso: string) {
  const m = Math.min(-1, Math.round((Date.parse(iso) - Date.now()) / 60000)) // 最少「1 分鐘前」
  if (m > -60) return rtf.value.format(m, 'minute')
  const h = Math.round(m / 60)
  return h > -24 ? rtf.value.format(h, 'hour') : rtf.value.format(Math.round(h / 24), 'day')
}
const daysLeft = (iso: string) => Math.max(0, Math.ceil((Date.parse(iso) - Date.now()) / 86400000))
const initial = (n: string) => [...n.trim()][0] ?? '?'
const price = (n: number) => `NT$ ${n.toLocaleString('en-US')}`
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('common.friends')" back="/dashboard" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('me.loading') }}</p>
    <div v-else class="fr" :class="{ 'fr-lonely': feed && !feed.events.length }">
      <div class="fr-top">
        <section v-if="data.incoming.length" class="c-card">
          <h2 class="c-h2">{{ $t('me.friends.incoming') }}</h2>
          <div v-for="r in data.incoming" :key="r.id" class="c-row wrap c-mb">
            <span class="c-grow"><NuxtLink :to="profilePath(r.user)"><strong>{{ r.user.display_name }}</strong></NuxtLink><small v-if="r.user.handle" class="c-mute"> @{{ r.user.handle }}</small></span>
            <button class="c-btn primary" @click="accept(r)">{{ $t('me.friends.accept') }}</button>
            <button class="c-btn" @click="dropReq(r)">{{ $t('me.friends.decline') }}</button>
          </div>
        </section>
        <section v-if="feed" class="c-card" :class="{ 'fr-no-friends': !data.friends.length }">
          <h2 class="c-h2">{{ $t('me.friends.birthdays') }}</h2>
          <p v-if="!feed.birthdays.length" class="c-mute c-m0">{{ $t('me.friends.bdNone') }}</p>
          <div v-for="b in feed.birthdays" :key="b.id" class="c-row c-mb">
            <span class="fr-date">{{ +b.birthday.slice(0, 2) }}/{{ +b.birthday.slice(3) }}</span>
            <span class="c-grow">
              <NuxtLink :to="profilePath(b)"><strong>{{ b.display_name }}</strong></NuxtLink>
              <span class="c-mute"> · {{ b.days ? $t('me.friends.bdIn', { n: b.days }, b.days) : $t('me.friends.bdToday') }}</span>
            </span>
          </div>
        </section>
      </div>

      <section class="fr-feed" :aria-labelledby="'fr-feed-h'">
        <h2 id="fr-feed-h" class="c-h2 x-h">{{ $t('me.friends.feedTitle') }}</h2>
        <p v-if="feedErr" class="c-err" role="alert">{{ feedErr }}</p>
        <p v-else-if="!feed" class="c-mute" role="status">{{ $t('me.loading') }}</p>
        <div v-else-if="!data.friends.length" class="c-mask">
          <p class="c-m0">{{ $t('me.friends.feedNoFriends') }}</p>
          <a href="#fr-invite" class="c-btn primary c-mt12">{{ $t('me.friends.feedInvite') }}</a>
        </div>
        <p v-else-if="!feed.events.length" class="c-mask">{{ $t('me.friends.feedEmpty') }}</p>
        <article v-for="(e, i) in feed?.events" :key="i" class="c-card">
          <div class="c-row fr-who">
            <span class="fr-ava" aria-hidden="true">{{ initial(e.owner.display_name) }}</span>
            <div class="c-grow">
              <NuxtLink :to="profilePath(e.owner)"><strong>{{ e.owner.display_name }}</strong></NuxtLink>
              <template v-if="e.kind === 'delivered'"> {{ $t('me.friends.delivered') }}</template>
              <template v-else-if="e.wishlist.is_new"> {{ $t('me.friends.newList') }}〈{{ e.wishlist.title }}〉</template>
              <template v-else> {{ $t('me.friends.added', { n: e.new_items }) }}〈{{ e.wishlist.title }}〉</template>
              <div class="c-mute">{{ ago(e.at) }}</div>
            </div>
            <span v-if="e.kind === 'delivered'" class="c-badge delivered">{{ $t('creator.analytics.claimStatus.delivered') }}</span>
            <span v-else-if="e.wishlist.closes_at" class="c-badge fr-hot">{{ daysLeft(e.wishlist.closes_at) ? $t('me.friends.closesIn', { n: daysLeft(e.wishlist.closes_at) }, daysLeft(e.wishlist.closes_at)) : $t('me.friends.closesToday') }}</span>
          </div>
          <NuxtLink :to="`/s/${e.wishlist.slug}`" class="fr-item">
            <img v-if="e.item.image_url" :src="e.item.image_url" alt="" class="c-thumb" loading="lazy">
            <span v-else class="c-thumb" aria-hidden="true">{{ initial(e.item.title) }}</span>
            <span class="c-grow">
              <strong>{{ e.item.title }}</strong>
              <span class="c-mute">
                <template v-if="e.kind === 'delivered'">〈{{ e.wishlist.title }}〉</template>
                <template v-else><template v-if="e.item.unit_price_amount != null">{{ price(e.item.unit_price_amount) }} · </template>{{ e.item.open ? $t('me.friends.open') : $t('me.friends.taken') }}</template>
              </span>
            </span>
          </NuxtLink>
          <template v-if="e.kind === 'items_added'">
            <p class="c-mute c-mt">{{ $t('me.friends.progress', { n: e.wishlist.item_count, pct: e.wishlist.completion_pct }, e.wishlist.item_count) }}</p>
            <div class="c-bar"><i :style="{ width: `${e.wishlist.completion_pct}%` }" /></div>
            <div class="c-row wrap c-mt12">
              <NuxtLink :to="`/s/${e.wishlist.slug}`" class="c-btn primary">{{ e.item.open ? $t('me.friends.give') : $t('me.friends.viewList') }}</NuxtLink>
              <span v-if="e.new_items > 1" class="c-mute">{{ $t('me.friends.more', { n: e.new_items - 1 }) }}</span>
            </div>
          </template>
        </article>
      </section>

      <div class="fr-side">
        <section class="c-card">
          <h2 class="c-h2">{{ $t('me.friends.addTitle') }}</h2>
          <label class="c-field"><span>{{ $t('me.friends.searchLabel') }}</span>
            <input
              v-model="q" type="search" maxlength="120" autocomplete="off" spellcheck="false"
              role="combobox" aria-autocomplete="list" aria-controls="fr-hits" aria-describedby="fr-hint"
              :aria-expanded="!!hits?.length" :aria-activedescendant="active >= 0 ? `fr-hit-${active}` : undefined"
              :placeholder="$t('me.friends.searchPh')" @keydown="onKey"
            >
          </label>
          <p v-if="searching" class="c-mute c-m0" role="status">{{ $t('me.friends.searching') }}</p>
          <p v-else-if="hits?.length" id="fr-hint" class="c-mute c-m0 fr-hint">{{ $t('me.friends.searchHint') }}</p>
          <ul v-if="hits?.length" id="fr-hits" class="fr-hits" role="listbox" :aria-label="$t('me.friends.searchResults')">
            <li v-for="(u, i) in hits" :id="`fr-hit-${i}`" :key="u.id" class="fr-hit" role="option" :aria-selected="i === active" :class="{ on: i === active }" @mouseenter="active = i">
              <img v-if="u.avatar_url" :src="u.avatar_url" alt="" class="fr-ava" loading="lazy">
              <span v-else class="fr-ava" aria-hidden="true">{{ initial(u.display_name) }}</span>
              <span class="c-grow fr-hit-t">
                <NuxtLink :to="profilePath(u)"><strong>{{ parts(u.display_name)[0] }}<mark>{{ parts(u.display_name)[1] }}</mark>{{ parts(u.display_name)[2] }}</strong></NuxtLink>
                <small v-if="u.handle" class="c-mute"> @{{ u.handle }}</small>
                <span v-if="relText(u)" class="c-mute fr-hit-s">{{ relText(u) }}</span>
              </span>
              <button v-if="u.relation === 'none'" class="c-btn primary" :disabled="rowBusy === u.id" :aria-label="`${$t('me.friends.add')} ${u.display_name}`" @click="addHit(u)">{{ $t('me.friends.add') }}</button>
              <button v-else-if="u.relation === 'incoming'" class="c-btn primary" :disabled="rowBusy === u.id" @click="acceptHit(u)">{{ $t('me.friends.accept') }}</button>
              <button v-else-if="u.relation === 'outgoing'" class="c-btn" disabled>{{ $t('me.friends.requested') }}</button>
            </li>
          </ul>
          <p v-else-if="hits && !searching" class="c-mute c-m0" role="status">{{ $t('me.friends.noMatch') }}</p>
        </section>

        <section id="fr-invite" class="c-card">
          <h2 class="c-h2">{{ $t('me.friends.inviteTitle') }}</h2>
          <p class="c-mute">{{ $t('me.friends.inviteHint') }}</p>
          <template v-if="invite">
            <label class="c-field"><span>{{ $t('me.friends.inviteUrl') }}</span><input :value="invite.url" readonly @focus="($event.target as HTMLInputElement).select()"></label>
            <CreatorQr :text="invite.url" />
            <div class="c-row wrap c-mt">
              <button class="c-btn" @click="copy">{{ copied ? $t('me.friends.copied') : $t('me.friends.copy') }}</button>
              <button class="c-btn" @click="makeInvite">{{ $t('me.friends.regen') }}</button>
              <button class="c-btn danger" @click="revoke">{{ $t('me.friends.revoke') }}</button>
            </div>
          </template>
          <template v-else>
            <button class="c-btn primary" @click="makeInvite">{{ $t('me.friends.gen') }}</button>
            <button class="c-btn danger" @click="revoke">{{ $t('me.friends.revokeExisting') }}</button>
          </template>
        </section>

        <section v-if="data.outgoing.length" class="c-card">
          <h2 class="c-h2">{{ $t('me.friends.outgoing') }}</h2>
          <div v-for="r in data.outgoing" :key="r.id" class="c-row wrap c-mb">
            <span class="c-grow"><NuxtLink :to="profilePath(r.user)"><strong>{{ r.user.display_name }}</strong></NuxtLink><small v-if="r.user.handle" class="c-mute"> @{{ r.user.handle }}</small></span>
            <button class="c-btn" @click="dropReq(r)">{{ $t('me.friends.cancelInvite') }}</button>
          </div>
        </section>

        <section class="c-card">
          <h2 class="c-h2">{{ $t('me.friends.mine', { n: data.friends.length }) }}</h2>
          <p v-if="!data.friends.length" class="c-mute">{{ $t('me.friends.none') }}</p>
          <div v-for="f in data.friends" :key="f.id" class="c-row wrap c-mb">
            <span class="c-grow">
              <NuxtLink :to="profilePath(f)"><strong>{{ f.display_name }}</strong></NuxtLink>
              <small v-if="f.handle" class="c-mute"> @{{ f.handle }}</small>
            </span>
            <button class="c-btn" @click="unfriend = f; ufErr = ''">{{ $t('me.friends.unfriend') }}</button>
          </div>
        </section>
      </div>
    </div>
    <CreatorConfirm :open="!!unfriend" :title="$t('me.friends.unfriendTitle', { name: unfriend?.display_name })" :text="$t('me.friends.unfriendText')" :ok="$t('me.friends.unfriend')" danger :busy="ufBusy" :error="ufErr" @close="unfriend = null" @ok="doUnfriend" />
  </main>
</template>
<style scoped>
/* 手機：邀請 / 生日 → 動態 → 加好友與名單；桌面 ≥1100：動態在左，其餘疊在右欄 */
.fr{display:flex;flex-direction:column}
.fr-feed{margin-bottom:12px}
.fr-side .c-card>.c-field:last-child{margin-bottom:0}
.fr-feed .x-h{margin-top:0}
.fr-top .c-h2,.fr-side .c-h2{font-size:15px;font-family:var(--font)}
.fr-side .c-card>form .c-btn,.fr-side .c-card>.c-btn{margin-right:8px}
.fr-hits{list-style:none;margin:8px 0 0;padding:0;border-top:1px solid var(--ink-line)}
.fr-hit{display:flex;gap:10px;align-items:center;padding:8px 6px;border-bottom:1px solid var(--line)}
.fr-hit .fr-ava{width:36px;height:36px;object-fit:cover}
.fr-hit.on{background:var(--bg-alt);box-shadow:inset 3px 0 0 var(--red)}
.fr-hint{font-size:12px}
.fr-hit-t{min-width:0;overflow-wrap:anywhere}
.fr-hit-s{display:block}
.fr-hit mark{background:none;color:inherit;text-decoration:underline;text-decoration-color:var(--red);text-underline-offset:3px}
.fr-who{align-items:flex-start}
.fr-who strong{margin-inline-end:.25em}
.fr-ava{flex:none;display:grid;place-items:center;width:40px;height:40px;background:var(--fg);color:var(--bg);font:800 15px var(--mono)}
.fr-date{flex:none;display:grid;place-items:center;min-width:56px;height:40px;border:1px solid var(--ink-line);font:800 14px var(--mono);font-variant-numeric:tabular-nums;box-shadow:inset 0 -3px 0 var(--red)}
.c-card a.fr-item{display:flex;gap:12px;align-items:center;margin-top:12px;padding:10px;border:1px solid var(--line);background:var(--bg-alt);transition:border-color var(--t) var(--ease),transform var(--t) var(--ease)}
.c-card a.fr-item:hover{border-color:var(--ink-line);transform:translateX(3px)}
.fr-item strong,.fr-item .c-mute{display:block}
.fr-side .c-card{scroll-margin-top:calc(var(--head-h) + 12px)}
.fr-hot{color:var(--red-text);border-color:var(--red)}
/* 手機沒有好友時藏起空的生日卡，讓「邀請朋友」成為第一眼看到的東西 */
@media (max-width:1099px){.fr-no-friends{display:none}}
@media (min-width:1100px){
  .fr{display:grid;grid-template-columns:minmax(0,1fr) 340px;grid-template-areas:"feed top" "feed side";grid-template-rows:auto 1fr;gap:0 24px;align-items:start}
  .fr-feed{grid-area:feed}.fr-top{grid-area:top}.fr-side{grid-area:side}
}
/* 動態區只有空狀態時，桌面不留一整欄空白：空狀態橫跨整列，下方的卡 2 欄 */
@media (min-width:768px){
  .fr.fr-lonely{display:grid;grid-template:none;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px;align-items:stretch}
  .fr-lonely .fr-feed{grid-area:auto;grid-column:1/-1;order:-1;margin:0}
  .fr-lonely :is(.fr-top,.fr-side){display:contents}
  .fr-lonely .c-card{margin:0}
}
</style>
