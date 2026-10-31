<script setup lang="ts">
// 渠道比較：實收、筆數、平均、成功率、付費人數
const props = defineProps<{ days: number }>()
const { t, te } = useI18n()
const { api } = useApi()
const data = ref<any>(null), busy = ref(false), err = ref('')
let seq = 0
async function load() {
  const my = ++seq // 區間快速切換時只採用最後一次的回應
  busy.value = true; err.value = ''
  try {
    const r = await api('/admin/finance/channels', { query: { days: props.days } })
    if (my === seq) data.value = r
  } catch (e) { if (my === seq) err.value = errMsg(e) }
  finally { if (my === seq) busy.value = false }
}
watch(() => props.days, load)
onMounted(load)

const money = (n: number | null) => n == null ? '–' : t('admin.finance.money', { n: fmtPts(Math.round(n)) })
const label = (m: string) => methodLabel(m, t, te)
</script>
<template>
  <div>
    <p v-if="err" class="c-err" role="alert">{{ err }} <button class="c-btn small" :disabled="busy" @click="load">{{ $t('admin.finance.retry') }}</button></p>
    <p v-else-if="!data" class="c-mute">{{ $t('admin.finance.loading') }}</p>
    <section v-else class="c-card" :aria-label="$t('admin.finance.channels.title')">
      <h2 class="c-h2">{{ $t('admin.finance.channels.title') }}</h2>
      <p class="c-mute">{{ $t('admin.finance.range', { from: data.range.from, to: data.range.to, days: data.range.days }) }}</p>
      <p class="c-mute">{{ $t('admin.finance.channels.note') }}</p>
      <p v-if="!data.channels?.length" class="c-mute">{{ $t('admin.finance.channels.empty') }}</p>
      <div v-else class="fin-scroll">
        <table class="fin-table">
          <thead><tr>
            <th>{{ $t('admin.finance.channels.method') }}</th><th>{{ $t('admin.finance.channels.gross') }}</th><th>{{ $t('admin.finance.channels.count') }}</th>
            <th>{{ $t('admin.finance.channels.avg') }}</th><th>{{ $t('admin.finance.channels.success') }}</th><th>{{ $t('admin.finance.channels.payers') }}</th>
          </tr></thead>
          <tbody>
            <tr v-for="c in data.channels" :key="c.method">
              <td>{{ label(c.method) }}</td><td>{{ money(c.gross) }}</td><td>{{ fmtCount(c.count) }}</td>
              <td>{{ money(c.avg) }}</td><td>{{ fmtRate(c.success_rate) }}</td><td>{{ fmtCount(c.payers) }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>
  </div>
</template>
