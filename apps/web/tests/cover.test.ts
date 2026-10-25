import { describe, expect, it } from 'vitest'
import { COVER_PRESETS, coverSrc } from '../app/utils/cover'

describe('coverSrc', () => {
  it('prefers the uploaded image', () => {
    expect(coverSrc({ cover_image_url: 'https://x/a.webp', cover_preset: 'baby' })).toBe('https://x/a.webp')
  })
  it('uses the preset when there is no upload', () => {
    for (const p of COVER_PRESETS) expect(coverSrc({ cover_image_url: null, cover_preset: p })).toBe(`/covers/${p}.svg`)
  })
  it('falls back to gift for none or unknown presets', () => {
    expect(coverSrc({})).toBe('/covers/gift.svg')
    expect(coverSrc({ cover_preset: 'nope' })).toBe('/covers/gift.svg')
  })
})
