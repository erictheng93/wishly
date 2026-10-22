// @vitest-environment happy-dom
import { readdirSync, readFileSync } from 'node:fs'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createI18n } from 'vue-i18n'
import { CreatorApiError, errMsg, useApi } from '../app/composables/useApi'
import { setGuestToken, useGuest } from '../app/composables/useGuest'
import { apiErrMsg } from '../app/utils/errors'

const messages = Object.fromEntries(['en', 'vi', 'zh-Hant'].map(locale => [locale,
  Object.assign({}, ...readdirSync(`i18n/locales/${locale}`).map(file => JSON.parse(readFileSync(`i18n/locales/${locale}/${file}`, 'utf8')))),
]))
const i18n = createI18n({ legacy: false, locale: 'en', fallbackLocale: false, messages }).global
const fetch = vi.fn()
beforeEach(() => {
  setGuestToken(null)
  vi.stubGlobal('useNuxtApp', () => ({ $i18n: i18n }))
  vi.stubGlobal('useRouter', () => ({ currentRoute: { value: { fullPath: '/' } } }))
  vi.stubGlobal('useRuntimeConfig', () => ({ public: { apiBase: '' } }))
  vi.stubGlobal('nativeHeaders', () => ({}))
  vi.stubGlobal('$fetch', fetch)
})
afterEach(() => { vi.unstubAllGlobals(); vi.resetAllMocks() })

// Removing code lookup, field normalization, or passing problem extensions must break these checks.
describe.each([
  ['en', 'This action is no longer allowed. Refresh the page and try again.', 'Only 1 item(s) remain. Please reduce the quantity.', 'Please enter a valid phone number.'],
  ['vi', 'Không thể thực hiện thao tác này nữa. Hãy tải lại trang rồi thử lại.', 'Chỉ còn 1 món. Vui lòng giảm số lượng.', 'Vui lòng nhập số điện thoại hợp lệ.'],
])('%s API errors', (locale, stale, remaining, phone) => {
  beforeEach(() => { i18n.locale.value = locale })
  it.each(['creator', 'guest'])('%s translates banners and fields while retaining error metadata', async client => {
    const api = client === 'creator' ? useApi().api : useGuest().api
    const data = { code: 'INVALID_STATE_TRANSITION', detail: '不允許的狀態轉換', claim_id: 'claim' }
    fetch.mockRejectedValue({ statusCode: 409, response: { status: 409 }, data })
    const error = await api('/claims/claim', { method: 'PATCH' }).catch(e => e)
    expect(error.detail).toBe(stale)
    expect(error.message).toBe(stale)
    expect(error.code).toBe(data.code)
    expect((error.body ?? error.data).claim_id).toBe('claim')
    if (client === 'creator') expect(errMsg(error)).toBe(stale)

    fetch.mockRejectedValue({ statusCode: 409, response: { status: 409 }, data: { code: 'ITEM_FULLY_CLAIMED', detail: '此品項剩餘 1 件', remaining: 1 } })
    const full = await api('/items/item/claims').catch(e => e)
    expect(full.detail).toBe(remaining)
    expect((full.body ?? full.data).remaining).toBe(1)

    fetch.mockRejectedValue({ statusCode: 422, response: { status: 422 }, data: { code: 'VALIDATION_FAILED', detail: '電話格式不正確', errors: [{ pointer: '/phone', code: 'FORMAT', detail: '電話只能包含數字、+、- 與空白' }] } })
    const invalid = await api('/wishlists/list/shipping-address').catch(e => e)
    const fields = invalid.errors ?? invalid.data.errors
    expect(fields[0]).toEqual({ pointer: '/phone', code: 'FORMAT', detail: phone })
    if (client === 'creator') expect(errMsg(invalid)).toBe(phone)
  })

  it('does not leak untranslated or server details, even for unknown codes', () => {
    for (const code of ['FUTURE_CODE', 'VALIDATION_FAILED', 'ITEM_FULLY_CLAIMED']) {
      const text = apiErrMsg({ status: 422, code, detail: '後端中文訊息' })
      expect(text).not.toMatch(/[\u3400-\u9fff]|\{\w+\}/)
      expect(text).not.toBe(`errors.${code}`)
    }
    expect(apiErrMsg({ status: 500, code: 'FUTURE_CODE', detail: 'secret database error' })).not.toContain('secret')
  })

  it('errMsg also translates errors constructed outside the transport', () => {
    expect(errMsg(new CreatorApiError(409, 'INVALID_STATE_TRANSITION', '不允許的狀態轉換'))).toBe(stale)
  })
})

it('resolves messages from the current locale on each call', () => {
  const e = { status: 409, code: 'INVALID_STATE_TRANSITION', detail: '不允許的狀態轉換' }
  i18n.locale.value = 'en'
  expect(apiErrMsg(e)).toBe('This action is no longer allowed. Refresh the page and try again.')
  i18n.locale.value = 'vi'
  expect(apiErrMsg(e)).toBe('Không thể thực hiện thao tác này nữa. Hãy tải lại trang rồi thử lại.')
  i18n.locale.value = 'zh-Hant'
  expect(apiErrMsg(e)).toBe(e.detail)
})
