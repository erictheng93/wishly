import { computed } from 'vue'

export type MeOrg = { id: string, name: string, role: 'owner' | 'coordinator' | 'volunteer', verification_status: string }
const KEY = 'wishly-org-mode'
const read = () => { try { return localStorage.getItem(KEY) } catch { return null } }

// 身分切換：current 為 org_id，null＝個人。存的 id 若不在 /me.orgs（被移出、組織消失）就視為個人
export function useOrgMode() {
  const { user } = useAuth()
  const saved = useState<string | null>('org-mode', read)
  const orgs = computed<MeOrg[]>(() => user.value?.orgs ?? [])
  const current = computed(() => (saved.value && orgs.value.some(o => o.id === saved.value) ? saved.value : null))
  function switchTo(id: string | null) {
    saved.value = id
    try { id ? localStorage.setItem(KEY, id) : localStorage.removeItem(KEY) } catch {}
    navigateTo(id ? `/orgs/${id}` : '/dashboard')
  }
  return { current, orgs, switchTo }
}
