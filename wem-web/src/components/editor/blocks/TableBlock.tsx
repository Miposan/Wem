/**
 * TableBlock — markdown 表格
 *
 * 交互：
 * - T 区域三角形点击 → 插入行/列（四边均有）
 * - 右键单元格 → 完整上下文菜单（插入/删除/移动/对齐）
 * - 列宽拖拽：鼠标靠近表格内部垂直分隔线 → col-resize → 拖拽调整
 * - 行/列抓手：点击选中行/列，拖拽移动行/列
 * - Tab / Enter 键盘导航
 * - Shift+Enter 单元格内软换行（markdown <br>）
 * - 多单元格拖选 + 内部 Ctrl+C/X/V
 * - Alt+Shift+方向键 移动行/列
 */

import { useRef, useCallback, useEffect, useState } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockAction } from '../core/types'
import { updateBlock } from '@/api/client'
import { ChevronUp, ChevronDown, ChevronLeft, ChevronRight, GripVertical, GripHorizontal } from 'lucide-react'

// ─── Props ───

interface TableBlockProps {
  block: BlockNode
  readonly: boolean
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

// ─── 列对齐类型 ───

type ColumnAlign = 'left' | 'center' | 'right' | ''

// ─── Markdown 解析 / 序列化 ───

function parseCells(md: string): string[][] {
  const lines = md.split('\n').filter((l) => l.trim())
  if (lines.length === 0) return [['']]
  const dataLines = lines.filter((l, i) => i !== 1 || !isDelimiter(l))
  if (dataLines.length === 0) return [['']]
  return dataLines.map((line) => {
    const trimmed = line.trim()
    const inner = trimmed.startsWith('|') ? trimmed.slice(1) : trimmed
    const content = inner.endsWith('|') ? inner.slice(0, -1) : inner
    return content.split('|').map((c) => c.trim().replace(/<br\s*\/?>/gi, '\n'))
  })
}

function parseAlignment(md: string): ColumnAlign[] {
  const lines = md.split('\n').filter((l) => l.trim())
  if (lines.length < 2) return []
  const delimiter = lines[1]?.trim()
  if (!isDelimiter(delimiter)) return []
  const inner = delimiter.startsWith('|') ? delimiter.slice(1, delimiter.endsWith('|') ? -1 : undefined) : delimiter
  return inner.split('|').map((cell) => {
    const c = cell.trim()
    const left = c.startsWith(':')
    const right = c.endsWith(':')
    if (left && right) return 'center'
    if (right) return 'right'
    if (left) return 'left'
    return ''
  })
}

function isDelimiter(line: string): boolean {
  const t = line.trim()
  if (!t.startsWith('|') || !t.endsWith('|') || t.length < 3) return false
  const inner = t.slice(1, -1).trim()
  if (!inner) return false
  return inner.split('|').every((cell) => {
    const c = cell.trim()
    return c.length >= 3 && [...c].every((ch) => ch === '-' || ch === ':' || ch === ' ') && [...c].filter((ch) => ch === '-').length >= 3
  })
}

function normalizeCells(cells: string[][]): string[][] {
  const cols = Math.max(...cells.map((r) => r.length), 1)
  return cells.map((r) => { const row = [...r]; while (row.length < cols) row.push(''); return row })
}

function serializeCells(cells: string[][], aligns: ColumnAlign[]): string {
  if (cells.length === 0) return ''
  const n = normalizeCells(cells)
  const cols = n[0].length
  const w = Array.from({ length: cols }, (_, c) =>
    Math.max(5, ...n.map((r) => r[c].replace(/\n/g, '<br>').length + 2))
  )
  const fmt = (r: string[]) =>
    '| ' + r.map((c, i) => c.replace(/\n/g, '<br>').padEnd(w[i] - 2)).join(' | ') + ' |'
  const delim = '| ' + w.map((v, i) => {
    const a = aligns[i] || ''
    const dash = '-'.repeat(v - 2)
    if (a === 'center') return ':' + dash + ':'
    if (a === 'right') return dash + ':'
    if (a === 'left') return ':' + dash
    return dash
  }).join(' | ') + ' |'
  return [fmt(n[0]), delim, ...n.slice(1).map(fmt)].join('\n')
}

const DEFAULT_TABLE = '| 列 1 | 列 2 |\n|------|-------|\n|      |      |'
const RESIZE_THRESHOLD = 6
const MIN_COL_WIDTH = 50
const MAX_COL_WIDTH = 600
const BORDER_CTX_THRESHOLD = 20

// ─── 组件 ───

export function TableBlock({ block, readonly, onContentChange, onAction }: TableBlockProps) {
  const [cells, setCells] = useState<string[][]>(() => parseCells(block.content?.trim() || DEFAULT_TABLE))
  const [aligns, setAligns] = useState<ColumnAlign[]>(() => parseAlignment(block.content?.trim() || DEFAULT_TABLE))
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; r: number; c: number } | null>(null)
  const [hoverPlus, setHoverPlus] = useState<{ type: 'row' | 'col'; afterIdx: number; side: 'left' | 'right' | 'top' | 'bottom' } | null>(null)
  const [colWidths, setColWidths] = useState<number[]>(() => {
    try { return JSON.parse(block.properties?.colWidths || '[]') } catch { return [] }
  })
  const [selection, setSelection] = useState<{ r1: number; c1: number; r2: number; c2: number } | null>(null)

