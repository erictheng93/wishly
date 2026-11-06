<script setup lang="ts">
const { api } = useApi()
const { ids, load, toggle } = useCloseFriends()
const friends = ref<any[] | null>(null), err = ref('')
onMounted(async () => {
  try { [friends.value] = await Promise.all([api('/friends').then(r => r.friends ?? []), load()]) } catch (e) { err.value = errMsg(e) }
})
async function flip(id: string, on: boolean) { err.value = ''; try { await toggle(id, on) } catch (e) { err.value = errMsg(e) } }
</script>
<template>
  <section id="close-friends" class="c-card">
    <h2 class="c-h2">{{ $t('settings.closeFriends.title') }}</h2>
    <p class="c-mute">{{ $t('settings.closeFriends.hint', { n: ids.size }) }}</p>
    <p v-if="err" class="c-err" role="alert">{{ err }}</p>
    <p v-else-if="!friends" class="c-mute" role="status">{{ $t('creator.vis.loading') }}</p>
    <p v-else-if="!friends.length" class="c-mute">{{ $t('creator.vis.noFriendsPre') }}<NuxtLink to="/friends">{{ $t('common.friends') }}</NuxtLink>{{ $t('creator.vis.noFriendsPost') }}</p>
    <label v-for="f in friends" :key="f.id" class="c-switch">
      <input type="checkbox" :checked="ids.has(f.id)" @change="flip(f.id, ($event.target as HTMLInputElement).checked)">
      <span>{{ f.display_name }}<small v-if="f.handle" class="c-mute">　@{{ f.handle }}</small></span>
    </label>
  </section>
</template>
