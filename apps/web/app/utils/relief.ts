// 救災據點的固定值清單：須與後端驗證完全一致（縣市用官方正式名「臺」）
export const COUNTIES = ['臺北市', '新北市', '桃園市', '臺中市', '臺南市', '高雄市', '基隆市', '新竹市', '嘉義市', '新竹縣', '苗栗縣', '彰化縣', '南投縣', '雲林縣', '嘉義縣', '屏東縣', '宜蘭縣', '花蓮縣', '臺東縣', '澎湖縣', '金門縣', '連江縣'] as const
export const HAZARDS = ['earthquake', 'typhoon', 'flood', 'fire', 'other'] as const
export const CATEGORIES = ['water', 'food', 'medical', 'hygiene', 'shelter', 'clothing', 'baby', 'other'] as const
export const URGENCIES = ['critical', 'high', 'medium', 'low'] as const

export const mapsUrl = (address: string): string => `https://www.google.com/maps/search/?api=1&query=${encodeURIComponent(address)}`

/** 今天（Asia/Taipei）的 YYYY-MM-DD；sv-SE 格式即 ISO 日期 */
export const taipeiToday = (): string => new Date().toLocaleDateString('sv-SE', { timeZone: 'Asia/Taipei' })
