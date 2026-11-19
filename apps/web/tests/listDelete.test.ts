import { expect, it } from 'vitest'
import { activeBlockMsg } from '../app/utils/listDelete'
import { t } from './setup'

it('認領與採購兩種 409 都帶數量並列', () => {
  const e = { errors: [{ code: 'ACTIVE_CLAIMS', count: 2 }, { code: 'ACTIVE_ORDERS', count: 1 }] }
  const m = activeBlockMsg(e, t)!
  expect(m).toContain('2 筆進行中的認領')
  expect(m).toContain('1 筆進行中的採購')
})
it('只有一種時只顯示該種；無關錯誤回 null', () => {
  expect(activeBlockMsg({ errors: [{ code: 'ACTIVE_ORDERS', count: 3 }] }, t)).toContain('3 筆進行中的採購')
  expect(activeBlockMsg({ errors: [{ code: 'ACTIVE_ORDERS', count: 3 }] }, t)).not.toContain('認領')
  expect(activeBlockMsg({ errors: [] }, t)).toBeNull()
  expect(activeBlockMsg(undefined, t)).toBeNull()
})
