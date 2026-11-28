// 組織文件 / 勸募許可上傳：org/apply.vue 與 orgs/[id]/index.vue 共用
type Api = <T = any>(path: string, opts?: any) => Promise<T>
export type PermitFields = { permit_no: string, valid_from: string, valid_to: string }

// 回傳 i18n key；通過則 null
export function docFileError(file: { size: number, type: string }): 'org.apply.fileBig' | 'org.apply.fileType' | null {
  if (file.size > 10 * 1024 * 1024) return 'org.apply.fileBig'
  if (!['application/pdf', 'image/jpeg', 'image/png'].includes(file.type)) return 'org.apply.fileType'
  return null
}

// File 不會被 ofetch 序列化；走 api() 才有錯誤正規化與 401 導向
export const uploadOrgDoc = (api: Api, orgId: string, kind: 'registration' | 'permit', file: File) =>
  api<any>(`/orgs/${orgId}/documents`, { method: 'POST', query: { kind }, body: file, headers: { 'Content-Type': file.type } })

// 上傳許可文件後建立許可；建立失敗就刪掉文件（不留孤兒；已上線組織刪文件會 409，故清理失敗要吞掉）再拋出原錯誤
export async function addPermitWithDoc(api: Api, orgId: string, permit: PermitFields, file: File) {
  const d = await uploadOrgDoc(api, orgId, 'permit', file)
  try { await api(`/orgs/${orgId}/permits`, { method: 'POST', body: { ...permit, document_id: d.id } }) } catch (e) {
    await api(`/orgs/${orgId}/documents/${d.id}`, { method: 'DELETE' }).catch(() => {})
    throw e
  }
}
