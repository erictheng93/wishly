// 個人頁路徑：沒設 handle 的人用 user id（後端 /users/{handle|id} 兩者皆可）
export const profilePath = (u: { id: string, handle?: string | null }) => `/u/${u.handle || u.id}`
