// 摯友名單（帳號層級）；friends.vue 與設定頁卡片共用同一份狀態
export function useCloseFriends() {
  const { api } = useApi()
  const ids = useState<Set<string>>('close-friends-ids', () => new Set())
  const list = useState<any[]>('close-friends-list', () => [])
  async function load() {
    list.value = (await api('/me/close-friends')).users ?? []
    ids.value = new Set(list.value.map(u => u.id))
  }
  // 樂觀更新，失敗還原並把錯誤丟給呼叫端
  async function toggle(id: string, on: boolean) {
    const prev = ids.value, next = new Set(prev)
    on ? next.add(id) : next.delete(id)
    ids.value = next
    try { await api(`/me/close-friends/${id}`, { method: on ? 'PUT' : 'DELETE' }) }
    catch (e) { ids.value = prev; throw e }
  }
  return { ids, list, load, toggle }
}
