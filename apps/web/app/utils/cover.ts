// 心願清單預設封面：id 與後端契約一致，圖在 public/covers/{id}.svg
export const COVER_PRESETS = ['birthday', 'wedding', 'baby', 'housewarming', 'graduation', 'holiday', 'travel', 'gift'] as const
export const DEFAULT_COVER = 'gift'

// 上傳圖優先，其次預設圖；都沒有就顯示 gift（只在前端顯示，不寫回後端）
export function coverSrc(w: { cover_image_url?: string | null, cover_preset?: string | null }): string {
  if (w.cover_image_url) return w.cover_image_url
  const p = w.cover_preset
  return `/covers/${p && (COVER_PRESETS as readonly string[]).includes(p) ? p : DEFAULT_COVER}.svg`
}
