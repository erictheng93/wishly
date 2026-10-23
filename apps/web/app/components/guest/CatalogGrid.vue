<script setup lang="ts">
import { fetchWishes, wishLeft, type WishScope } from '~/utils/inspireWishes'
import type { CatalogProduct } from '~/utils/catalog'
// 精選商品卡片格 + 「加到我的願望清單」「誰想要這個」兩個對話框；無購物車，只能加到自己的清單或看外部商品
defineProps<{ products: CatalogProduct[] }>()
const { t } = useI18n()

// 加到清單：僅 client 端（sheet 由點擊開啟）；未登入時 useApi 會 401 導去登入並帶回
const { api } = useApi()
const route = useRoute()
const pick = ref<CatalogProduct | null>(null)
const lists = ref<any[]>([]), loading = ref(false), err = ref('')
async function open(p: CatalogProduct) {
  pick.value = p; loading.value = true; err.value = ''
  try {
    const all: any[] = []
    let cursor: string | undefined
    do {
      const r = await api('/wishlists', { query: { limit: 100, status: 'draft,active', cursor } })
      all.push(...r.data); cursor = r.next_cursor ?? undefined
    } while (cursor)
    lists.value = all
  } catch (e) { if (pick.value) err.value = t('guest.inspire.loadErr') } finally { loading.value = false }
}
function choose(id: string) {
  const p = pick.value!
  const query: Record<string, string> = { add: '1', title: p.title, product_url: p.product_url, ref: p.ref }
  if (p.price_amount != null) query.unit_price_amount = String(p.price_amount)
  if (p.brand) query.brand = p.brand.name
  if (p.spec) query.spec = p.spec
  pick.value = null
  return navigateTo({ path: `/lists/${id}/edit`, query })
}

// 誰想要這個：client 端帶憑證抓（不用 useApi：401 會被導去登入）。未登入只看得到公開
const { public: { apiBase } } = useRuntimeConfig()
const { user, fetchMe } = useAuth()
const who = ref<CatalogProduct | null>(null), scope = ref<WishScope>('')
const wishes = ref<any[]>([]), wLoading = ref(false), wErr = ref(false)
const SCOPES: WishScope[] = ['', 'public', 'friends', 'selected']
let seq = 0 // 快速切換商品 / 範圍時只採用最後一次請求
async function loadWishes() {
  const p = who.value; if (!p) return
  const n = ++seq
  wLoading.value = true; wErr.value = false
  try { const r = await fetchWishes(apiBase, p.ref, scope.value); if (n === seq) wishes.value = r }
  catch { if (n === seq) wErr.value = true } finally { if (n === seq) wLoading.value = false }
}
async function openWho(p: CatalogProduct) {
  who.value = p; scope.value = ''; wishes.value = []
  await fetchMe() // noRedirect：只為判斷是否登入
  loadWishes()
}
function setScope(s: WishScope) { scope.value = s; loadWishes() }
</script>

