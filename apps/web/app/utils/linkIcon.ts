// 連結按鈕圖示：依網域挑品牌圖示，認不得就用通用連結圖示
const HOSTS: [RegExp, string][] = [
  [/(^|\.)instagram\.com$/, 'instagram'], [/(^|\.)threads\.(net|com)$/, 'threads'], [/(^|\.)(youtube\.com|youtu\.be)$/, 'youtube'],
  [/(^|\.)line\.me$/, 'line'], [/(^|\.)(facebook\.com|fb\.com|fb\.me)$/, 'facebook'], [/(^|\.)tiktok\.com$/, 'tiktok'],
  [/(^|\.)(x\.com|twitter\.com)$/, 'x'], [/(^|\.)github\.com$/, 'github'],
]
export function linkIcon(url: string): string {
  let host = ''
  try { host = new URL(url).hostname.toLowerCase() } catch { return 'link' }
  return HOSTS.find(([re]) => re.test(host))?.[1] ?? 'link'
}
