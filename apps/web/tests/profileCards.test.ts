// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import UiIcon from '../app/components/UiIcon.vue'
import { computed, nextTick, onMounted, reactive, ref } from 'vue'
import GiftPrefsCard from '../app/components/creator/profile/GiftPrefsCard.vue'
import DatesCard from '../app/components/creator/profile/DatesCard.vue'
import { INTEREST_GROUPS, MAX_INTERESTS } from '../app/utils/interests'
import { daysInMonth, toBirthday } from '../app/utils/birthday'
import { mount, flush } from './vueHarness'

const user = ref<any>({})
const api = vi.fn(async (path: string) => (path === '/me/dates' ? { dates: [] } : {}))
let mounted: ReturnType<typeof mount> | undefined
beforeEach(() => {
  for (const [k, v] of Object.entries({ ref, reactive, computed, onMounted, useApi: () => ({ api }), useAuth: () => ({ user }),
    INTEREST_GROUPS, MAX_INTERESTS, daysInMonth, toBirthday, apiErrMsg: () => 'err' })) vi.stubGlobal(k, v)
  api.mockClear()
})
afterEach(() => { mounted?.unmount(); mounted = undefined; vi.unstubAllGlobals() })
async function open(c: any, u: any = {}) {
  user.value = u
  mounted = mount(c, {}, { UiIcon })
  await flush()
  return mounted.el
}

it('disables the add button when a gift list already has 20 entries', async () => {
  const el = await open(GiftPrefsCard, { gift_likes: Array.from({ length: 20 }, (_, i) => `x${i}`) })
  const input = el.querySelector<HTMLInputElement>('fieldset input')!
  input.value = 'x20'; input.dispatchEvent(new Event('input')); await nextTick()
  expect(el.querySelector<HTMLButtonElement>('[data-add]')!.disabled).toBe(true)
  expect(el.querySelectorAll<HTMLButtonElement>('[data-add]')[1]!.disabled).toBe(true) // 空輸入
})

it('disables unselected interest chips once 10 are picked', async () => {
  const tags = INTEREST_GROUPS.flatMap(g => g.tags)
  const el = await open(GiftPrefsCard, { interests: tags.slice(0, MAX_INTERESTS) })
  const chips = [...el.querySelectorAll<HTMLButtonElement>('[data-chip]')]
  expect(chips.filter(c => c.disabled)).toHaveLength(tags.length - MAX_INTERESTS)
  expect(chips.slice(0, MAX_INTERESTS).every(c => !c.disabled)).toBe(true)
})

it('requires a year once a date stops recurring', async () => {
  const el = await open(DatesCard)
  const year = el.querySelector<HTMLInputElement>('[data-year]')!
  expect(year.required).toBe(false)
  el.querySelector<HTMLInputElement>('[data-recurring]')!.click(); await nextTick()
  expect(year.required).toBe(true)
})

it('edits an existing date through PATCH instead of adding a new one', async () => {
  const row = { id: 'd1', title: '生日', month_day: '05-01', year: null, recurring: true, visibility: 'friends' }
  api.mockImplementation(async (path: string) => (path === '/me/dates' ? { dates: [row] } : {}))
  const el = await open(DatesCard)
  el.querySelector<HTMLButtonElement>('[data-edit]')!.click(); await nextTick()
  const title = el.querySelector<HTMLInputElement>('form input')!
  expect(title.value).toBe('生日')
  title.value = '新生日'; title.dispatchEvent(new Event('input')); await nextTick()
  el.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true })); await flush()
  expect(api).toHaveBeenCalledWith('/me/dates/d1', expect.objectContaining({ method: 'PATCH', body: expect.objectContaining({ title: '新生日', month_day: '05-01' }) }))
  api.mockImplementation(async (path: string) => (path === '/me/dates' ? { dates: [] } : {}))
})

it('reorders gift entries with the down button and sends that order', async () => {
  const el = await open(GiftPrefsCard)
  const input = el.querySelector<HTMLInputElement>('fieldset input')!
  for (const v of ['a', 'b']) {
    input.value = v; input.dispatchEvent(new Event('input')); await nextTick()
    el.querySelector<HTMLButtonElement>('[data-add]')!.click(); await nextTick()
  }
  el.querySelector<HTMLButtonElement>('[data-down]')!.click(); await nextTick()
  el.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true })); await flush()
  expect(api).toHaveBeenCalledWith('/me', expect.objectContaining({ method: 'PATCH', body: expect.objectContaining({ gift_likes: ['b', 'a'] }) }))
})