  const tableRef = useRef<HTMLTableElement>(null)
  const selectionRef = useRef<typeof selection>(null)
  selectionRef.current = selection
  const clipboardRef = useRef<{ data: string[][]; mode: 'copy' | 'cut'; cutRange?: { r1: number; c1: number; r2: number; c2: number } } | null>(null)
  const selectDragRef = useRef<{ anchor: { r: number; c: number } } | null>(null)
  const outerRef = useRef<HTMLDivElement>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const ctxRef = useRef<HTMLDivElement>(null)
  const lastSyncedMdRef = useRef<string | null>(null)
  const cellsRef = useRef(cells)
  cellsRef.current = cells
  const alignsRef = useRef(aligns)
  alignsRef.current = aligns
  const colWidthsRef = useRef(colWidths)
  colWidthsRef.current = colWidths
  const bordersCacheRef = useRef<{
    hBorders: number[]; vBorders: number[]
    tableTop: number; tableLeft: number; tableWidth: number; tableHeight: number
  } | null>(null)
  const dragRef = useRef<{ col: number; startX: number; widths: number[] } | null>(null)
  const resizeColRef = useRef<number | null>(null)
  const reorderRef = useRef<{ type: 'row' | 'col'; from: number; startYorX: number } | null>(null)
  const [dropLine, setDropLine] = useState<{ type: 'row' | 'col'; pos: number; start: number; length: number } | null>(null)
  // 内容空间的抓手位置（不受滚动影响，offsetTop/offsetLeft）
  const contentPosRef = useRef<{
    rows: { top: number; height: number }[]
    cols: { left: number; width: number }[]
  } | null>(null)
  const [handlePos, setHandlePos] = useState<{
    rows: { top: number; height: number }[]
    cols: { left: number; width: number }[]
    wrapperLeft: number; wrapperTop: number
    tableLeft: number; tableTop: number; tableWidth: number; tableHeight: number
  } | null>(null)
  const onContentChangeRef = useRef(onContentChange)
  onContentChangeRef.current = onContentChange
  const blockIdRef = useRef(block.id)
  blockIdRef.current = block.id

  // ─── 外部 content 同步 ───

  useEffect(() => {
    const md = (block.content?.trim() || DEFAULT_TABLE)
    if (md === lastSyncedMdRef.current) return
    lastSyncedMdRef.current = null
    setCells(parseCells(md))
    setAligns(parseAlignment(md))
  }, [block.content])

