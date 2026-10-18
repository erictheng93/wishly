<script setup lang="ts">
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('lists.new.pageTitle') })
const { api } = useApi()
const f = reactive({ type: 'registry', title: '', description: '', event_date: '', closes_at: '', surprise_mode: false, show_claimer_names: false, visibility: 'friends' })
const allowed = ref<string[]>([])
const cover = ref<string | null>(null)
const upBusy = ref(false), busy = ref(false), err = ref(''), dateErr = ref('')
const today = new Date(Date.now() + 8 * 3600e3).toISOString().slice(0, 10)
const nowLocal = toLocalInput(new Date().toISOString())
async function submit() {
  err.value = dateErr.value = ''
  if (f.surprise_mode && !f.event_date) { dateErr.value = t('lists.new.errSurprise'); return }
  if (f.surprise_mode && f.event_date <= today) { dateErr.value = t('lists.new.errEventDate'); return }
  if (f.closes_at && f.closes_at <= toLocalInput(new Date().toISOString())) { dateErr.value = t('lists.new.errClose'); return }
  busy.value = true
  try {
    const body: any = { type: f.type, title: f.title.trim(), description: f.description || null, event_date: f.event_date || null, closes_at: fromLocalInput(f.closes_at), visibility: f.visibility, surprise_mode: f.surprise_mode, show_claimer_names: f.show_claimer_names }
    if (cover.value) body.cover_image_key = cover.value
    const w = await api('/wishlists', { method: 'POST', body })
    // 名單寫入失敗不擋導頁：清單已建立，編輯頁會顯示目前名單，可再調整
    if (f.visibility === 'selected') await api(`/wishlists/${w.id}/allowed-users`, { method: 'PUT', body: { user_ids: allowed.value } }).catch(() => {})
    await navigateTo(`/lists/${w.id}/edit`)
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('lists.new.pageTitle')" back="/dashboard" />
    <form @submit.prevent="submit">
      <div class="c-field"><span>{{ $t('lists.new.type') }}</span>
        <div class="c-seg">
          <label><input v-model="f.type" type="radio" value="personal"><span>{{ $t('lists.common.personal') }}</span></label>
          <label><input v-model="f.type" type="radio" value="registry"><span>{{ $t('lists.common.registry') }}</span></label>
        </div>
      </div>
      <label class="c-field"><span>{{ $t('lists.new.title', { n: f.title.length }) }}</span><input v-model="f.title" maxlength="100" required :placeholder="$t('lists.new.titlePh')"></label>
      <label class="c-field"><span>{{ $t('lists.new.desc') }}</span><textarea v-model="f.description" rows="2" maxlength="300" /></label>
      <CreatorImageUploader v-model="cover" purpose="cover" @busy="upBusy = $event" />
      <label class="c-field"><span>{{ $t('lists.new.event') }}</span><input v-model="f.event_date" type="date" :min="today"></label>
      <label class="c-field"><span>{{ $t('lists.new.closes') }}</span><input v-model="f.closes_at" type="datetime-local" :min="nowLocal"></label>
      <p v-if="dateErr" class="c-err" role="alert">{{ dateErr }}</p>
      <label class="c-switch"><input v-model="f.surprise_mode" type="checkbox"><span>{{ $t('lists.new.surprise') }}<br><small class="c-mute">{{ $t('lists.new.surpriseHint') }}</small></span></label>
      <label class="c-switch"><input v-model="f.show_claimer_names" type="checkbox"><span>{{ $t('lists.new.showNames') }}<br><small class="c-mute">{{ $t('lists.new.showNamesHint') }}</small></span></label>
      <CreatorVisibilityPicker v-model:visibility="f.visibility" v-model:users="allowed" />
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block" :disabled="busy || upBusy || !f.title.trim()">{{ busy ? $t('lists.new.creating') : $t('lists.new.submit') }}</button>
    </form>
  </main>
</template>
