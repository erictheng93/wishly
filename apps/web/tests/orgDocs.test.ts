import { describe, expect, it, vi } from 'vitest'
import { addPermitWithDoc, docFileError } from '../app/utils/orgDocs'

describe('docFileError', () => {
  it('檔案過大或型別不符回對應 key', () => {
    expect(docFileError({ size: 10 * 1024 * 1024 + 1, type: 'application/pdf' })).toBe('org.apply.fileBig')
    expect(docFileError({ size: 1, type: 'image/gif' })).toBe('org.apply.fileType')
    expect(docFileError({ size: 10 * 1024 * 1024, type: 'image/png' })).toBeNull()
  })
})

describe('addPermitWithDoc', () => {
  const permit = { permit_no: 'A1', valid_from: '2026-01-01', valid_to: '2026-12-31' }
  const file = new File(['x'], 'a.pdf', { type: 'application/pdf' })
  it('建立許可失敗時刪除剛上傳的文件並拋出原錯誤', async () => {
    const err = new Error('boom')
    const api = vi.fn(async (path: string, o?: any) => {
      if (path.endsWith('/documents') && o.method === 'POST') return { id: 'd1' }
      if (path.endsWith('/permits')) throw err
      return null
    })
    await expect(addPermitWithDoc(api as any, 'o1', permit, file)).rejects.toBe(err)
    expect(api).toHaveBeenLastCalledWith('/orgs/o1/documents/d1', { method: 'DELETE' })
  })
  it('成功時帶 document_id 且不刪文件', async () => {
    const api = vi.fn(async (path: string) => (path.endsWith('/documents') ? { id: 'd2' } : null))
    await addPermitWithDoc(api as any, 'o1', permit, file)
    expect(api).toHaveBeenCalledWith('/orgs/o1/permits', { method: 'POST', body: { ...permit, document_id: 'd2' } })
    expect(api).toHaveBeenCalledTimes(2)
  })
})
