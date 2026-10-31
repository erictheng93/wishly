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
  <main class="c-page oi">
    <CreatorHeader :title="$t('org.invite.title')" />
    <p v-if="loading" class="c-mute" role="status">{{ $t('org.apply.loading') }}</p>
    <section v-else-if="err && !info" class="c-card oi-card oi-bad"><p class="c-err oi-text" role="alert">{{ err }}</p></section>
    <p v-else-if="err" class="c-err" role="alert">{{ err }}</p>
    <section v-if="info" class="c-card oi-card">
      <p class="oi-text">{{ $t('org.invite.text', { org: info.org_display_name, role: $t(`org.members.r.${info.role}`) }) }}</p>
      <button type="button" class="c-btn primary block" :disabled="busy" @click="accept">{{ $t('org.invite.accept') }}</button>
    </section>
  </main>
</template>

<style scoped>
.oi-card{max-width:520px;margin:24px auto 0;padding:36px clamp(20px,5vw,40px) 32px;border:1px solid var(--ink-line);border-top:3px solid var(--red);box-shadow:8px 8px 0 var(--bg-alt),8px 8px 0 1px var(--line);position:relative;animation:rise .5s var(--ease) both}
.oi-card::before{content:"INVITATION / MEMBER" / "";display:block;margin-bottom:18px;font:600 11px var(--mono);letter-spacing:.18em;color:var(--mute)}
.oi-text{font-size:clamp(20px,3vw,24px);font-weight:800;line-height:1.4;letter-spacing:-.01em;margin:0 0 28px;text-wrap:balance}
.oi-bad{border-top-color:var(--ink-line)}
.oi-bad::before{content:"INVITATION / INVALID" / ""!important}
.oi-bad .oi-text{font-size:clamp(17px,2.6vw,20px);font-weight:700;margin:0}
.oi-card .c-btn{min-height:52px}
</style>
