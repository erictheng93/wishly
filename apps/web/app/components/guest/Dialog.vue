<script setup lang="ts">
import '~/assets/guest.css'
// 訪客端 bottom sheet 的 modal 外殼：原生 <dialog>.showModal()（焦點鎖定、Esc、inert 背景由瀏覽器處理）。
// 由父層 v-if 掛載即開啟；dirty 時 Esc / 點背景要二次確認，避免丟掉已輸入內容。
const props = defineProps<{ labelledby: string, dirty?: boolean }>()
const emit = defineEmits<{ close: [] }>()
const { t } = useI18n()
const el = ref<HTMLDialogElement>()
let opener: HTMLElement | null = null
const leave = () => !props.dirty || confirm(t('guest.discardConfirm'))
onBeforeMount(() => { opener = document.activeElement as HTMLElement | null })
onMounted(() => {
  el.value!.showModal()
  // 初始焦點：第一個可輸入欄位，沒有就第一個連結 / 按鈕
  const q = (sel: string) => el.value!.querySelector<HTMLElement>(sel)
  ;(q('input:not([type=hidden]):not([readonly]), textarea') ?? q('a[href], button:not(:disabled)'))?.focus()
})
// 父層 v-if 移除時 <dialog> 直接消失，瀏覽器不會還焦點；且 modal 開著時背景是 inert，所以先 close() 再自己還
onBeforeUnmount(() => { el.value?.close(); if (opener?.isConnected) opener.focus() })
</script>

<template>
  <dialog ref="el" class="g-dialog" :aria-labelledby="labelledby"
    @cancel.prevent="leave() && emit('close')" @close="emit('close')" @click.self="leave() && emit('close')">
    <slot />
  </dialog>
</template>

<style>
dialog.g-dialog .g-sheet h1{font-weight:900;letter-spacing:-.02em}
dialog.g-dialog .g-sheet.center{padding-top:40px}
dialog.g-dialog .g-sheet.center h1::before{content:"OK" / "";display:block;width:max-content;margin:0 auto 14px;padding:3px 10px;border:1px solid currentColor;font:700 11px var(--mono);letter-spacing:.2em}
dialog.g-dialog .g-sheet input[type=checkbox],dialog.g-dialog .g-sheet input[type=radio]{width:20px;height:20px;flex:none}
dialog.g-dialog .g-sheet fieldset legend{font:600 11px var(--mono);letter-spacing:.12em;text-transform:uppercase;margin-bottom:4px}
dialog.g-dialog .g-sheet fieldset label.g-radio{align-items:center;margin:0!important;border-width:0 0 1px;background:transparent;padding:0 2px;font-size:inherit!important;line-height:inherit}
dialog.g-dialog .g-sheet label.g-radio input{margin-top:1px}
dialog.g-dialog .g-sheet fieldset label.g-radio input{margin-top:0}
@media (max-width:767px){
  dialog.g-dialog .g-sheet{border-top-width:0}
  dialog.g-dialog .g-sheet::before{content:"";display:block;width:40px;height:4px;background:var(--line);margin:-14px auto 16px}
}
</style>
