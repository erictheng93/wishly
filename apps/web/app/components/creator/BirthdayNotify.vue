<script setup lang="ts">
// 生日清單通知開關（new / edit 共用；組織與據點清單不要放這個元件）
const on = defineModel<boolean>({ required: true })
const props = defineProps<{ visibility: string }>()
const { user } = useAuth()
const block = computed(() => birthdayNotifyBlock(user.value, props.visibility))
watch(block, b => { if (b) on.value = false }, { immediate: true }) // 不能開就取消勾選
</script>
<template>
  <div class="c-field">
    <label class="c-switch">
      <input v-model="on" type="checkbox" :disabled="!!block">
      <span>{{ $t('creator.bday.label') }}<br>
        <small class="c-mute">{{ block ? $t(`creator.bday.block.${block}`) : $t(`creator.bday.who.${visibility === 'selected' ? 'selected' : 'friends'}`) }}</small>
      </span>
    </label>
    <p v-if="block && block !== 'listPrivate'" class="c-mute c-m0"><small><NuxtLink to="/settings">{{ $t('creator.bday.goSettings') }}</NuxtLink></small></p>
    <p class="c-mute c-m0"><small>{{ $t('creator.bday.single') }}</small></p>
  </div>
</template>
