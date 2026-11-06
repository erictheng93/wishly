<script setup lang="ts">
// 個人頁外觀：封面、自我介紹、外部連結（上下移動排序，整組覆寫）
const MAX_LINKS = 10
const { api } = useApi()
const { t } = useI18n()
const { user } = useAuth()
const bio = ref(user.value?.bio ?? '')
const preset = ref<string | null>(user.value?.cover_preset ?? null)
const imageKey = ref<string | null>(user.value?.cover_image_key ?? null)
const links = ref<{ title: string, url: string, hidden: boolean }[]>([])
const linkErrs = ref<Record<number, string>>({})
const uploading = ref(false), busy = ref(false), err = ref(''), msg = ref(''), copied = ref(false)
const pageUrl = computed(() => user.value?.handle ? `${siteOrigin()}/l/${user.value.handle}` : '')
onMounted(async () => {
  try { links.value = (await api('/me/links')).links } catch (e: any) { err.value = apiErrMsg(e) }
})
function move(i: number, d: number) {
  const j = i + d
  if (j < 0 || j >= links.value.length) return
  ;[links.value[i], links.value[j]] = [links.value[j]!, links.value[i]!]
}
async function save() {
  if (busy.value || uploading.value) return
  busy.value = true; err.value = msg.value = ''; linkErrs.value = {}
  try {
    user.value = await api('/me', { method: 'PATCH', body: { bio: bio.value.trim() || null, cover_preset: preset.value, cover_image_key: imageKey.value } })
    await api('/me/links', { method: 'PUT', body: { links: links.value.map(l => ({ title: l.title.trim(), url: l.url.trim(), hidden: l.hidden })) } })
    msg.value = t('settings.saved')
  } catch (e: any) {
    for (const x of e.errors ?? []) { const m = /^\/links\/(\d+)\//.exec(x.pointer ?? ''); if (m) linkErrs.value[+m[1]!] = x.detail }
    err.value = Object.keys(linkErrs.value).length ? '' : apiErrMsg(e)
  } finally { busy.value = false }
}
async function copy() { try { await copyText(pageUrl.value); copied.value = true } catch { err.value = t('settings.social.copyFail') } }
</script>
<template>
  <section id="appearance" class="c-card">
    <h2 class="c-h2">{{ $t('settings.pc.ap.title') }}</h2>
    <form @submit.prevent="save">
      <CreatorCoverPicker v-model:preset="preset" v-model:image-key="imageKey" :current-url="user?.cover_image_url" @busy="uploading = $event" />
      <label class="c-field"><span>{{ $t('settings.pc.ap.bio') }}</span><textarea v-model="bio" maxlength="160" rows="3" /></label>
      <fieldset class="c-field pf-set">
        <legend class="c-lbl">{{ $t('settings.pc.ap.links') }}</legend>
        <div v-for="(l, i) in links" :key="i" class="pf-link">
          <div class="c-row">
            <UiIcon :name="linkIcon(l.url) as any" />
            <input v-model="l.title" maxlength="40" required :placeholder="$t('settings.pc.ap.linkTitle')" :aria-label="$t('settings.pc.ap.linkTitle')">
          </div>
          <input v-model="l.url" type="url" maxlength="500" required :placeholder="$t('settings.pc.ap.linkUrl')" :aria-label="$t('settings.pc.ap.linkUrl')">
          <small v-if="linkErrs[i]" class="c-err">{{ linkErrs[i] }}</small>
          <div class="c-row wrap">
            <label class="c-row"><input v-model="l.hidden" type="checkbox">{{ $t('settings.pc.ap.linkHidden') }}</label>
            <span class="c-grow" />
            <button type="button" class="c-btn small" :disabled="i === 0" :aria-label="$t('settings.pc.up')" @click="move(i, -1)"><UiIcon name="arrow-up" :size="16" /></button>
            <button type="button" class="c-btn small" :disabled="i === links.length - 1" :aria-label="$t('settings.pc.down')" @click="move(i, 1)"><UiIcon name="arrow-down" :size="16" /></button>
            <button type="button" class="c-btn small danger" @click="links.splice(i, 1)">{{ $t('settings.pc.remove') }}</button>
          </div>
        </div>
        <button type="button" class="c-btn small" :disabled="links.length >= MAX_LINKS" @click="links.push({ title: '', url: 'https://', hidden: false })"><UiIcon name="plus" :size="16" />{{ $t('settings.pc.add') }}</button>
        <small v-if="links.length >= MAX_LINKS" class="c-mute"> {{ $t('settings.pc.ap.linksMax') }}</small>
      </fieldset>
      <p class="c-mute st-kv">
        <template v-if="pageUrl"><span>{{ $t('settings.pc.ap.pageUrl') }}</span><code>{{ pageUrl }}</code> <button type="button" class="c-btn" @click="copy">{{ copied ? $t('settings.pc.ap.copied') : $t('settings.pc.ap.copyPage') }}</button></template>
        <template v-else>{{ $t('settings.pc.ap.needHandle') }}</template>
      </p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
      <button class="c-btn" :disabled="busy || uploading">{{ $t('settings.save') }}</button>
    </form>
  </section>
</template>
<style scoped>
.pf-set{border:0;padding:0;min-width:0}
.pf-link{display:grid;gap:8px;padding:12px 0;border-top:1px solid var(--line)}
.pf-link input[type=text],.pf-link input[type=url]{min-width:0}
</style>
