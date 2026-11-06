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
  const el = await open({ links: [{ title: 'IG', url: 'https://instagram.com/a' }], home_address: '台北市某路 1 號', gift_likes: '書' })
  const a = el.querySelector('a')!
  expect(a.getAttribute('rel')).toBe('nofollow ugc noopener noreferrer')
  expect(a.getAttribute('target')).toBe('_blank')
  expect(el.textContent).toContain('家裡地址')
  expect(el.textContent).toContain('台北市某路 1 號')
})