  useEffect(() => {
    if (!block.content?.trim()) onContentChange(block.id, DEFAULT_TABLE)
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ─── 右键菜单关闭 ───

  useEffect(() => {
    if (!ctxMenu) return
    const md = (e: MouseEvent) => { if (ctxRef.current?.contains(e.target as Node)) return; setCtxMenu(null) }
    const kd = (e: KeyboardEvent) => { if (e.key === 'Escape') setCtxMenu(null) }
    const id = setTimeout(() => { document.addEventListener('mousedown', md); document.addEventListener('keydown', kd) }, 0)
    return () => { clearTimeout(id); document.removeEventListener('mousedown', md); document.removeEventListener('keydown', kd) }
  }, [ctxMenu])

  // ─── 内部同步 ───

  const sync = useCallback((next: string[][], nextAligns?: ColumnAlign[]) => {
    const md = serializeCells(next, nextAligns ?? alignsRef.current)
    lastSyncedMdRef.current = md
    onContentChangeRef.current(blockIdRef.current, md)
  }, [])

  /** Atomically update cells (+ optional aligns/widths) and sync to parent */
  const commit = useCallback((nextCells: string[][], nextAligns?: ColumnAlign[], nextWidths?: number[]) => {
    cellsRef.current = nextCells
    if (nextAligns !== undefined) {
      alignsRef.current = nextAligns
      setAligns(nextAligns)
    }
    if (nextWidths !== undefined) {
      colWidthsRef.current = nextWidths
      setColWidths(nextWidths)
    }
    setCells(nextCells)
    sync(nextCells, nextAligns)
    if (nextWidths !== undefined) {
      updateBlock(blockIdRef.current, {
        properties: { colWidths: JSON.stringify(nextWidths) },
        properties_mode: 'merge',
      }).catch(() => {})
    }
  }, [sync])

  const handleChange = useCallback((r: number, c: number, value: string, el: HTMLTextAreaElement) => {
    setSelection(null)
    const next = [...cellsRef.current]
    next[r] = [...next[r]]
    while (next[r].length <= c) next[r].push('')
    next[r][c] = value
    commit(next)
    el.style.height = 'auto'
    el.style.height = el.scrollHeight + 'px'
  }, [commit])

  // auto-resize 所有 textarea（内容或列宽变化时）
  useEffect(() => {
    tableRef.current?.querySelectorAll('textarea.wem-tableblock-cell-input').forEach((ta) => {
      ta.style.height = 'auto'
      ta.style.height = ta.scrollHeight + 'px'
    })
  }, [cells, colWidths])

  const handleCellMouseDown = useCallback((e: React.MouseEvent, r: number, c: number) => {
    if (readonly) return
    if (e.button !== 0) return
    // 列宽 resize 区域内不启动拖选，交给 handleTableMouseDown 处理
    if (resizeColRef.current !== null) return

    const anchor = { r, c }
    selectDragRef.current = { anchor }
    setSelection(null)

    const onMove = (ev: MouseEvent) => {
      const el = document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null
      const cellEl = el?.closest('[data-r][data-c]') as HTMLElement | null
      if (!cellEl) return
      const cr = parseInt(cellEl.dataset.r!)
      const cc = parseInt(cellEl.dataset.c!)
      const a = selectDragRef.current?.anchor
      if (!a || (cr === a.r && cc === a.c)) return
      setSelection({
        r1: Math.min(a.r, cr), c1: Math.min(a.c, cc),
        r2: Math.max(a.r, cr), c2: Math.max(a.c, cc),
      })
    }

    const onUp = () => {
      selectDragRef.current = null
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
    }

    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
  }, [readonly])

  // ─── 行列操作 ───

  const addRow = useCallback((afterIndex: number) => {
    if (readonly) return
    setSelection(null)
    const next = cellsRef.current.map(r => [...r])
    const cols = next[0]?.length ?? 2
    next.splice(afterIndex + 1, 0, Array(cols).fill(''))
    commit(next)
  }, [readonly, commit])

  const deleteRow = useCallback((index: number) => {
    if (readonly) return
    setSelection(null)
    const prev = cellsRef.current
    if (prev.length <= 1) return
    const next = prev.map(r => [...r])
    next.splice(index, 1)
    commit(next)
  }, [readonly, commit])

  const addCol = useCallback((afterIndex: number) => {
    if (readonly) return
    setSelection(null)
    const nextCells = cellsRef.current.map(r => { const row = [...r]; row.splice(afterIndex + 1, 0, ''); return row })
    const nextAligns = [...alignsRef.current]
    nextAligns.splice(afterIndex + 1, 0, '')
    commit(nextCells, nextAligns)
  }, [readonly, commit])

  const deleteCol = useCallback((index: number) => {
    if (readonly) return
    setSelection(null)
    const prev = cellsRef.current
    if ((prev[0]?.length ?? 0) <= 1) return
    const nextCells = prev.map(r => { const row = [...r]; row.splice(index, 1); return row })
    const nextAligns = [...alignsRef.current]
    nextAligns.splice(index, 1)
    commit(nextCells, nextAligns)
  }, [readonly, commit])

  const moveRowUp = useCallback((index: number) => {
    if (readonly || index <= 0) return
    const next = cellsRef.current.map(r => [...r])
    ;[next[index - 1], next[index]] = [next[index], next[index - 1]]
    commit(next)
  }, [readonly, commit])

  const moveRowDown = useCallback((index: number) => {
    if (readonly) return
    const prev = cellsRef.current
    if (index >= prev.length - 1) return
    const next = prev.map(r => [...r])
    ;[next[index], next[index + 1]] = [next[index + 1], next[index]]
    commit(next)
  }, [readonly, commit])

  const moveColLeft = useCallback((index: number) => {
    if (readonly || index <= 0) return
    const nextCells = cellsRef.current.map(r => { const row = [...r]; [row[index - 1], row[index]] = [row[index], row[index - 1]]; return row })
    const nextAligns = [...alignsRef.current]
    ;[nextAligns[index - 1], nextAligns[index]] = [nextAligns[index], nextAligns[index - 1]]
    commit(nextCells, nextAligns)
  }, [readonly, commit])

  const moveColRight = useCallback((index: number) => {
    if (readonly) return
    const prevCells = cellsRef.current
    if (index >= (prevCells[0]?.length ?? 0) - 1) return
    const nextCells = prevCells.map(r => { const row = [...r]; [row[index], row[index + 1]] = [row[index + 1], row[index]]; return row })
    const nextAligns = [...alignsRef.current]
    ;[nextAligns[index], nextAligns[index + 1]] = [nextAligns[index + 1], nextAligns[index]]
    commit(nextCells, nextAligns)
  }, [readonly, commit])

  const applyAlign = useCallback((cols: number[], align: ColumnAlign) => {
    const nextAligns = [...alignsRef.current]
    for (const col of cols) {
      while (nextAligns.length <= col) nextAligns.push('')
      nextAligns[col] = align
    }
    commit(cellsRef.current, nextAligns)
  }, [commit])

  const autoFitColumns = useCallback(() => {
    setColWidths([])
    updateBlock(blockIdRef.current, {
      properties: { colWidths: '[]' },
      properties_mode: 'merge',
    }).catch(() => {})
  }, [])

  // ─── 行列拖拽移动（Obsidian 风格抓手） ───

  const moveRowTo = useCallback((from: number, to: number) => {
    if (readonly || from === to) return
    const idx = from < to ? to - 1 : to
    const next = cellsRef.current.map(r => [...r])
    const [moved] = next.splice(from, 1)
    next.splice(idx, 0, moved)
    commit(next)
  }, [readonly, commit])

  const moveColTo = useCallback((from: number, to: number) => {
    if (readonly || from === to) return
    const idx = from < to ? to - 1 : to
    const nextCells = cellsRef.current.map(r => {
      const row = [...r]
      const [moved] = row.splice(from, 1)
      row.splice(idx, 0, moved)
      return row
    })
    const nextAligns = [...alignsRef.current]
    const [a] = nextAligns.splice(from, 1)
    nextAligns.splice(idx, 0, a)
    const widths = colWidthsRef.current
    const nextWidths = widths.length > 0 ? (() => {
      const w = [...widths]
      const [moved] = w.splice(from, 1)
      w.splice(idx, 0, moved)
      return w
    })() : undefined
    commit(nextCells, nextAligns, nextWidths)
  }, [readonly, commit])

  // ─── 键盘导航 ───

  const moveCell = useCallback((r: number, c: number) => {
    setTimeout(() => { (tableRef.current?.querySelector(`[data-r="${r}"][data-c="${c}"]`) as HTMLElement | null)?.focus() }, 0)
  }, [])

  const handleKeyDown = useCallback((e: React.KeyboardEvent, r: number, c: number) => {
    const cells = cellsRef.current
    const rowCount = cells.length
    const colCount = cells[0]?.length ?? 0
    const sel = selectionRef.current

    // ── Alt+Shift+方向键：移动行/列 ──
    if (e.altKey && e.shiftKey) {
      if (e.key === 'ArrowUp') { e.preventDefault(); moveRowUp(r); return }
      if (e.key === 'ArrowDown') { e.preventDefault(); moveRowDown(r); return }
      if (e.key === 'ArrowLeft') { e.preventDefault(); moveColLeft(c); return }
      if (e.key === 'ArrowRight') { e.preventDefault(); moveColRight(c); return }
    }

    // ── Selection key handling ──
    if ((e.ctrlKey || e.metaKey) && e.key === 'c' && sel) {
      e.preventDefault()
      const data = cells.slice(sel.r1, sel.r2 + 1).map(row => row.slice(sel.c1, sel.c2 + 1))
      clipboardRef.current = { data, mode: 'copy' }
      return
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'x' && sel) {
      e.preventDefault()
      const data = cells.slice(sel.r1, sel.r2 + 1).map(row => row.slice(sel.c1, sel.c2 + 1))
      clipboardRef.current = { data, mode: 'cut', cutRange: { r1: sel.r1, c1: sel.c1, r2: sel.r2, c2: sel.c2 } }
      const next = cells.map(row => [...row])
      for (let ri = sel.r1; ri <= sel.r2; ri++) for (let ci = sel.c1; ci <= sel.c2; ci++) next[ri][ci] = ''
      commit(next)
      return
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'v' && clipboardRef.current) {
      e.preventDefault()
      const clip = clipboardRef.current
      const { data } = clip
      const target = sel ?? { r1: r, c1: c, r2: r, c2: c }
      const next = cells.map(row => [...row])
      for (let dr = 0; dr < data.length; dr++) {
        for (let dc = 0; dc < data[dr].length; dc++) {
          const tr = target.r1 + dr, tc = target.c1 + dc
          if (tr < next.length && tc < (next[0]?.length ?? 0)) next[tr][tc] = data[dr][dc]
        }
      }
      if (clip.mode === 'cut' && clip.cutRange) {
        const cr = clip.cutRange
        for (let ri = cr.r1; ri <= cr.r2; ri++) {
          for (let ci = cr.c1; ci <= cr.c2; ci++) {
            if (ri < next.length && ci < (next[0]?.length ?? 0)) {
              const inTarget = ri >= target.r1 && ri <= target.r2 && ci >= target.c1 && ci <= target.c2
              if (!inTarget) next[ri][ci] = ''
            }
          }
        }
      }
      commit(next)
      clipboardRef.current = { ...clip, mode: 'copy' }
      setSelection(null)
      return
    }
    if (e.key === 'Delete' && sel) {
      e.preventDefault()
      const next = cells.map(row => [...row])
      for (let ri = sel.r1; ri <= sel.r2; ri++) for (let ci = sel.c1; ci <= sel.c2; ci++) next[ri][ci] = ''
      commit(next)
      return
    }
    if (e.key === 'Escape') {
      if (sel) { setSelection(null); return }
      ;(e.currentTarget as HTMLElement).blur()
      return
    }

    // ── Normal cell navigation ──
    if (e.key === 'Tab') {
      e.preventDefault()
      let nr = r, nc = c + (e.shiftKey ? -1 : 1)
      if (nc >= colCount) { nc = 0; nr++ }
      if (nc < 0) { nc = colCount - 1; nr-- }
      if (nr >= rowCount) { commit([...cells, Array(colCount || 2).fill('')]) }
      if (nr < 0) return
      moveCell(nr, nc)
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      const nr = r + 1
      if (nr >= rowCount) { commit([...cells, Array(colCount || 2).fill('')]) }
      moveCell(nr, c)
    } else if (e.key === 'ArrowDown') {
      const ta = e.currentTarget as HTMLTextAreaElement
      const onLastLine = !ta.value.substring(ta.selectionStart).includes('\n')
      if (onLastLine && r + 1 < rowCount) { e.preventDefault(); moveCell(r + 1, c) }
    } else if (e.key === 'ArrowUp') {
      const ta = e.currentTarget as HTMLTextAreaElement
      const onFirstLine = !ta.value.substring(0, ta.selectionStart).includes('\n')
      if (onFirstLine && r - 1 >= 0) { e.preventDefault(); moveCell(r - 1, c) }
    } else if (e.key === 'Backspace') {
      if (sel) {
        e.preventDefault()
        const next = cells.map(row => [...row])
        for (let ri = sel.r1; ri <= sel.r2; ri++) for (let ci = sel.c1; ci <= sel.c2; ci++) next[ri][ci] = ''
        commit(next)
        return
      }
      const ta = e.currentTarget as HTMLTextAreaElement
      if (ta.value === '' && c === 0 && rowCount > 1 && ta.selectionStart === 0) {
        e.preventDefault()
        const next = cells.map(row => [...row])
        if (next.length <= 1) return
        next.splice(r, 1)
        commit(next)
        moveCell(Math.min(r, next.length - 1), 0)
      }
    }
  }, [commit, moveCell, moveRowUp, moveRowDown, moveColLeft, moveColRight])

  // ─── 悬浮骑跨线插入指示器 ───

  const getBorderPositions = useCallback(() => {
    const table = tableRef.current
    const outer = outerRef.current
    if (!table || !outer) return null

    const outerRect = outer.getBoundingClientRect()
    const tableRect = table.getBoundingClientRect()

    const tableTop = tableRect.top - outerRect.top
    const tableLeft = tableRect.left - outerRect.left

    const rows = table.querySelectorAll('tr')

    const hBorders: number[] = [tableTop]
    rows.forEach((row) => {
      const rowRect = row.getBoundingClientRect()
      hBorders.push(rowRect.bottom - outerRect.top)
    })

    const vBorders: number[] = [tableLeft]
    const firstRowCells = rows[0]?.querySelectorAll('td, th')
    if (firstRowCells) {
      firstRowCells.forEach((cell) => {
        const cellRect = cell.getBoundingClientRect()
        vBorders.push(cellRect.right - outerRect.left)
      })
    }

    const result = {
      hBorders,
      vBorders,
      tableTop,
      tableLeft,
      tableWidth: tableRect.width,
      tableHeight: tableRect.height,
    }
    bordersCacheRef.current = result
    return result
  }, [])

  // 缓存失效：ResizeObserver 监听表格尺寸变化
  useEffect(() => {
    const table = tableRef.current
    if (!table) return
    const ro = new ResizeObserver(() => { bordersCacheRef.current = null })
    ro.observe(table)
    return () => ro.disconnect()
  }, [])

  // 滚动同步：直接更新抓手 DOM 样式，跳过 React
  useEffect(() => {
    const wrapper = tableRef.current?.parentElement
    if (!wrapper) return
    const onScroll = () => {
      bordersCacheRef.current = null
      syncHandleDOM()
    }
    wrapper.addEventListener('scroll', onScroll, { passive: true })
    return () => wrapper.removeEventListener('scroll', onScroll)
  }, [])

  // 缓存失效：cells 变化时行列数可能改变
  useEffect(() => { bordersCacheRef.current = null }, [cells])

  // ─── 行列拖拽移动（Obsidian 风格抓手） ───

  // 根据 contentPosRef + 当前 scroll 直接更新抓手 DOM
  const syncHandleDOM = useCallback(() => {
    const wrapper = tableRef.current?.parentElement
    const outer = outerRef.current
    const root = rootRef.current
    const cp = contentPosRef.current
    if (!wrapper || !outer || !root || !cp) return
    const sl = wrapper.scrollLeft
    const st = wrapper.scrollTop
    const wLeft = wrapper.offsetLeft
    const wTop = wrapper.offsetTop
    const tLeft = tableRef.current?.offsetLeft ?? 0
    const tTop = tableRef.current?.offsetTop ?? 0
    const outerOff = outer.offsetTop

    root.querySelectorAll<HTMLElement>('[data-handle-row]').forEach(el => {
      const idx = parseInt(el.dataset.handleRow!)
      const pos = cp.rows[idx]
      if (pos) el.style.top = `${outerOff + wTop + tTop + pos.top - st}px`
    })
    outer.querySelectorAll<HTMLElement>('[data-handle-col]').forEach(el => {
      const idx = parseInt(el.dataset.handleCol!)
      const pos = cp.cols[idx]
      if (pos) el.style.left = `${wLeft + tLeft + pos.left - sl}px`
    })
  }, [])

  // 计算抓手位置（仅在数据变化时，不在滚动时）
  useEffect(() => {
    const table = tableRef.current
    const wrapper = table?.parentElement
    if (!table || !wrapper) return

    const rows = table.querySelectorAll('tr')
    const contentRows = Array.from(rows).map(row => ({
      top: (row as HTMLElement).offsetTop,
      height: (row as HTMLElement).offsetHeight,
    }))
    const firstRowCells = rows[0]?.querySelectorAll('td, th')
    const contentCols = firstRowCells
      ? Array.from(firstRowCells).map(cell => ({
          left: (cell as HTMLElement).offsetLeft,
          width: (cell as HTMLElement).offsetWidth,
        }))
      : []

    contentPosRef.current = { rows: contentRows, cols: contentCols }

    const wLeft = wrapper.offsetLeft
    const wTop = wrapper.offsetTop
    const sl = wrapper.scrollLeft
    const st = wrapper.scrollTop
    const tLeft = table.offsetLeft
    const tTop = table.offsetTop

    setHandlePos({
      rows: contentRows.map(p => ({ top: wTop + tTop + p.top - st, height: p.height })),
      cols: contentCols.map(p => ({ left: wLeft + tLeft + p.left - sl, width: p.width })),
      wrapperLeft: wLeft,
      wrapperTop: wTop,
      tableLeft: wLeft + tLeft - sl,
      tableTop: wTop + tTop - st,
      tableWidth: table.offsetWidth,
      tableHeight: table.offsetHeight,
    })
  }, [cells, colWidths])

  const handleReorderMouseDown = useCallback((type: 'row' | 'col', index: number, e: React.MouseEvent) => {
    if (readonly || e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()
    const startCoord = type === 'row' ? e.clientY : e.clientX
    reorderRef.current = { type, from: index, startYorX: startCoord }
    let moved = false
    let lastTargetIdx: number | null = null
    document.body.style.userSelect = 'none'

    const calcTarget = (ev: MouseEvent) => {
      const outer = outerRef.current
      if (!outer) return
      const borders = bordersCacheRef.current ?? getBorderPositions()
      if (!borders) return
      const outerRect = outer.getBoundingClientRect()
      const coord = type === 'row' ? ev.clientY - outerRect.top : ev.clientX - outerRect.left
      const bs = type === 'row' ? borders.hBorders : borders.vBorders
      for (let i = 0; i < bs.length - 1; i++) {
        if (coord < bs[i + 1]) {
          const mid = (bs[i] + bs[i + 1]) / 2
          const targetIdx = coord < mid ? i : i + 1
          if (targetIdx !== index && targetIdx !== index + 1) {
            lastTargetIdx = targetIdx
            setDropLine({
              type,
              pos: coord < mid ? bs[i] : bs[i + 1],
              start: type === 'row' ? borders.tableLeft : borders.tableTop,
              length: type === 'row' ? borders.tableWidth : borders.tableHeight,
            })
          } else {
            lastTargetIdx = null
            setDropLine(null)
          }
          return
        }
      }
      // 超出最后一个边界
      lastTargetIdx = null
      setDropLine(null)
    }

    const onMove = (ev: MouseEvent) => {
      const ref = reorderRef.current
      if (!ref) return
      if (!moved && Math.abs((type === 'row' ? ev.clientY : ev.clientX) - startCoord) < 4) return
      moved = true
      calcTarget(ev)
    }

    const onUp = () => {
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
      document.body.style.userSelect = ''
      reorderRef.current = null
      if (!moved) {
        const count = type === 'row'
          ? (cellsRef.current[0]?.length ?? 0)
          : cellsRef.current.length
        if (type === 'row') {
          setSelection({ r1: index, c1: 0, r2: index, c2: count - 1 })
        } else {
          setSelection({ r1: 0, c1: index, r2: count - 1, c2: index })
        }
      } else if (lastTargetIdx !== null) {
        if (type === 'row') moveRowTo(index, lastTargetIdx)
        else moveColTo(index, lastTargetIdx)
      }
      setDropLine(null)
    }

    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
  }, [readonly, getBorderPositions, moveRowTo, moveColTo])

  // 列宽数组与列数同步（列数变化时重置，内容编辑不影响）
  useEffect(() => {
    const count = cells[0]?.length ?? 0
    setColWidths(prev => {
      if (prev.length === count) return prev
      if (prev.length < count) return [...prev, ...Array(count - prev.length).fill(0)]
      return prev.slice(0, count)
    })
  }, [cells])

  // aligns 数组与列数同步
  useEffect(() => {
    const count = cells[0]?.length ?? 0
    setAligns(prev => {
      if (prev.length === count) return prev
      if (prev.length < count) return [...prev, ...Array(count - prev.length).fill('' as ColumnAlign)]
      return prev.slice(0, count)
    })
  }, [cells])

  const handleMouseMove = useCallback((e: React.MouseEvent) => {
    if (readonly) return
    if ((e.target as HTMLElement).closest('.wem-tableblock-row-handle, .wem-tableblock-col-handle')) return
    if (reorderRef.current) return

    const borders = bordersCacheRef.current ?? getBorderPositions()
    if (!borders) return

    const outer = outerRef.current
    if (!outer) return
    const outerRect = outer.getBoundingClientRect()
    const mouseX = e.clientX - outerRect.left
    const mouseY = e.clientY - outerRect.top

    const { tableTop, tableLeft, tableWidth, tableHeight, vBorders, hBorders } = borders
    const tableRight = tableLeft + tableWidth
    const tableBottom = tableTop + tableHeight

    // T 区域检测：左/右 margin → 行插入三角形
    if (mouseY >= tableTop && mouseY <= tableBottom) {
      if (mouseX < tableLeft && mouseX >= tableLeft - 28) {
        for (let i = 0; i < hBorders.length; i++) {
          if (Math.abs(mouseY - hBorders[i]) < BORDER_CTX_THRESHOLD) {
            setHoverPlus({ type: 'row', afterIdx: i - 1, side: 'left' })
            return
          }
        }
      } else if (mouseX > tableRight && mouseX <= tableRight + 14) {
        for (let i = 0; i < hBorders.length; i++) {
          if (Math.abs(mouseY - hBorders[i]) < BORDER_CTX_THRESHOLD) {
            setHoverPlus({ type: 'row', afterIdx: i - 1, side: 'right' })
            return
          }
        }
      }
    }
    // T 区域检测：上/下 margin → 列插入三角形
    if (mouseX >= tableLeft && mouseX <= tableRight) {
      if (mouseY < tableTop && mouseY >= tableTop - 26) {
        for (let j = 0; j < vBorders.length; j++) {
          if (Math.abs(mouseX - vBorders[j]) < BORDER_CTX_THRESHOLD) {
            setHoverPlus({ type: 'col', afterIdx: j - 1, side: 'top' })
            return
          }
        }
      } else if (mouseY > tableBottom && mouseY <= tableBottom + 14) {
        for (let j = 0; j < vBorders.length; j++) {
          if (Math.abs(mouseX - vBorders[j]) < BORDER_CTX_THRESHOLD) {
            setHoverPlus({ type: 'col', afterIdx: j - 1, side: 'bottom' })
            return
          }
        }
      }
    }
    setHoverPlus(null)

    const insideTable = mouseX >= tableLeft && mouseX <= tableRight && mouseY >= tableTop && mouseY <= tableBottom
    if (insideTable) {
      if (dragRef.current) return
      let resizeCol = -1
      for (let i = 1; i < vBorders.length; i++) {
        if (Math.abs(mouseX - vBorders[i]) < RESIZE_THRESHOLD) { resizeCol = i - 1; break }
      }
      resizeColRef.current = resizeCol >= 0 ? resizeCol : null
      const table = tableRef.current
      if (table) {
        if (resizeCol >= 0) table.classList.add('wem-tableblock-resizing')
        else table.classList.remove('wem-tableblock-resizing')
      }
      return
    }

    if (resizeColRef.current !== null) {
      resizeColRef.current = null
      tableRef.current?.classList.remove('wem-tableblock-resizing')
    }
  }, [readonly, getBorderPositions])

  const handleMouseLeave = useCallback(() => {
    resizeColRef.current = null
    setHoverPlus(null)
    tableRef.current?.classList.remove('wem-tableblock-resizing')
  }, [])

  // ─── 列宽拖拽 ───

  const handleTableMouseDown = useCallback((e: React.MouseEvent) => {
    if (readonly) return
    if (selectDragRef.current) return
    if (dragRef.current) return
    const col = resizeColRef.current
    if (col === null) return

    e.preventDefault()
    e.stopPropagation()

    const table = tableRef.current
    if (!table) return
    const firstRow = table.querySelector('tr')
    if (!firstRow) return
    const firstCells = firstRow.querySelectorAll('td, th')
    const widths = Array.from(firstCells).map(c => (c as HTMLElement).offsetWidth)

    dragRef.current = { col, startX: e.clientX, widths }
    let finalWidths: number[] = widths

    const onMove = (ev: MouseEvent) => {
      const d = dragRef.current
      if (!d) return
      const next = [...d.widths]
      next[d.col] = Math.min(MAX_COL_WIDTH, Math.max(MIN_COL_WIDTH, d.widths[d.col] + ev.clientX - d.startX))
      finalWidths = next
      setColWidths(next)
    }
    const onUp = () => {
      dragRef.current = null
      document.removeEventListener('mousemove', onMove)
      document.removeEventListener('mouseup', onUp)
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
      // 持久化列宽到 properties
      if (finalWidths.length > 0) {
        updateBlock(blockIdRef.current, {
          properties: { colWidths: JSON.stringify(finalWidths) },
          properties_mode: 'merge',
        }).catch(() => {})
      }
    }

    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
    document.addEventListener('mousemove', onMove)
    document.addEventListener('mouseup', onUp)
  }, [readonly])

  // ─── 右键菜单 ───

  const handleTableContextMenu = useCallback((e: React.MouseEvent) => {
    if (readonly) return
    e.stopPropagation(); e.preventDefault()

    const cell = (e.target as HTMLElement).closest('[data-r][data-c]') as HTMLElement | null
    if (!cell) return
    setCtxMenu({
      x: Math.min(e.clientX, window.innerWidth - 200),
      y: Math.min(e.clientY, window.innerHeight - 520),
      r: parseInt(cell.dataset.r!), c: parseInt(cell.dataset.c!),
    })
  }, [readonly])

  const ctxAction = useCallback((fn: () => void) => { fn(); setCtxMenu(null) }, [])

  // ─── Render ───

  const colCount = cells[0]?.length ?? 0
  const isEmpty = cells.length === 1 && cells[0].every((c) => !c.trim())

  const isSelected = (r: number, c: number) => {
    if (!selection) return false
    if (selection.r1 === selection.r2 && selection.c1 === selection.c2) return false
    return r >= selection.r1 && r <= selection.r2 && c >= selection.c1 && c <= selection.c2
  }

  const cellAlignStyle = (c: number) => aligns[c] ? { textAlign: aligns[c] } : undefined

  const sel = selectionRef.current
  const hasMultiSel = !!sel && !(sel.r1 === sel.r2 && sel.c1 === sel.c2)
  const alignTargetCols: number[] = hasMultiSel && sel
    ? Array.from({ length: sel.c2 - sel.c1 + 1 }, (_, i) => sel.c1 + i)
    : ctxMenu ? [ctxMenu.c] : []

  return (
    <div ref={rootRef} className="wem-tableblock">
      {/* ── 行拖拽抓手（左侧编辑器边距区域） ── */}
      {handlePos && !readonly && handlePos.rows.map((pos, i) => (
        <button
          key={`rh${i}`}
          type="button"
          className="wem-tableblock-row-handle"
          style={{ top: pos.top, height: pos.height, left: -26 }}
          data-handle-row={i}
          onMouseDown={(e) => handleReorderMouseDown('row', i, e)}
          title={i === 0 ? '选中/拖拽标题行' : `选中/拖拽第 ${i} 行`}
        >
          <GripVertical size={12} />
        </button>
      ))}

      {/* ── 悬浮检测区域 ── */}
      <div
        ref={outerRef}
        className="wem-tableblock-outer"
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      >
        <div className="wem-tableblock-wrapper">
          <table ref={tableRef} className="wem-tableblock-table" style={colWidths.some(w => w > 0) ? { width: 'max-content' } : undefined} onContextMenu={handleTableContextMenu} onMouseDown={handleTableMouseDown}>
            <colgroup>
              {(cells[0] ?? []).map((_, c) => (
                <col key={c} style={colWidths[c] > 0 ? { width: colWidths[c] } : undefined} />
              ))}
            </colgroup>
            <thead>
              <tr>
                {(cells[0] ?? []).map((cell, c) => (
                  <th key={c} className={`wem-tableblock-header${isSelected(0, c) ? ' wem-tableblock-selected' : ''}`}>
                    <textarea className="wem-tableblock-cell-input" rows={1} data-r={0} data-c={c} value={cell} readOnly={readonly} placeholder={readonly ? '' : 'Header'} onChange={(e) => handleChange(0, c, e.target.value, e.target)} onKeyDown={(e) => handleKeyDown(e, 0, c)} onMouseDown={(e) => handleCellMouseDown(e, 0, c)} style={cellAlignStyle(c)} />
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {cells.slice(1).map((row, r) => (
                <tr key={r}>
                  {row.map((cell, c) => (
                    <td key={c} className={isSelected(r + 1, c) ? 'wem-tableblock-selected' : undefined}>
                      <textarea className="wem-tableblock-cell-input" rows={1} data-r={r + 1} data-c={c} value={cell} readOnly={readonly} placeholder=" " onChange={(e) => handleChange(r + 1, c, e.target.value, e.target)} onKeyDown={(e) => handleKeyDown(e, r + 1, c)} onMouseDown={(e) => handleCellMouseDown(e, r + 1, c)} style={cellAlignStyle(c)} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {/* ── 列拖拽抓手（上方） ── */}
        {handlePos && !readonly && handlePos.cols.map((pos, i) => (
          <button
            key={`ch${i}`}
            type="button"
            className="wem-tableblock-col-handle"
            style={{ left: pos.left, width: pos.width, top: handlePos.wrapperTop - 22 }}
            data-handle-col={i}
            onMouseDown={(e) => handleReorderMouseDown('col', i, e)}
            title={`选中/拖拽第 ${i + 1} 列`}
          >
            <GripHorizontal size={12} />
          </button>
        ))}

        {/* ── 拖拽放置指示线 ── */}
        {dropLine && (
          <div
            className={`wem-tableblock-drop-${dropLine.type}`}
            style={dropLine.type === 'row'
              ? { top: dropLine.pos - 1, left: dropLine.start, width: dropLine.length }
              : { left: dropLine.pos - 1, top: dropLine.start, height: dropLine.length }
            }
          />
        )}

        {/* ── 插入三角形 + 辅助线（T 区域四边） ── */}
        {hoverPlus && !readonly && (() => {
          const borders = bordersCacheRef.current ?? getBorderPositions()
          if (!borders) return null
          const borderPos = hoverPlus.type === 'row'
            ? borders.hBorders[hoverPlus.afterIdx + 1]
            : borders.vBorders[hoverPlus.afterIdx + 1]
          if (borderPos == null) return null

          let triStyle: React.CSSProperties
          let triCls: string
          if (hoverPlus.type === 'row') {
            const top = borderPos - 5
            if (hoverPlus.side === 'left') {
              triStyle = { top, left: borders.tableLeft }
              triCls = 'wem-tableblock-insert-left'
            } else {
              triStyle = { top, left: borders.tableLeft + borders.tableWidth - 7 }
              triCls = 'wem-tableblock-insert-right'
            }
          } else {
            const left = borderPos - 5
            if (hoverPlus.side === 'top') {
              triStyle = { left, top: borders.tableTop }
              triCls = 'wem-tableblock-insert-top'
            } else {
              triStyle = { left, top: borders.tableTop + borders.tableHeight - 7 }
              triCls = 'wem-tableblock-insert-bottom'
            }
          }

          return <>
            {/* 辅助线 */}
            <div
              className={`wem-tableblock-guide-${hoverPlus.type}`}
              style={hoverPlus.type === 'row'
                ? { top: borderPos - 1, left: borders.tableLeft, width: borders.tableWidth }
                : { left: borderPos - 1, top: borders.tableTop, height: borders.tableHeight }
              }
            />
            {/* 三角形按钮 */}
            <button
              type="button"
              className={`wem-tableblock-insert ${triCls}`}
              style={triStyle}
              onClick={() => {
                if (hoverPlus.type === 'row') addRow(hoverPlus.afterIdx)
                else addCol(hoverPlus.afterIdx)
                setHoverPlus(null)
              }}
            />
          </>
        })()}
      </div>

      {/* ── 右键菜单 ── */}
      {ctxMenu && !readonly && (
        <div ref={ctxRef} className="wem-tableblock-ctx" style={{ position: 'fixed', left: ctxMenu.x, top: ctxMenu.y }}>
              {ctxMenu.r > 0 && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => addRow(ctxMenu.r - 1))}><ChevronUp className="h-3.5 w-3.5" /> 在上方插入行</button>}
              <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => addRow(ctxMenu.r))}><ChevronDown className="h-3.5 w-3.5" /> 在下方插入行</button>
              <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => addCol(ctxMenu.c - 1))}><ChevronLeft className="h-3.5 w-3.5" /> 在左侧插入列</button>
              <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => addCol(ctxMenu.c))}><ChevronRight className="h-3.5 w-3.5" /> 在右侧插入列</button>
              <div className="wem-tableblock-ctx-sep" />
              {ctxMenu.r > 0 && <>
                <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => moveRowUp(ctxMenu.r))}><ChevronUp className="h-3.5 w-3.5" /> 上移一行</button>
                {ctxMenu.r < cells.length - 1 && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => moveRowDown(ctxMenu.r))}><ChevronDown className="h-3.5 w-3.5" /> 下移一行</button>}
              </>}
              {ctxMenu.c > 0 && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => moveColLeft(ctxMenu.c))}><ChevronLeft className="h-3.5 w-3.5" /> 左移一列</button>}
              {ctxMenu.c < colCount - 1 && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => moveColRight(ctxMenu.c))}><ChevronRight className="h-3.5 w-3.5" /> 右移一列</button>}
              <div className="wem-tableblock-ctx-sep" />
              <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => applyAlign(alignTargetCols, 'left'))}>左对齐</button>
              <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => applyAlign(alignTargetCols, 'center'))}>居中对齐</button>
              <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => applyAlign(alignTargetCols, 'right'))}>右对齐</button>
              {alignTargetCols.some(c => aligns[c]) && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => applyAlign(alignTargetCols, ''))}>默认对齐</button>}
              <div className="wem-tableblock-ctx-sep" />
              {colWidths.some(w => w > 0) && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => autoFitColumns())}>自动调整列宽</button>}
              {ctxMenu.r > 0 && <button className="wem-tableblock-ctx-item destructive" onClick={() => ctxAction(() => deleteRow(ctxMenu.r))}>删除当前行</button>}
              {colCount > 1 && <button className="wem-tableblock-ctx-item destructive" onClick={() => ctxAction(() => deleteCol(ctxMenu.c))}>删除当前列</button>}
              <div className="wem-tableblock-ctx-sep" />
              <button className="wem-tableblock-ctx-item destructive" onClick={() => ctxAction(() => onAction({ type: 'delete', blockId: block.id }))}>删除表格</button>
        </div>
      )}

      {isEmpty && !readonly && (
        <button className="wem-tableblock-delete" onClick={() => onAction({ type: 'delete', blockId: block.id })}>删除空表格</button>
      )}
    </div>
  )
}