<template>
  <div class="g-grid">
    <article v-for="p in products" :key="p.ref" class="g-card">
      <img v-if="p.image_url" class="x-img" :src="p.image_url" alt="" loading="lazy">
      <div v-else class="x-img x-ph" aria-hidden="true">{{ (p.brand?.name || p.title).slice(0, 1) }}</div>
      <p v-if="p.brand" class="g-mute"><NuxtLink :to="`/brands/${p.brand.slug}`">{{ p.brand.name }}</NuxtLink></p>
      <strong class="x-title">{{ p.title }}</strong>
      <p v-if="p.spec" class="g-mute">{{ p.spec }}</p>
      <p v-if="p.price_amount != null" class="x-price">NT${{ p.price_amount.toLocaleString('en-US') }}</p>
      <p v-if="p.note" class="g-mute">{{ p.note }}</p>
      <div class="x-act">
        <button class="g-btn block" @click="open(p)">{{ $t('guest.inspire.add') }}</button>
        <button class="g-btn ghost block" @click="openWho(p)">{{ $t('guest.inspire.who') }}</button>
        <a class="g-btn ghost block" :href="p.product_url" target="_blank" rel="noopener nofollow">{{ $t('guest.inspire.view') }}</a>
      </div>
    </article>
  </div>
  <GuestDialog v-if="who" labelledby="inspire-who" @close="who = null">
    <div class="g-sheet">
      <h1 id="inspire-who">{{ $t('guest.inspire.whoTitle') }}</h1>
      <p class="g-mute">{{ who.title }}</p>
      <div class="g-chips" role="group" :aria-label="$t('guest.inspire.whoTitle')">
        <button v-for="s in SCOPES" :key="s" class="g-btn ghost sm" :class="{ on: scope === s }" :aria-pressed="scope === s" @click="setScope(s)">{{ $t(`guest.inspire.scope.${s || 'all'}`) }}</button>
      </div>
      <p v-if="!user" class="g-mute">{{ $t('guest.inspire.loginHint') }} <NuxtLink :to="`/login?redirect=${encodeURIComponent(route.fullPath)}`">{{ $t('guest.slug.login') }}</NuxtLink></p>
      <p v-if="wLoading" class="g-mute" role="status">{{ $t('guest.inspire.loading') }}</p>
      <template v-else-if="wErr">
        <p class="g-field-err" role="alert">{{ $t('guest.inspire.whoErr') }}</p>
        <button class="g-btn ghost block" @click="loadWishes">{{ $t('guest.inspire.retry') }}</button>
      </template>
      <p v-else-if="!wishes.length" class="g-mute">{{ $t('guest.inspire.whoEmpty') }}</p>
      <ul v-else class="x-wishes">
        <li v-for="w in wishes" :key="w.item.id" class="x-wish">
          <div>
            <NuxtLink v-if="w.owner.handle" :to="`/u/${w.owner.handle}`"><b>{{ w.owner.display_name }}</b></NuxtLink><b v-else>{{ w.owner.display_name }}</b>
            <span class="g-badge">{{ $t(`creator.vis.${w.wishlist.visibility}.label`) }}</span>
            <p class="g-mute">{{ w.wishlist.title }}</p>
            <p class="g-mute">{{ w.item.open ? $t('guest.inspire.left', { n: wishLeft(w.item) }) : $t('guest.inspire.done') }}</p>
          </div>
          <NuxtLink v-if="w.item.open" :to="`/s/${w.wishlist.slug}?item=${w.item.id}`" class="g-btn sm">{{ $t('guest.inspire.gift') }}</NuxtLink>
          <button v-else class="g-btn sm" disabled>{{ $t('guest.inspire.done') }}</button>
        </li>
      </ul>
      <button class="g-btn ghost block x-cancel" @click="who = null">{{ $t('guest.cancel') }}</button>
    </div>
  </GuestDialog>
  <GuestDialog v-if="pick" labelledby="inspire-pick" @close="pick = null">
    <div class="g-sheet">
      <h1 id="inspire-pick">{{ $t('guest.inspire.pickTitle') }}</h1>
      <p class="g-mute">{{ pick.title }}</p>
      <p v-if="loading" class="g-mute" role="status">{{ $t('guest.inspire.loading') }}</p>
      <p v-else-if="err" class="g-field-err" role="alert">{{ err }}</p>
      <template v-else-if="!lists.length">
        <p class="g-mute">{{ $t('guest.inspire.noLists') }}</p>
        <NuxtLink to="/lists/new" class="g-btn block">{{ $t('guest.inspire.create') }}</NuxtLink>
      </template>
      <div v-else class="x-lists">
        <button v-for="w in lists" :key="w.id" class="g-btn ghost block" @click="choose(w.id)">{{ w.title }}</button>
      </div>
      <button class="g-btn ghost block x-cancel" @click="pick = null">{{ $t('guest.cancel') }}</button>
    </div>
  </GuestDialog>
</template>

<style scoped>
.x-img{display:block;width:100%;aspect-ratio:4/3;object-fit:cover;margin-bottom:8px}
.x-ph{display:flex;align-items:center;justify-content:center;background:var(--bg-alt);border:1px solid var(--line);color:var(--red-text);font-size:48px;font-weight:700}
.x-title{display:block;font-size:17px;overflow-wrap:anywhere}
.x-price{font-weight:700;color:var(--red-text);margin:4px 0}
.x-act{display:grid;gap:8px;margin-top:12px}
.x-lists{display:grid;gap:8px;margin-top:12px}
.x-lists .g-btn{justify-content:flex-start;text-align:left}
.x-wishes{list-style:none;padding:0;margin:12px 0 0;display:grid;gap:8px}
.x-wish{display:flex;justify-content:space-between;align-items:center;gap:12px;border:1px solid var(--line);padding:8px 12px}
.x-cancel{margin-top:12px}
</style>
