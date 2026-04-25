import axios from 'axios'
import type {
  ApiResponse,
  Block,
  BatchReq,
  BatchResult,
  CreateBlockReq,
  CreateDocumentReq,
  DeleteBlockReq,
  DeleteDocumentReq,
  DeleteResult,
  DocumentChildrenResult,
  DocumentContentResult,
  ExportBlockReq,
  ExportReq,
  ExportResult,
  GetBlockReq,
  GetChildrenReq,
  GetDocumentReq,
  GetHistoryReq,
  HistoryEntry,
  ImportTextReq,
  ImportResult,
  MergeReq,
  MergeResult,
  MoveBlockReq,
  MoveTreeReq,
  RestoreReq,
  RestoreResult,
  SplitReq,
  SplitResult,
  UndoRedoResult,
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

// =====================================================
//  API Functions — 全 POST RPC 风格
// =====================================================

// ---------- Health ----------

export function healthCheck() {
  return get<null>('/health')
}

// ---------- Document ----------

export function listDocuments() {
  return post<Block[]>('/documents/list')
}

export function createDocument(req: CreateDocumentReq) {
  return post<Block>('/documents', req)
}

export function getDocument(id: string) {
  return post<DocumentContentResult>('/documents/get', { id } as GetDocumentReq)
}

export function deleteDocument(id: string) {
  return post<DeleteResult>('/documents/delete', { id } as DeleteDocumentReq)
}

export function getDocumentChildren(id: string) {
  return post<DocumentChildrenResult>('/documents/children', { id } as GetChildrenReq)
}

export function exportDocument(id: string, format = 'markdown') {
  return post<ExportResult>('/documents/export', { id, format } as ExportReq)
}

// ---------- Block ----------

export function createBlock(req: CreateBlockReq) {
  return post<Block>('/blocks/create', req)
}

export function getBlock(id: string, includeDeleted = false) {
  return post<Block>('/blocks/get', { id, include_deleted: includeDeleted } as GetBlockReq)
}

export function updateBlock(id: string, req: Omit<UpdateBlockReq, 'id'>) {
  return post<Block>('/blocks/update', { id, ...req } as UpdateBlockReq)
}

export function deleteBlock(id: string, editor_id?: string) {
  return post<DeleteResult>('/blocks/delete', { id, editor_id } as DeleteBlockReq)
}

export function deleteTree(id: string, editor_id?: string) {
  return post<DeleteResult>('/blocks/delete-tree', { id, editor_id } as DeleteBlockReq)
}

export function moveBlock(id: string, req: Omit<MoveBlockReq, 'id'>) {
  return post<Block>('/blocks/move', { id, ...req } as MoveBlockReq)
}

export function moveTree(id: string, req: Omit<MoveTreeReq, 'id'>) {
  return post<Block>('/blocks/move-tree', { id, ...req } as MoveTreeReq)
}

export function moveDocument(id: string, req: Omit<MoveTreeReq, 'id'>) {
  return post<Block>('/documents/move', { id, ...req } as MoveTreeReq)
}

export function restoreBlock(id: string, editor_id?: string) {
  return post<RestoreResult>('/blocks/restore', { id, editor_id } as RestoreReq)
}

// getChildren 已删除：MVP 阶段通过 getDocument 获取完整内容树

// ---------- Batch ----------

export function batchBlocks(req: BatchReq) {
  return post<BatchResult>('/blocks/batch', req)
}

// ---------- Export ----------

export function exportBlock(id: string, depth: 'children' | 'descendants' = 'descendants', format = 'markdown') {
  return post<ExportResult>('/blocks/export', { id, format, depth } as ExportBlockReq)
}

// ---------- Import ----------

export function importText(req: ImportTextReq) {
  return post<ImportResult>('/documents/import', req)
}

// ---------- History / Version ----------

export function getDocumentHistory(id: string, limit = 50) {
  return post<HistoryEntry[]>('/documents/history', { id, limit } as GetHistoryReq)
}

// ---------- Undo / Redo ----------

export function undoDocument(documentId: string) {
  return post<UndoRedoResult>('/documents/undo', { document_id: documentId })
}

export function redoDocument(documentId: string) {
  return post<UndoRedoResult>('/documents/redo', { document_id: documentId })
}

// ---------- Split / Merge 意图 API ----------

export function splitBlock(id: string, req: Omit<SplitReq, 'id'>) {
  return post<SplitResult>('/blocks/split', { id, ...req } as SplitReq)
}

export function mergeBlock(id: string, req: Omit<MergeReq, 'id'>) {
  return post<MergeResult>('/blocks/merge', { id, ...req } as MergeReq)
}
