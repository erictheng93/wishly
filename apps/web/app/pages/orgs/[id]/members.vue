<script setup lang="ts">
// 成員與邀請（owner 管理；其他角色只能看成員）
definePageMeta({ middleware: 'auth' })
const { t } = useI18n()
useHead({ title: () => t('org.members.title') })
const { api } = useApi()
const { user } = useAuth()
const id = useRoute().params.id as string
const { orgs, current, adopt } = useOrgMode()
const isOwner = computed(() => orgs.value.find(o => o.id === id)?.role === 'owner')
const ROLES = ['owner', 'coordinator', 'volunteer']
const members = ref<any[]>([]), invites = ref<any[]>([])
const err = ref(''), busy = ref(false), copied = ref(false)
const form = reactive({ email: '', role: 'volunteer', handle: '' })
const link = ref('')
const off = ref<any>(null)
if (current.value !== id) adopt(id)

async function load() {
  members.value = await api(`/orgs/${id}/members`)
  if (isOwner.value) invites.value = (await api<any[]>(`/orgs/${id}/invites`)).filter(i => !i.accepted_at && !i.cancelled_at)
}
async function run(fn: () => Promise<void>) {
  busy.value = true; err.value = ''
  try { await fn() } catch (e) { err.value = errMsg(e) } finally { busy.value = false }
}
onMounted(() => run(load))
const invite = () => run(async () => {
  const r = await api(`/orgs/${id}/invites`, { method: 'POST', body: { email: form.email, role: form.role, ...(form.handle ? { handle: form.handle } : {}) } })
  link.value = r.url; copied.value = false; form.email = ''; form.handle = ''
  await load()
})
const cancel = (i: any) => run(async () => { await api(`/orgs/${id}/invites/${i.id}`, { method: 'DELETE' }); await load() })
const patch = (m: any, body: any) => run(async () => { await api(`/orgs/${id}/members/${m.user_id}`, { method: 'PATCH', body }); await load() })
const disable = () => run(async () => { await patch(off.value, { disabled: true }); off.value = null })
async function copy() { try { await navigator.clipboard.writeText(link.value); copied.value = true } catch {} }
</script>

<template>
  <main class="c-page m-page">
    <CreatorHeader :title="$t('org.members.title')" :back="`/orgs/${id}`" />
    <p v-if="err" class="c-err c-errbox" role="alert">{{ err }}</p>

    <section class="c-card f-sec">
      <h2 class="c-h2">{{ $t('org.members.list') }}</h2>
      <ul class="m-list">
        <li v-for="m in members" :key="m.user_id" :data-testid="`member-${m.user_id}`">
          <strong>{{ m.display_name }}</strong><span v-if="m.handle" class="c-mute">@{{ m.handle }}</span>
          <span v-if="m.disabled_at" class="c-mute">{{ $t('org.members.disabled') }}</span>
          <select v-if="isOwner && !m.disabled_at" :value="m.role" :aria-label="$t('org.members.role')" :disabled="busy" @change="patch(m, { role: ($event.target as HTMLSelectElement).value })">
            <option v-for="r in ROLES" :key="r" :value="r">{{ $t(`org.members.r.${r}`) }}</option>
          </select>
          <span v-else>{{ $t(`org.members.r.${m.role}`) }}</span>
          <button v-if="isOwner && !m.disabled_at && m.user_id !== user?.id" type="button" class="c-btn danger" @click="off = m">{{ $t('org.members.disable') }}</button>
          <button v-if="isOwner && m.disabled_at" type="button" class="c-btn" :disabled="busy" @click="patch(m, { disabled: false })">{{ $t('org.members.enable') }}</button>
        </li>
      </ul>
    </section>

    <template v-if="isOwner">
      <section class="c-card f-sec">
        <h2 class="c-h2">{{ $t('org.members.inviteTitle') }}</h2>
        <form class="m-form" @submit.prevent="invite">
          <label class="c-field"><span>Email</span><input v-model="form.email" type="email" required autocomplete="off"></label>
          <label class="c-field"><span>{{ $t('org.members.role') }}</span>
            <select v-model="form.role"><option v-for="r in ROLES" :key="r" :value="r">{{ $t(`org.members.r.${r}`) }}</option></select>
          </label>
          <label class="c-field"><span>{{ $t('org.members.handle') }}</span><input v-model="form.handle" maxlength="30" autocomplete="off"></label>
          <button class="c-btn primary" :disabled="busy || !form.email">{{ $t('org.members.send') }}</button>
        </form>
        <div v-if="link" role="status" class="m-link">
          <p>{{ $t('org.members.linkHint') }}</p>
          <input :value="link" readonly :aria-label="$t('org.members.link')" @focus="($event.target as HTMLInputElement).select()">
          <button type="button" class="c-btn" @click="copy">{{ copied ? $t('org.members.copied') : $t('org.members.copy') }}</button>
          <CreatorQr :text="link" />
        </div>
      </section>

      <section class="c-card f-sec">
        <h2 class="c-h2">{{ $t('org.members.pending') }}</h2>
        <p v-if="!invites.length" class="c-mute">{{ $t('org.members.noPending') }}</p>
        <ul class="m-list">
          <li v-for="i in invites" :key="i.id">
            <strong>{{ i.email }}</strong><span>{{ $t(`org.members.r.${i.role}`) }}</span>
            <button type="button" class="c-btn" :disabled="busy" @click="cancel(i)">{{ $t('org.members.cancel') }}</button>
          </li>
        </ul>
      </section>
    </template>

    <CreatorConfirm :open="!!off" :title="$t('org.members.disableTitle')" :text="$t('org.members.disableText', { name: off?.display_name })" danger :busy="busy" @close="off = null" @ok="disable" />
  </main>
