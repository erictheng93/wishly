<script setup lang="ts">
import { INTEREST_GROUPS, MAX_INTERESTS } from '~/utils/interests'
// 送禮偏好（喜歡 / 不想收到）與興趣標籤，各自可設定誰看得到
const MAX = 20
const KEYS = ['gift_likes', 'gift_dislikes'] as const
const VIS = ['public', 'friends', 'close', 'private'] as const
const { api } = useApi()
const { t } = useI18n()
const { user } = useAuth()
const lists = reactive<Record<(typeof KEYS)[number], string[]>>({ gift_likes: [...(user.value?.gift_likes ?? [])], gift_dislikes: [...(user.value?.gift_dislikes ?? [])] })
const draft = reactive({ gift_likes: '', gift_dislikes: '' })
const picked = ref<string[]>([...(user.value?.interests ?? [])])
const prefsVis = ref(user.value?.gift_prefs_vis ?? 'friends'), interestsVis = ref(user.value?.interests_vis ?? 'public')
const busy = ref(false), err = ref(''), msg = ref('')
function add(k: (typeof KEYS)[number]) {
  const v = draft[k].trim()
  if (!v || lists[k].length >= MAX) return
  if (!lists[k].includes(v)) lists[k].push(v)
  draft[k] = ''
}
function move(k: (typeof KEYS)[number], i: number, d: number) {
  const l = lists[k], j = i + d
  if (j >= 0 && j < l.length) [l[i], l[j]] = [l[j]!, l[i]!]
}
function toggle(tag: string) {
  const i = picked.value.indexOf(tag)
  if (i >= 0) picked.value.splice(i, 1)
  else if (picked.value.length < MAX_INTERESTS) picked.value.push(tag)
}
async function save() {
  if (busy.value) return
  busy.value = true; err.value = msg.value = ''
  try {
    user.value = await api('/me', { method: 'PATCH', body: { gift_likes: lists.gift_likes, gift_dislikes: lists.gift_dislikes, gift_prefs_vis: prefsVis.value, interests: picked.value, interests_vis: interestsVis.value } })
    msg.value = t('settings.saved')
  } catch (e: any) { err.value = apiErrMsg(e) } finally { busy.value = false }
}
</script>
<template>
  <section id="gift-prefs" class="c-card">
    <h2 class="c-h2">{{ $t('settings.pc.gp.title') }}</h2>
    <form @submit.prevent="save">
      <fieldset v-for="k in KEYS" :key="k" class="c-field pf-set">
        <legend class="c-lbl">{{ $t(k === 'gift_likes' ? 'settings.pc.gp.likes' : 'settings.pc.gp.dislikes') }}</legend>
        <ul class="identity-list">
          <li v-for="(x, i) in lists[k]" :key="x" class="c-row">
            <span class="c-grow">{{ x }}</span>
            <button type="button" class="c-btn small" data-up :disabled="i === 0" :aria-label="$t('settings.pc.up')" @click="move(k, i, -1)"><UiIcon name="arrow-up" :size="16" /></button>
            <button type="button" class="c-btn small" data-down :disabled="i === lists[k].length - 1" :aria-label="$t('settings.pc.down')" @click="move(k, i, 1)"><UiIcon name="arrow-down" :size="16" /></button>
            <button type="button" class="c-btn small danger" @click="lists[k].splice(i, 1)">{{ $t('settings.pc.remove') }}</button>
          </li>
        </ul>
        <div class="c-row">
          <input v-model="draft[k]" maxlength="40" :placeholder="$t(k === 'gift_likes' ? 'settings.pc.gp.ph' : 'settings.pc.gp.phDislike')" :aria-label="$t(k === 'gift_likes' ? 'settings.pc.gp.likes' : 'settings.pc.gp.dislikes')" @keydown.enter.prevent="add(k)">
          <button type="button" class="c-btn" data-add :disabled="lists[k].length >= MAX || !draft[k].trim()" @click="add(k)">{{ $t('settings.pc.add') }}</button>
        </div>
        <small v-if="lists[k].length >= MAX" class="c-mute">{{ $t('settings.pc.gp.max') }}</small>
      </fieldset>
      <label class="c-field"><span>{{ $t('settings.pc.showOn') }}</span>
        <select v-model="prefsVis"><option v-for="v in VIS" :key="v" :value="v">{{ $t(`settings.pc.vis.${v}`) }}</option></select>
      </label>
      <fieldset class="c-field pf-set">
        <legend class="c-lbl">{{ $t('settings.pc.gp.interests') }} ({{ $t('settings.pc.gp.count', { n: picked.length, max: MAX_INTERESTS }) }})</legend>
        <div v-for="g in INTEREST_GROUPS" :key="g.id" class="pf-group">
          <p class="c-mute c-small">{{ $t(`interests.group.${g.id}`) }}</p>
          <div class="c-row wrap">
            <button v-for="tag in g.tags" :key="tag" type="button" class="c-btn small" :class="{ on: picked.includes(tag) }" :aria-pressed="picked.includes(tag)" data-chip :disabled="!picked.includes(tag) && picked.length >= MAX_INTERESTS" @click="toggle(tag)">{{ $t(`interests.tag.${tag}`) }}</button>
          </div>
        </div>
      </fieldset>
      <label class="c-field"><span>{{ $t('settings.pc.gp.interestsVis') }}</span>
        <select v-model="interestsVis"><option v-for="v in VIS" :key="v" :value="v">{{ $t(`settings.pc.vis.${v}`) }}</option></select>
      </label>
      <p v-if="err" class="c-err" role="alert">{{ err }}</p>
      <p v-if="msg" class="c-ok" role="status">{{ msg }}</p>
      <button class="c-btn" :disabled="busy">{{ $t('settings.save') }}</button>
    </form>
  </section>
</template>
<style scoped>
.pf-set{border:0;padding:0;min-width:0}
.pf-group{margin:0 0 10px}
</style>
