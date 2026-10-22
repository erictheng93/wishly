<script setup lang="ts">
import '~/assets/guest.css'
const props = defineProps<{ item: any, mine: any | null, masked: boolean, closed: boolean, online: boolean, loginTo?: any }>() // loginTo 有值 = 未登入，認捐鈕改為前往登入
defineEmits<{ claim: [], fund: [] }>()
const crowd = computed(() => props.item.funding_mode === 'crowdfund')
const ds = computed(() => deriveDisplayStatus(props.item) ?? '')
const full = computed(() => crowd.value ? ds.value !== 'open' : props.item.is_fully_claimed)
// image_status 非 ready 時 API 不給 image_url，顯示灰色佔位
</script>
<template>
  <li :id="`item-${item.id}`" class="g-card g-item" :class="{ full }">
    <img v-if="item.image_url" class="g-thumb" :src="item.image_url" alt="" loading="lazy">
    <div v-else class="g-thumb">{{ $t('guest.card.thumb') }}</div>
    <div class="g-body">
      <div class="g-title">{{ item.title }}<span v-if="item.priority" class="g-badge" :class="item.priority">{{ $t('guest.card.priority', { level: $t('guest.card.prio.' + item.priority) }) }}</span><span v-if="crowd && ds" class="g-badge" :class="'ds-' + ds">{{ displayStatusLabel(ds) }}</span></div>
      <div v-if="item.brand || item.spec" class="g-mute">{{ [item.brand, item.spec].filter(Boolean).join('・') }}</div>
      <template v-if="crowd">
        <div class="g-bar" role="progressbar" :aria-valuenow="item.progress_percent ?? 0" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: (item.progress_percent ?? 0) + '%' }" /></div>
        <div v-if="item.pledged_points != null" class="g-mute">{{ $t('guest.card.raised') }} <b class="g-num">{{ fmtPts(item.pledged_points) }} / {{ $t('guest.pts', { n: fmtPts(item.target_points) }) }}</b><template v-if="ds === 'open'">{{ $t('guest.card.leftPts', { n: fmtPts(item.remaining_points) }) }}</template></div>
        <div v-else class="g-mute">{{ $t('guest.card.target') }} <b class="g-num">{{ $t('guest.pts', { n: fmtPts(item.target_points) }) }}</b></div>
        <div v-if="item.funding_deadline && ds === 'open'" class="g-mute">{{ $t('guest.card.until', { time: fmtTime(item.funding_deadline) }) }}</div>
        <div v-if="!masked && item.contributors?.length" class="g-mute">{{ $t('guest.card.contributors', { names: item.contributors.map((c: any) => `${c.display_name} ${$t('guest.pts', { n: fmtPts(c.points) })}`).join($t('guest.listSep')) }) }}</div>
      </template>
      <template v-else>
        <div class="g-bar" role="progressbar" :aria-valuenow="item.progress_percent" aria-valuemin="0" aria-valuemax="100"><i :style="{ width: item.progress_percent + '%' }" /></div>
        <div class="g-mute">{{ $t('guest.card.claimed') }} <b class="g-num">{{ item.qty_claimed }} / {{ item.qty_needed }}</b><template v-if="mine">{{ $t('guest.card.mineQty', { n: mine.qty }) }}</template></div>
        <div v-if="!masked && item.claimers?.length" class="g-mute">{{ $t('guest.card.claimedBy', { names: item.claimers.map((c: any) => c.display_name).join($t('guest.listSep')) }) }}</div>
      </template>
      <div class="g-actions">
        <span v-if="masked" class="g-mute">{{ $t('guest.card.ownerBlocked') }}</span>
        <template v-else-if="crowd">
          <button v-if="ds === 'open' && !closed" class="g-btn sm js-claim" :disabled="!online" @click="$emit('fund')">{{ $t('guest.card.fundBtn') }}</button>
          <span v-else-if="ds && ds !== 'open'" class="g-mute">{{ ds === 'expired' ? $t('guest.card.expired') : $t('guest.card.fundedDone') }}</span>
        </template>
        <button v-else-if="mine && !closed" class="g-btn ghost sm js-claim" :disabled="!online" @click="$emit('claim')">{{ $t('guest.card.editMine') }}</button>
        <span v-else-if="full" class="g-mute">{{ $t('guest.card.fullClaimed') }}</span>
        <NuxtLink v-else-if="!closed && loginTo" class="g-btn sm" :to="loginTo">{{ $t('guest.card.loginClaim') }}</NuxtLink>
        <button v-else-if="!closed" class="g-btn sm js-claim" :disabled="!online" @click="$emit('claim')">{{ $t('guest.card.claimBtn') }}</button>
        <a v-if="item.product_url && !masked" :href="item.product_url" target="_blank" rel="noopener nofollow" class="g-link">{{ $t('guest.card.productLink') }}</a>
      </div>
    </div>
  </li>
</template>
