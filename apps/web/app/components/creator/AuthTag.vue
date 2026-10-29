<script setup lang="ts">
// 登入 / 註冊 / 忘記密碼表單上方的標本籤：純裝飾（aria-hidden），步驟進度用等寬字與方塊表示
defineProps<{ left: string; right: string; step?: number; of?: number }>()
</script>
<template>
  <div class="au-tag" aria-hidden="true">
    <span><i class="au-sq" />{{ left }}</span>
    <span v-if="of" class="au-steps"><i v-for="n in of" :key="n" :class="{ on: n <= (step || 1) }" />{{ right }}</span>
    <span v-else>{{ right }}</span>
  </div>
</template>
<style>
.au-tag{display:flex;align-items:center;justify-content:space-between;gap:12px;margin:0 0 22px;padding:0 0 12px;border-bottom:1px solid var(--line);font:600 11px/1 var(--mono);letter-spacing:.14em;text-transform:uppercase;color:var(--mute)}
.au-tag>span{display:inline-flex;align-items:center;gap:8px}
.au-sq{display:inline-block;width:8px;height:8px;background:var(--red)}
.au-steps i{width:14px;height:4px;background:var(--hatch),var(--bg-alt);outline:1px solid var(--line);outline-offset:-1px}
.au-steps i.on{background:var(--red);outline:0}
.au-steps{gap:4px!important}.au-steps i{width:16px}.au-steps i:last-of-type{margin-right:6px}
.c-auth{position:relative;animation:rise .55s var(--ease) both .05s}
.c-auth-lead{padding-left:14px;border-left:2px solid var(--red);font-size:15px}
@media (min-width:768px){
  .au-tag{border-bottom-color:var(--ink-line)}
  /* 卡片四角的對位記號與位移硬陰影 */
  .c-auth{box-shadow:10px 10px 0 var(--bg-alt),10px 10px 0 1px var(--line)}
  .c-auth::before,.c-auth::after{content:"";position:absolute;width:14px;height:14px;pointer-events:none}
  .c-auth::before{top:-8px;left:-8px;border-top:2px solid var(--red);border-left:2px solid var(--red)}
  .c-auth::after{bottom:-8px;right:-8px;border-bottom:2px solid var(--red);border-right:2px solid var(--red)}
  .c-page:has(>.c-auth) .c-title{margin-top:48px;margin-bottom:8px}
}
</style>
