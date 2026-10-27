<script setup lang="ts">
// 接受組織邀請：需登入（auth middleware 會帶 redirect 回來）
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.invite.title') })
const { api } = useApi()
const { fetchMe } = useAuth()
const { switchTo } = useOrgMode()
const token = useRoute().params.token as string
const info = ref<any>(null)
const err = ref(''), busy = ref(false), loading = ref(true)

// email 不符要帶入受邀 email，一般 errMsg 不會插值
const fail = (e: any) => { err.value = e?.code === 'INVITE_EMAIL_MISMATCH' ? t('org.invite.mismatch', { email: e.body?.email ?? '' }) : errMsg(e) }
onMounted(async () => {
  try { info.value = await api(`/org-invites/${token}`) } catch (e) { fail(e) } finally { loading.value = false }
})
async function accept() {
  busy.value = true; err.value = ''
  try {
    const r = await api(`/org-invites/${token}/accept`, { method: 'POST' })
    await fetchMe(true)
    switchTo(r.org_id)
  } catch (e) { fail(e) } finally { busy.value = false }
}
</script>

<template>
  <main class="c-page">
    <CreatorHeader :title="$t('org.invite.title')" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-if="loading" class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
    <section v-else-if="info" class="c-card">
      <p>{{ $t('org.invite.text', { org: info.org_display_name, role: $t(`org.members.r.${info.role}`) }) }}</p>
      <button type="button" class="c-btn primary" :disabled="busy" @click="accept">{{ $t('org.invite.accept') }}</button>
    </section>
  </main>
</template>
