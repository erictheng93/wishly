<script setup lang="ts">
// 個人頁的資料區塊：後端依觀看者過濾，缺鍵 = 看不到，這裡不判斷權限
import { formatBirthday } from '~/utils/birthday'
import { linkIcon } from '~/utils/linkIcon'
defineProps<{ user: any }>()
const { te } = useI18n()
const interest = (id: string, $t: (k: string) => string) => (te(`interests.tag.${id}`) ? $t(`interests.tag.${id}`) : id)
</script>
<template>
  <div>
    <section v-if="user.links?.length">
      <h2 class="c-h2 pf-sec">{{ $t('me.profile.secLinks') }}</h2>
      <a v-for="l in user.links" :key="l.url" :href="l.url" target="_blank" rel="nofollow ugc noopener noreferrer" class="c-btn block pf-link"><UiIcon :name="(linkIcon(l.url) as any)" :size="18" />{{ l.title }}</a>
    </section>
    <div v-if="user.gift_likes?.length || user.gift_dislikes?.length" class="pf-prefs">
      <section v-if="user.gift_likes?.length">
        <h2 class="c-h2 pf-sec">{{ $t('me.profile.secLikes') }}</h2>
        <p class="c-row wrap"><span v-for="x in user.gift_likes" :key="x" class="c-badge pf-chip">{{ x }}</span></p>
      </section>
      <section v-if="user.gift_dislikes?.length">
        <h2 class="c-h2 pf-sec">{{ $t('me.profile.secDislikes') }}</h2>
        <p class="c-row wrap"><span v-for="x in user.gift_dislikes" :key="x" class="c-badge pf-chip">{{ x }}</span></p>
      </section>
    </div>
    <section v-if="user.interests?.length">
      <h2 class="c-h2 pf-sec">{{ $t('me.profile.secInterests') }}</h2>
      <p class="c-row wrap"><span v-for="id in user.interests" :key="id" class="c-badge">{{ interest(id, $t) }}</span></p>
    </section>
    <section v-if="user.home_address">
      <h2 class="c-h2 pf-sec">{{ $t('me.profile.secAddress') }}</h2>
      <p class="pf-txt">{{ user.home_address }}</p>
    </section>
    <section v-if="user.dates?.length">
      <h2 class="c-h2 pf-sec">{{ $t('me.profile.secDates') }}</h2>
      <ul class="pf-list">
        <li v-for="d in user.dates" :key="d.title + d.month_day"><strong>{{ d.title }}</strong> <span class="c-mute">{{ formatBirthday(d.month_day) }}{{ d.year ? ` ${d.year}` : '' }} · {{ d.days_until ? $t('me.profile.inDays', { n: d.days_until }) : $t('me.profile.dateToday') }}</span></li>
      </ul>
    </section>
  </div>
</template>
<style scoped>
.pf-sec{font:800 16px/1.3 var(--font);letter-spacing:0;text-transform:none;margin:40px 0 8px}
.pf-prefs{display:grid;gap:0 24px}
@media (min-width:480px){.pf-prefs{grid-template-columns:1fr 1fr}}
.pf-chip{font-family:var(--mono);font-size:12px}
.pf-txt{white-space:pre-line;margin:0}
.pf-list{list-style:none;margin:0;padding:0;display:grid;gap:8px}
.pf-link{display:flex;align-items:center;justify-content:center;gap:8px;margin-bottom:8px}
</style>
