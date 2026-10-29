<script setup lang="ts">
import '~/assets/console.css' // 後台台帳／KPI 樣式（.c-admin、.k-）
// 品牌館與精選商品後台：品牌 / 商品的列表與新增、編輯。slug / ref 建立後不可改；圖片經 CreatorImageUploader（purpose catalog）
import { OCCASIONS, RECIPIENTS } from '~/utils/catalog'
definePageMeta({ middleware: 'staff' })
const { t } = useI18n()
useHead({ title: () => t('admin.catalog.title') })
const { api } = useApi()
const tab = ref<'brands' | 'products'>('brands')
const err = ref(''), msg = ref('')

const brands = ref<any[]>([]), bLoading = ref(false)
async function loadBrands() {
  bLoading.value = true; err.value = ''
  try { brands.value = (await api('/admin/brands')).brands } catch (e) { err.value = errMsg(e) } finally { bLoading.value = false }
}
const brandName = (id: string | null) => brands.value.find(b => b.id === id)?.name ?? t('admin.catalog.curated')

const brandFilter = ref(''), products = ref<any[]>([]), pNext = ref<number | null>(null), pLoading = ref(false)
async function loadProducts(more = false) {
  pLoading.value = true; err.value = ''
  try {
    const r = await api('/admin/catalog/products', { query: { brand_id: brandFilter.value || undefined, page: more ? pNext.value : undefined } })
    products.value = more ? [...products.value, ...r.products] : r.products; pNext.value = r.next_page ?? null
  } catch (e) { err.value = errMsg(e) } finally { pLoading.value = false }
}
watch(brandFilter, () => loadProducts())
onMounted(() => { loadBrands(); loadProducts() })

// 表單（同一個 sheet 依 tab 切換欄位）：editing=null 為新增
const open = ref(false), editing = ref<any | null>(null), f = reactive<any>({})
const imageKey = ref<string | null>(null), upBusy = ref(false), busy = ref(false), ferr = ref('')
function show(row: any | null) {
  editing.value = row; ferr.value = ''; imageKey.value = null
  Object.assign(f, tab.value === 'brands'
    ? { slug: '', name: '', description: '', website_url: '', active: true, sort_order: 0, ...row }
    : { ref: '', brand_id: '', title: '', spec: '', note: '', price_amount: '', product_url: '', recipients: [], occasions: [], active: true, sort_order: 0, ...row, ...(row ? { brand_id: row.brand_id ?? '', recipients: [...row.recipients], occasions: [...row.occasions] } : {}) })
  open.value = true
}
const blank = (v: any) => (v == null || String(v).trim() === '' ? null : String(v).trim())
function body() {
  const base = { active: f.active, sort_order: Number(f.sort_order) || 0 }
  if (tab.value === 'brands') {
    const b: any = { ...base, name: f.name.trim(), description: blank(f.description), website_url: blank(f.website_url) }
    if (!editing.value) b.slug = f.slug.trim()
    if (imageKey.value) b.logo_key = imageKey.value
    return b
  }
  const b: any = { ...base, brand_id: f.brand_id || null, title: f.title.trim(), spec: blank(f.spec), note: blank(f.note), product_url: f.product_url.trim(), recipients: f.recipients, occasions: f.occasions, price_amount: blank(f.price_amount) == null ? null : Number(f.price_amount) }
  if (!editing.value) b.ref = f.ref.trim()
  if (imageKey.value) b.image_key = imageKey.value
  return b
}
async function save() {
  busy.value = true; ferr.value = ''
  const path = tab.value === 'brands' ? '/admin/brands' : '/admin/catalog/products'
  try {
    await api(editing.value ? `${path}/${editing.value.id}` : path, { method: editing.value ? 'PATCH' : 'POST', body: body() })
    open.value = false; msg.value = t('admin.catalog.saved')
    await (tab.value === 'brands' ? loadBrands() : loadProducts())
  } catch (e) { ferr.value = errMsg(e) } finally { busy.value = false }
}
const toggle = (list: string[], v: string) => { const i = list.indexOf(v); i < 0 ? list.push(v) : list.splice(i, 1) }
</script>

