<script setup lang="ts">
import '~/assets/creator-pages.css'
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('lists.new.pageTitle') })
const { api } = useApi()
const route = useRoute()
// 建立只要名稱與封面；可見性（預設 friends）、日期、驚喜模式等由後端給預設值，下一步在清單設定調整
const title = ref('')
const cover = ref<string | null>(null), coverPreset = ref<string | null>(DEFAULT_COVER)
const upBusy = ref(false), busy = ref(false), err = ref('')
async function submit() {
  err.value = ''
  busy.value = true
  try {
    const body: any = { type: 'personal', title: title.value.trim() }
    // 封面互斥：上傳照片就清 preset，選預設圖就清 key
    if (cover.value) Object.assign(body, { cover_image_key: cover.value, cover_preset: null })
    else Object.assign(body, { cover_preset: coverPreset.value, cover_image_key: null })
    const w = await api('/wishlists', { method: 'POST', body })
    useOnboarding().refresh() // 新手導覽：建立清單完成
    await navigateTo({ path: `/lists/${w.id}/edit`, query: route.query }) // 保留靈感頁帶來的 ?add=1 預填
  } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <main class="c-page">
    <CreatorHeader :title="$t('lists.new.pageTitle')" back="/dashboard" />
    <form class="ln" @submit.prevent="submit">
      <label class="c-field"><span>{{ $t('lists.new.title', { n: title.length }) }}</span><input v-model="title" maxlength="100" required :placeholder="$t('lists.new.titlePh')"></label>
      <CreatorCoverPicker v-model:preset="coverPreset" v-model:image-key="cover" @busy="upBusy = $event" />
      <p class="c-mute c-small">{{ $t('lists.new.note') }}</p>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <button class="c-btn primary block" :disabled="busy || upBusy || !title.trim()">{{ busy ? $t('lists.new.creating') : $t('lists.new.submit') }}</button>
    </form>
  </main>
</template>
<style scoped>
.ln{max-width:560px}
</style>
