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
  <main class="c-page">
    <CreatorHeader :title="$t('org.members.title')" :back="`/orgs/${id}`" />
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>

    <section class="c-card">
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
      <section class="c-card">
        <h2 class="c-h2">{{ $t('org.members.inviteTitle') }}</h2>
        <form @submit.prevent="invite">
          <label class="c-field"><span>Email</span><input v-model="form.email" type="email" required autocomplete="off"></label>
          <label class="c-field"><span>{{ $t('org.members.role') }}</span>
            <select v-model="form.role"><option v-for="r in ROLES" :key="r" :value="r">{{ $t(`org.members.r.${r}`) }}</option></select>
          </label>
          <label class="c-field"><span>{{ $t('org.members.handle') }}</span><input v-model="form.handle" maxlength="30" autocomplete="off"></label>
          <button class="c-btn primary" :disabled="busy || !form.email">{{ $t('org.members.send') }}</button>
        </form>
        <div v-if="link" role="status">
          <p>{{ $t('org.members.linkHint') }}</p>
          <input :value="link" readonly :aria-label="$t('org.members.link')" @focus="($event.target as HTMLInputElement).select()">
          <button type="button" class="c-btn" @click="copy">{{ copied ? $t('org.members.copied') : $t('org.members.copy') }}</button>
          <CreatorQr :text="link" />
        </div>
      </section>

      <section class="c-card">
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
.m-list{list-style:none;padding:0;margin:12px 0 0;display:grid;gap:12px}
.m-list li{display:flex;flex-wrap:wrap;gap:4px 12px;align-items:center}
</style>