</template>

<style scoped>
.f-sec{padding:0;border-top:2px solid var(--ink-line);margin-bottom:24px}
.f-sec>*{margin-left:18px;margin-right:18px}
.f-sec>.c-h2{margin:0 0 18px;padding:14px 18px;background:var(--bg-alt);border-bottom:1px solid var(--line)}
.f-sec>.c-h2~*:last-child{margin-bottom:18px}
.m-list{list-style:none;padding:0;margin:0 18px!important;display:block}
.m-list select{appearance:none;padding-right:34px!important;background-image:linear-gradient(45deg,transparent 50%,var(--fg) 50%),linear-gradient(135deg,var(--fg) 50%,transparent 50%);background-position:calc(100% - 17px) 50%,calc(100% - 12px) 50%;background-size:5px 5px;background-repeat:no-repeat;min-height:44px;box-sizing:border-box;font:inherit;font-size:14px;padding:0 10px;border:1px solid var(--ink-line);background-color:var(--card);color:var(--fg)}
.m-list li{display:flex;flex-wrap:wrap;gap:6px 12px;align-items:center;padding:12px 0;border-bottom:1px solid var(--line);min-height:56px}
.m-list li:last-child{border-bottom:0}
.m-list li>strong{font-size:16px}
.m-list li>.c-mute{font:500 12px var(--mono)}
.m-list li>select{min-width:130px;margin-left:auto}
.m-list li>select~.c-btn,.m-list li>span:not(.c-mute)~.c-btn,.m-list li>.c-btn{margin-left:auto}
.m-list li>select+.c-btn{margin-left:0}
.m-list li>span:not(.c-mute){font:600 11px/22px var(--mono);letter-spacing:.1em;text-transform:uppercase;padding:0 8px;border:1px solid var(--ink-line);margin-left:0}
.m-list li>strong{overflow-wrap:anywhere;min-width:0}
.m-list li>.c-btn{min-height:44px}
.m-list li>strong+span:not(.c-mute)+.c-btn{margin-left:auto}
.m-form{display:grid;grid-template-columns:1fr;gap:0 20px}
.m-form>.c-btn{justify-self:start}
@media (min-width:768px){.m-form{grid-template-columns:2fr 1fr 1fr}.m-form>.c-btn{grid-column:1/-1}}
.m-link{margin-top:20px;padding:16px;border:1px dashed var(--ink-line);background:var(--hatch),var(--bg-alt)}
.m-link input{width:100%;box-sizing:border-box;min-height:48px;margin:8px 0 12px;padding:0 12px;border:1px solid var(--line);background:var(--card);color:var(--fg);font:500 13px var(--mono)}
</style>
