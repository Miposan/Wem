import axios from 'axios'
import type {
  ApiResponse,
  Block,
  BatchReq,
  BatchResult,
  CreateBlockReq,
  CreateDocumentReq,
  DeleteResult,
  DocumentChildrenResult,
  DocumentContentResult,
  ExportResult,
  HistoryEntry,
  ImportTextReq,
  ImportResult,
  MergeReq,
  MergeResult,
  MoveBlockReq,
  RestoreResult,
  RollbackReq,
  SplitReq,
  SplitResult,
  UpdateBlockReq,
} from '@/types/api'

// ---- Axios 实例 ----

const api = axios.create({
  baseURL: import.meta.env.VITE_API_BASE_URL ?? 'http://localhost:6809/api/v1',
  headers: { 'Content-Type': 'application/json' },
  timeout: 10_000,
})

// ---- 拦截器：统一解包 { code, msg, data } ----

api.interceptors.response.use(
  (res) => {
    const body = res.data as ApiResponse
    if (body.code !== 0) {
      return Promise.reject(new ApiError(res.status, body.code, body.msg))
    }
    res.data = body.data
    return res
  },
  (error) => {
    if (error.response) {
      const body = error.response.data as ApiResponse
      const code = body?.code ?? error.response.status
      const msg = body?.msg ?? error.message
      return Promise.reject(new ApiError(error.response.status, code, msg))
    }
    return Promise.reject(error)
  },
)

// ---- Error ----

export class ApiError extends Error {
  constructor(
    public status: number,
    public code: number,
    msg: string,
  ) {
    super(msg)
    this.name = 'ApiError'
  }
}

// ---- Helpers ----

function get<T>(url: string, params?: Record<string, unknown>) {
  return api.get<T>(url, { params }).then((r) => r.data as T)
}

function post<T>(url: string, data?: unknown) {
  return api.post<T>(url, data).then((r) => r.data as T)
}

function put<T>(url: string, data?: unknown) {
  return api.put<T>(url, data).then((r) => r.data as T)
}

function del<T>(url: string, params?: Record<string, unknown>) {
  return api.delete<T>(url, { params }).then((r) => r.data as T)
}

// =====================================================
//  API Functions
// =====================================================

// ---------- Health ----------

export function healthCheck() {
  return get<null>('/health')
}

// ---------- Document ----------

export function listDocuments() {
  return get<Block[]>('/documents')
}

export function createDocument(req: CreateDocumentReq) {
  return post<Block>('/documents', req)
}

export function getDocument(id: string) {
  return get<DocumentContentResult>(`/documents/${id}`)
}

export function deleteDocument(id: string) {
  return del<DeleteResult>(`/documents/${id}`)
}

export function getDocumentChildren(id: string) {
  return get<DocumentChildrenResult>(`/documents/${id}/children`)
}

export function exportDocument(id: string, format = 'markdown') {
  return get<ExportResult>(`/documents/${id}/export`, { format })
}

// ---------- Block ----------

export function createBlock(req: CreateBlockReq) {
  return post<Block>('/blocks', req)
}

export function getBlock(id: string, includeDeleted = false) {
  return get<Block>(`/blocks/${id}`, { include_deleted: includeDeleted })
}

export function updateBlock(id: string, req: UpdateBlockReq) {
  return put<Block>(`/blocks/${id}`, req)
}

export function deleteBlock(id: string) {
  return del<DeleteResult>(`/blocks/${id}`)
}

export function moveBlock(id: string, req: MoveBlockReq) {
  return post<Block>(`/blocks/${id}/move`, req)
}

export function restoreBlock(id: string) {
  return post<RestoreResult>(`/blocks/${id}/restore`)
}

// getChildren 已删除：MVP 阶段通过 getDocument 获取完整内容树

// ---------- Batch ----------

export function batchBlocks(req: BatchReq) {
  return post<BatchResult>('/blocks/batch', req)
}

// ---------- Import ----------

export function importText(req: ImportTextReq) {
  return post<ImportResult>('/blocks/import', req)
}

// ---------- History / Version ----------

export function getBlockHistory(id: string, limit = 50) {
  return get<HistoryEntry[]>(`/blocks/${id}/history`, { limit })
}

export function getBlockVersion(id: string, version: number) {
  return get<unknown>(`/blocks/${id}/versions/${version}`)
}

export function rollbackBlock(id: string, req: RollbackReq) {
  return post<unknown>(`/blocks/${id}/rollback`, req)
}

export function createSnapshot(id: string) {
  return post<unknown>(`/blocks/${id}/snapshot`)
}

// ---------- Split / Merge 意图 API ----------

export function splitBlock(id: string, req: SplitReq) {
  return post<SplitResult>(`/blocks/${id}/split`, req)
}

export function mergeBlock(id: string, req: MergeReq) {
  return post<MergeResult>(`/blocks/${id}/merge`, req)
}
