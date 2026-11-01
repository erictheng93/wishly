// 後台列表遮罩 email：只留第一個字與網域
export const maskEmail = (e: string) => e.replace(/^(.).*(@.*)$/, '$1***$2')
