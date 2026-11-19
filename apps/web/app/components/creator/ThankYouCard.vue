<script setup lang="ts">
const props = defineProps<{ claimId: string; thanked: boolean }>()
const { api } = useApi()
const open = ref(false), busy = ref(false), sent = ref(false)
const message = ref(''), error = ref('')
const length = computed(() => [...message.value.trim()].length)

async function send() {
  if (busy.value || sent.value || props.thanked || !length.value || length.value > 500) return
  busy.value = true
  error.value = ''
  try {
    await api(`/claims/${props.claimId}/thank-you`, { method: 'POST', body: { message: message.value.trim() } })
    sent.value = true
    open.value = false
  } catch (e) { error.value = errMsg(e) } finally { busy.value = false }
}
</script>

<template>
  <span v-if="thanked || sent" class="c-mute" role="status">{{ $t('lists.progress.thankYouSent') }}</span>
  <template v-else>
    <button type="button" class="c-btn" @click="open = true">{{ $t('lists.progress.thankYouWrite') }}</button>
    <CreatorSheet :open="open" :title="$t('lists.progress.thankYouWrite')" @close="open = false">
      <form @submit.prevent="send">
        <label class="c-field"><span>{{ $t('lists.progress.thankYouMessage', { n: length }) }}</span>
          <textarea v-model="message" rows="4" required autofocus :disabled="busy" /></label>
        <p v-if="error" class="c-err" role="alert">{{ error }}</p>
        <div class="c-row">
          <button type="button" class="c-btn" @click="open = false">{{ $t('creator.confirm.cancel') }}</button>
          <button class="c-btn primary" :disabled="busy || !length || length > 500">{{ $t('lists.progress.thankYouSend') }}</button>
        </div>
      </form>
    </CreatorSheet>
  </template>
</template>
