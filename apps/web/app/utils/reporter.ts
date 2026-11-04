// 檢舉者品質提示：駁回達 3 筆且駁回率 ≥ 50% 時標記，提醒客服多看一眼（只是提示，不自動處置）。
export function reporterSuspicious(s: { total: number; dismissed: number } | null | undefined): boolean {
  return !!s && s.total > 0 && s.dismissed >= 3 && s.dismissed / s.total >= 0.5
}
