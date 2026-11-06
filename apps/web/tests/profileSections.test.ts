// @vitest-environment happy-dom
import { afterEach, expect, it } from 'vitest'
import ProfileSections from '../app/components/creator/ProfileSections.vue'
import { mount, flush } from './vueHarness'

let m: ReturnType<typeof mount> | undefined
afterEach(() => { m?.unmount(); m = undefined })
async function open(user: any) {
  m = mount(ProfileSections, { user }, { UiIcon: { props: ['name'], template: '<i />' } })
  await flush()
  return m.el
}

it('renders no block title when the keys are missing', async () => {
  const el = await open({ display_name: 'A' })
  expect(el.textContent).not.toContain('喜歡的禮物')
  expect((await open({ gift_likes: [], gift_dislikes: [] })).querySelector('section')).toBeNull()
  expect(el.textContent).not.toContain('家裡地址')
  expect(el.querySelector('section')).toBeNull()
})

it('shows the nearest date with days_until', async () => {
  const el = await open({ dates: [{ title: '結婚紀念日', month_day: '12-01', days_until: 25 }, { title: '搬家', month_day: '03-02', year: 2027, days_until: 120 }] })
  expect(el.textContent).toContain('結婚紀念日')
  expect(el.textContent).toContain('還有 25 天')
  expect(el.querySelectorAll('li')[0].textContent).toContain('結婚紀念日')
})

it('opens links in a new tab with safe rel', async () => {
  const el = await open({ links: [{ title: 'IG', url: 'https://instagram.com/a' }], home_address: '台北市某路 1 號', gift_likes: ['書'] })
  const a = el.querySelector('a')!
  expect(a.getAttribute('rel')).toBe('nofollow ugc noopener noreferrer')
  expect(a.getAttribute('target')).toBe('_blank')
  expect(el.textContent).toContain('家裡地址')
  expect(el.textContent).toContain('台北市某路 1 號')
})

it('renders gift preferences as chips, not JSON', async () => {
  const el = await open({ gift_likes: ['黑巧克力', '書'], gift_dislikes: ['香水'] })
  const chips = [...el.querySelectorAll('.c-badge')].map(c => c.textContent)
  expect(chips).toEqual(['黑巧克力', '書', '香水'])
  expect(el.textContent).not.toContain('[')
})

it('orders blocks links, prefs, interests, address, dates', async () => {
  const el = await open({ dates: [{ title: 'D', month_day: '12-01', days_until: 5 }], home_address: 'ADDR', interests: ['x'], gift_likes: ['g'], links: [{ title: 'L', url: 'https://a.b' }] })
  const heads = [...el.querySelectorAll('h2')].map(h => h.textContent)
  expect(heads).toEqual(['連結', '喜歡的禮物', '興趣', '家裡地址', '重要日子'])
})

it('formats date as month/day and appends year only for one-off dates', async () => {
  const el = await open({ dates: [{ title: 'A', month_day: '12-01', days_until: 5 }, { title: 'B', month_day: '03-02', year: 2027, days_until: 9 }] })
  const li = el.querySelectorAll('li')
  expect(li[0].textContent).toContain('12 月 1 日 ·')
  expect(li[1].textContent).toContain('3 月 2 日 2027')
})
