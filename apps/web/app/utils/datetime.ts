/** API 的 UTC ISO 時間 ⇄ <input type="datetime-local"> 的本地時間字串（精度到分） */
export const toLocalInput = (iso?: string | null): string => {
  if (!iso) return ''
  const d = new Date(iso)
  return new Date(d.getTime() - d.getTimezoneOffset() * 60000).toISOString().slice(0, 16)
}
export const fromLocalInput = (local: string): string | null => (local ? new Date(local).toISOString() : null)
/** PATCH 用：輸入沒變就回 undefined（不送），避免精度被截到分鐘而「改動」到原值（含已過期的值） */
export const closesAtChange = (local: string, original?: string | null): string | null | undefined =>
  local === toLocalInput(original) ? undefined : fromLocalInput(local)