<template>
  <main class="c-page c-admin">
    <CreatorHeader :title="$t('admin.catalog.title')" back="/admin" />
    <div class="c-row wrap c-mb" role="group" :aria-label="$t('admin.catalog.tabsLabel')">
      <button v-for="k in (['brands', 'products'] as const)" :key="k" class="c-btn c-grow" :class="{ primary: tab === k }" :aria-pressed="tab === k" @click="tab = k">{{ $t(`admin.catalog.tab.${k}`) }}</button>
    </div>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>

    <template v-if="tab === 'brands'">
      <button class="c-btn primary block" @click="show(null)">{{ $t('admin.catalog.newBrand') }}</button>
      <p v-if="bLoading && !brands.length" class="c-mute" role="status">{{ $t('admin.catalog.loading') }}</p>
      <p v-else-if="!brands.length" class="c-center c-mute">{{ $t('admin.catalog.emptyBrands') }}</p>
      <article v-for="b in brands" :key="b.id" class="c-card">
        <div class="c-row wrap">
          <strong class="c-grow">{{ b.name }}　<span class="c-mute">{{ b.slug }}</span></strong>
          <span class="c-badge" :class="{ active: b.active }">{{ $t(b.active ? 'admin.catalog.on' : 'admin.catalog.off') }}</span>
        </div>
        <p v-if="b.description" class="c-mute c-small">{{ b.description }}</p>
        <p v-if="b.logo_status && b.logo_status !== 'ready'" class="c-mute c-small">{{ $t('admin.catalog.logoStatus', { s: b.logo_status }) }}</p>
        <button class="c-btn" @click="show(b)">{{ $t('admin.catalog.edit') }}</button>
      </article>
    </template>

    <template v-else>
      <label class="c-field c-narrow"><span>{{ $t('admin.catalog.brand') }}</span>
        <select v-model="brandFilter"><option value="">{{ $t('admin.catalog.allBrands') }}</option><option v-for="b in brands" :key="b.id" :value="b.id">{{ b.name }}</option></select>
      </label>
      <button class="c-btn primary block" @click="show(null)">{{ $t('admin.catalog.newProduct') }}</button>
      <p v-if="pLoading && !products.length" class="c-mute" role="status">{{ $t('admin.catalog.loading') }}</p>
      <p v-else-if="!products.length" class="c-center c-mute">{{ $t('admin.catalog.emptyProducts') }}</p>
      <article v-for="p in products" :key="p.id" class="c-card">
        <div class="c-row wrap">
          <strong class="c-grow">{{ p.title }}</strong>
          <span class="c-badge" :class="{ active: p.active }">{{ $t(p.active ? 'admin.catalog.on' : 'admin.catalog.off') }}</span>
        </div>
        <p class="c-mute c-small">{{ p.ref }}・{{ brandName(p.brand_id) }}<template v-if="p.price_amount != null">・NT${{ p.price_amount.toLocaleString('en-US') }}</template></p>
        <p v-if="p.image_status && p.image_status !== 'ready'" class="c-mute c-small">{{ $t('admin.catalog.imageStatus', { s: p.image_status }) }}</p>
        <button class="c-btn" @click="show(p)">{{ $t('admin.catalog.edit') }}</button>
      </article>
      <button v-if="pNext" class="c-btn block" :disabled="pLoading" @click="loadProducts(true)">{{ $t('admin.catalog.loadMore') }}</button>
    </template>

    <CreatorSheet :open="open" :title="$t(`admin.catalog.${tab === 'brands' ? 'brandForm' : 'productForm'}`)" @close="open = false">
      <form @submit.prevent="save">
        <template v-if="tab === 'brands'">
          <CreatorImageUploader v-model="imageKey" purpose="catalog" :current-url="editing?.logo_url" @busy="upBusy = $event" />
          <label class="c-field"><span>{{ $t('admin.catalog.slug') }}</span><input v-model="f.slug" maxlength="60" :disabled="!!editing" required></label>
          <label class="c-field"><span>{{ $t('admin.catalog.name') }}</span><input v-model="f.name" maxlength="100" required></label>
          <label class="c-field"><span>{{ $t('admin.catalog.description') }}</span><textarea v-model="f.description" maxlength="500" rows="3" /></label>
          <label class="c-field"><span>{{ $t('admin.catalog.website') }}</span><input v-model="f.website_url" type="url" maxlength="2000"></label>
        </template>
        <template v-else>
          <CreatorImageUploader v-model="imageKey" purpose="catalog" :current-url="editing?.image_url" @busy="upBusy = $event" />
          <label class="c-field"><span>{{ $t('admin.catalog.ref') }}</span><input v-model="f.ref" maxlength="64" pattern="[A-Za-z0-9_\-]{1,64}" :disabled="!!editing" required></label>
          <label class="c-field"><span>{{ $t('admin.catalog.brand') }}</span>
            <select v-model="f.brand_id"><option value="">{{ $t('admin.catalog.curated') }}</option><option v-for="b in brands" :key="b.id" :value="b.id">{{ b.name }}</option></select>
          </label>
          <label class="c-field"><span>{{ $t('admin.catalog.productTitle') }}</span><input v-model="f.title" maxlength="120" required></label>
          <label class="c-field"><span>{{ $t('admin.catalog.spec') }}</span><input v-model="f.spec" maxlength="200"></label>
          <label class="c-field"><span>{{ $t('admin.catalog.note') }}</span><textarea v-model="f.note" maxlength="500" rows="2" /></label>
          <label class="c-field"><span>{{ $t('admin.catalog.price') }}</span><input v-model="f.price_amount" type="number" min="0" step="1" inputmode="numeric" required></label>
          <label class="c-field"><span>{{ $t('admin.catalog.productUrl') }}</span><input v-model="f.product_url" type="url" maxlength="2000" required></label>
          <fieldset class="c-field"><legend>{{ $t('guest.inspire.recipient') }}</legend>
            <label v-for="v in RECIPIENTS" :key="v" class="c-row"><input type="checkbox" :checked="f.recipients.includes(v)" @change="toggle(f.recipients, v)">{{ $t(`guest.inspire.r.${v}`) }}</label>
          </fieldset>
          <fieldset class="c-field"><legend>{{ $t('guest.inspire.occasion') }}</legend>
            <label v-for="v in OCCASIONS" :key="v" class="c-row"><input type="checkbox" :checked="f.occasions.includes(v)" @change="toggle(f.occasions, v)">{{ $t(`guest.inspire.o.${v}`) }}</label>
          </fieldset>
        </template>
        <label class="c-field"><span>{{ $t('admin.catalog.sort') }}</span><input v-model="f.sort_order" type="number" step="1" inputmode="numeric"></label>
        <label class="c-row"><input v-model="f.active" type="checkbox">{{ $t('admin.catalog.active') }}</label>
        <p v-if="ferr" class="c-err" role="alert">{{ ferr }}</p>
        <div class="c-row c-mt">
          <button type="button" class="c-btn c-grow" @click="open = false">{{ $t('admin.catalog.cancel') }}</button>
          <button class="c-btn primary c-grow" :disabled="busy || upBusy">{{ busy ? $t('admin.catalog.saving') : $t('admin.catalog.save') }}</button>
        </div>
      </form>
    </CreatorSheet>
  </main>
</template>
