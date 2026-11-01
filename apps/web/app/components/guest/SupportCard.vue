<script setup lang="ts">
import '~/assets/guest.css'
// 支持品項（funding_mode=support）：導向另一張清單，不可認捐、無進度條。方案 A：公開累計成果。
const props = defineProps<{ item: any, closed: boolean, index?: number }>()
const sp = computed(() => props.item.support ?? {})
const target = computed(() => sp.value.target)
const hasNums = computed(() => sp.value.points != null && sp.value.qty != null)
const canGo = computed(() => !!target.value && !sp.value.ended && !props.closed)
const idx = computed(() => String(props.index ?? 0).padStart(2, '0'))
</script>
<template>
  <li :id="`item-${item.id}`" class="g-card g-item sp-card" :class="{ full: sp.ended }">
    <img v-if="target" class="g-thumb" :src="coverSrc(target)" alt="" loading="lazy">
    <div v-else class="g-thumb">{{ $t('guest.card.thumb') }}</div>
    <div class="g-body">
      <div class="sp-kind"><span v-if="index" aria-hidden="true">No.{{ idx }}　</span>{{ $t('guest.support.kind') }}</div>
      <div class="g-title">{{ item.title }}</div>
      <div v-if="target" class="g-mute">{{ [target.owner_name, target.title].filter(Boolean).join('・') }}</div>
      <dl v-if="hasNums" class="sp-nums">
        <div><dt>{{ $t('guest.support.points') }}</dt><dd class="g-num">{{ fmtPts(sp.points) }}</dd></div>
        <div><dt>{{ $t('guest.support.qty') }}</dt><dd class="g-num">{{ sp.qty }}</dd></div>
      </dl>
      <div class="g-actions">
        <NuxtLink v-if="canGo" class="g-btn sm" :to="`/s/${target.slug}?via=${item.id}`">{{ $t('guest.support.go') }}</NuxtLink>
        <span v-else-if="sp.ended" class="g-mute">{{ $t('guest.support.ended') }}</span>
      </div>
    </div>
  </li>
</template>

<style scoped>
.g-item{border-top:2px solid var(--ink-line)}
.sp-kind{font:700 11px var(--mono);letter-spacing:.12em;color:var(--red)}
.sp-nums{display:flex;gap:18px;margin:10px 0 4px;padding:8px 0;border-top:1px solid var(--line);border-bottom:1px solid var(--line)}
.sp-nums div{display:flex;flex-direction:column}
.sp-nums dd{order:-1;margin:0;font:700 19px var(--mono);line-height:1.2}
.sp-nums dt{font-size:12px;color:var(--mute)}
</style>
