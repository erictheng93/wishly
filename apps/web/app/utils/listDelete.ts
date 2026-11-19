// 永久刪除清單被 409 擋下時的文案：依 errors[] 的 ACTIVE_CLAIMS / ACTIVE_ORDERS 帶數量組成，兩種都有就都顯示；否則回 null 由呼叫端用通用錯誤
export function activeBlockMsg(e: any, t: (key: string, params?: Record<string, unknown>) => string): string | null {
  const parts = (e?.errors ?? []).flatMap((x: any) =>
    x?.code === 'ACTIVE_CLAIMS' ? [t('lists.dashboard.purgeActiveClaims', { count: x.count })]
      : x?.code === 'ACTIVE_ORDERS' ? [t('lists.dashboard.purgeActiveOrders', { count: x.count })] : [])
  return parts.length ? parts.join(' ') : null
}
