/**
 * TableBlock — 飞书风格 markdown 表格
 *
 * 交互：
 * - 外围区域悬浮 → 出现 "+" 按钮和辅助插入线
 *   行插入：鼠标在表格外左/右区域 → 横线 + 左侧 "+" 按钮
 *   列插入：鼠标在表格外上/下区域 → 竖线 + 上方 "+" 按钮
 * - 列宽拖拽：鼠标靠近表格内部垂直分隔线 → col-resize → 拖拽调整
 * - 右键菜单：指定位置插入行/列、删除行/列/表格、行列移动、列对齐
 * - Tab / Enter 键盘导航
 * - Shift+Enter 单元格内软换行（markdown <br>）
 * - 多单元格拖选 + 内部 Ctrl+C/X/V
 * - Alt+Shift+方向键 移动行/列
 */

import { useRef, useCallback, useEffect, useState } from 'react'
import type { BlockNode } from '@/types/api'
import type { BlockAction } from '../core/types'
import { updateBlock } from '@/api/client'
import { Plus, ChevronUp, ChevronDown, ChevronLeft, ChevronRight } from 'lucide-react'

// ─── Props ───

interface TableBlockProps {
  block: BlockNode
  readonly: boolean
  onContentChange: (blockId: string, content: string) => void
  onAction: (action: BlockAction) => void
}

// ─── 悬浮指示器状态 ───

interface HoverIndicator {
  type: 'row' | 'col'
  /** 在第 insertAfter 行/列之后插入；-1 表示插入到第一行/列之前 */
  insertAfter: number
  /** 辅助线的像素位置（行→Y，列→X），相对于 outer 容器 */
  pos: number
  /** 辅助线起点像素 */
  lineStart: number
  /** 辅助线长度像素 */
  lineLength: number
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

// ─── 组件 ───

export function TableBlock({ block, readonly, onContentChange, onAction }: TableBlockProps) {
  const [cells, setCells] = useState<string[][]>(() => parseCells(block.content?.trim() || DEFAULT_TABLE))
  const [aligns, setAligns] = useState<ColumnAlign[]>(() => parseAlignment(block.content?.trim() || DEFAULT_TABLE))
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; r: number; c: number } | null>(null)
  const [hoverIndicator, setHoverIndicator] = useState<HoverIndicator | null>(null)
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
  const ctxRef = useRef<HTMLDivElement>(null)
  const skipParseRef = useRef(false)
  const prevIndicatorRef = useRef<HoverIndicator | null>(null)
  const cellsRef = useRef(cells)
  cellsRef.current = cells
  const alignsRef = useRef(aligns)
  alignsRef.current = aligns
  const bordersCacheRef = useRef<{
    hBorders: number[]; vBorders: number[]
    tableTop: number; tableLeft: number; tableWidth: number; tableHeight: number
  } | null>(null)
  const dragRef = useRef<{ col: number; startX: number; widths: number[] } | null>(null)
  const resizeColRef = useRef<number | null>(null)
  const onContentChangeRef = useRef(onContentChange)
  onContentChangeRef.current = onContentChange
  const blockIdRef = useRef(block.id)
  blockIdRef.current = block.id

  // ─── 外部 content 同步 ───

  useEffect(() => {
    if (skipParseRef.current) { skipParseRef.current = false; return }
    const md = block.content?.trim() || DEFAULT_TABLE
    setCells(parseCells(md))
    setAligns(parseAlignment(md))
  }, [block.content])

  useEffect(() => {
    if (!block.content?.trim()) { skipParseRef.current = true; onContentChange(block.id, DEFAULT_TABLE) }
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
    skipParseRef.current = true
    onContentChangeRef.current(blockIdRef.current, serializeCells(next, nextAligns ?? alignsRef.current))
  }, [])

  const handleChange = useCallback((r: number, c: number, value: string, el: HTMLTextAreaElement) => {
    setSelection(null)
    setCells((prev) => {
      const next = [...prev]
      next[r] = [...prev[r]]
      while (next[r].length <= c) next[r].push('')
      next[r][c] = value
      sync(next)
      return next
    })
    // textarea auto-resize
    el.style.height = 'auto'
    el.style.height = el.scrollHeight + 'px'
  }, [sync])

  // 初始化时 auto-resize 所有 textarea
  useEffect(() => {
    tableRef.current?.querySelectorAll('textarea.wem-tableblock-cell-input').forEach((ta) => {
      ta.style.height = 'auto'
      ta.style.height = ta.scrollHeight + 'px'
    })
  }, [cells])

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
    setCells((prev) => { const next = prev.map((r) => [...r]); const cols = next[0]?.length ?? 2; next.splice(afterIndex + 1, 0, Array(cols).fill('')); sync(next); return next })
  }, [readonly, sync])

  const deleteRow = useCallback((index: number) => {
    if (readonly) return
    setSelection(null)
    setCells((prev) => { if (prev.length <= 1) return prev; const next = prev.map((r) => [...r]); next.splice(index, 1); sync(next); return next })
  }, [readonly, sync])

  const addCol = useCallback((afterIndex: number) => {
    if (readonly) return
    setSelection(null)
    setCells((prev) => {
      const next = prev.map((r) => { const row = [...r]; row.splice(afterIndex + 1, 0, ''); return row })
      setAligns((pa) => { const a = [...pa]; a.splice(afterIndex + 1, 0, ''); return a })
      sync(next)
      return next
    })
  }, [readonly, sync])

  const deleteCol = useCallback((index: number) => {
    if (readonly) return
    setSelection(null)
    setCells((prev) => {
      if ((prev[0]?.length ?? 0) <= 1) return prev
      const next = prev.map((r) => { const row = [...r]; row.splice(index, 1); return row })
      setAligns((pa) => { const a = [...pa]; a.splice(index, 1); return a })
      sync(next)
      return next
    })
  }, [readonly, sync])

  const moveRowUp = useCallback((index: number) => {
    if (readonly || index <= 0) return
    setCells((prev) => {
      const next = prev.map((r) => [...r])
      ;[next[index - 1], next[index]] = [next[index], next[index - 1]]
      sync(next)
      return next
    })
  }, [readonly, sync])

  const moveRowDown = useCallback((index: number) => {
    if (readonly) return
    setCells((prev) => {
      if (index >= prev.length - 1) return prev
      const next = prev.map((r) => [...r])
      ;[next[index], next[index + 1]] = [next[index + 1], next[index]]
      sync(next)
      return next
    })
  }, [readonly, sync])

  const moveColLeft = useCallback((index: number) => {
    if (readonly || index <= 0) return
    setCells((prev) => {
      const next = prev.map((r) => { const row = [...r]; [row[index - 1], row[index]] = [row[index], row[index - 1]]; return row })
      setAligns((pa) => { const a = [...pa]; [a[index - 1], a[index]] = [a[index], a[index - 1]]; return a })
      sync(next)
      return next
    })
  }, [readonly, sync])

  const moveColRight = useCallback((index: number) => {
    if (readonly) return
    setCells((prev) => {
      const cols = prev[0]?.length ?? 0
      if (index >= cols - 1) return prev
      const next = prev.map((r) => { const row = [...r]; [row[index], row[index + 1]] = [row[index + 1], row[index]]; return row })
      setAligns((pa) => { const a = [...pa]; [a[index], a[index + 1]] = [a[index + 1], a[index]]; return a })
      sync(next)
      return next
    })
  }, [readonly, sync])

  const setColumnAlign = useCallback((col: number, align: ColumnAlign) => {
    setAligns((prev) => {
      const next = [...prev]
      while (next.length <= col) next.push('')
      next[col] = align
      sync(cellsRef.current, next)
      return next
    })
  }, [sync])

  const autoFitColumns = useCallback(() => {
    setColWidths([])
    updateBlock(blockIdRef.current, {
      properties: { colWidths: '[]' },
      properties_mode: 'merge',
    }).catch(() => {})
  }, [])

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
      setCells((prev) => {
        const next = prev.map(row => [...row])
        for (let ri = sel.r1; ri <= sel.r2; ri++) for (let ci = sel.c1; ci <= sel.c2; ci++) next[ri][ci] = ''
        sync(next)
        return next
      })
      return
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'v' && clipboardRef.current) {
      e.preventDefault()
      const clip = clipboardRef.current
      const { data } = clip
      const target = sel ?? { r1: r, c1: c, r2: r, c2: c }
      setCells((prev) => {
        const next = prev.map(row => [...row])
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
        sync(next)
        return next
      })
      clipboardRef.current = { ...clip, mode: 'copy' }
      setSelection(null)
      return
    }
    if (e.key === 'Delete' && sel) {
      e.preventDefault()
      setCells((prev) => {
        const next = prev.map(row => [...row])
        for (let ri = sel.r1; ri <= sel.r2; ri++) for (let ci = sel.c1; ci <= sel.c2; ci++) next[ri][ci] = ''
        sync(next)
        return next
      })
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
      if (nr >= rowCount) { setCells((prev) => { const cols = prev[0]?.length ?? 2; const next = [...prev, Array(cols).fill('')]; sync(next); return next }); moveCell(nr, 0); return }
      if (nr < 0) return
      moveCell(nr, nc)
    } else if (e.key === 'Enter' && !e.shiftKey) {
      // Enter → 跳到下一行（Shift+Enter 由 textarea 默认处理，插入软换行）
      e.preventDefault()
      const nr = r + 1
      if (nr >= rowCount) { setCells((prev) => { const cols = prev[0]?.length ?? 2; const next = [...prev, Array(cols).fill('')]; sync(next); return next }) }
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
        setCells((prev) => {
          const next = prev.map(row => [...row])
          for (let ri = sel.r1; ri <= sel.r2; ri++) for (let ci = sel.c1; ci <= sel.c2; ci++) next[ri][ci] = ''
          sync(next)
          return next
        })
        return
      }
      const ta = e.currentTarget as HTMLTextAreaElement
      if (ta.value === '' && c === 0 && rowCount > 1 && ta.selectionStart === 0) {
        e.preventDefault()
        setCells((prev) => { if (prev.length <= 1) return prev; const next = prev.map((row) => [...row]); next.splice(r, 1); sync(next); moveCell(Math.min(r, next.length - 1), 0); return next })
      }
    }
  }, [sync, moveCell, moveRowUp, moveRowDown, moveColLeft, moveColRight])

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

  // 缓存失效：cells 变化时行列数可能改变
  useEffect(() => { bordersCacheRef.current = null }, [cells])

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
    if (readonly) {
      if (prevIndicatorRef.current) { prevIndicatorRef.current = null; setHoverIndicator(null) }
      return
    }

    // 鼠标在 "+" 按钮上时保持 indicator 不变
    if ((e.target as HTMLElement).closest('.wem-tableblock-plus')) return

    const borders = bordersCacheRef.current ?? getBorderPositions()
    if (!borders) return

    const outer = outerRef.current
    if (!outer) return
    const outerRect = outer.getBoundingClientRect()
    const mouseX = e.clientX - outerRect.left
    const mouseY = e.clientY - outerRect.top

    const { tableTop, tableLeft, tableWidth, tableHeight, hBorders, vBorders } = borders
    const tableRight = tableLeft + tableWidth
    const tableBottom = tableTop + tableHeight

    // 鼠标在表格内部 → 检测列宽 resize handle
    const insideTable = mouseX >= tableLeft && mouseX <= tableRight && mouseY >= tableTop && mouseY <= tableBottom
    if (insideTable) {
      if (prevIndicatorRef.current) { prevIndicatorRef.current = null; setHoverIndicator(null) }
      if (dragRef.current) return

      // 检测是否靠近垂直分隔线 → 显示 col-resize 光标
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

    // 鼠标离开表格区域 → 重置 resize 光标
    if (resizeColRef.current !== null) {
      resizeColRef.current = null
      tableRef.current?.classList.remove('wem-tableblock-resizing')
    }

    let newIndicator: HoverIndicator | null = null

    // 左/右外围区域 → 行插入（根据 mouseY 找最近的行边界）
    if ((mouseX < tableLeft || mouseX > tableRight) && mouseY >= tableTop && mouseY <= tableBottom) {
      let nearestIdx = -1
      let nearestDist = Infinity
      hBorders.forEach((pos, i) => {
        const d = Math.abs(mouseY - pos)
        if (d < nearestDist) { nearestIdx = i; nearestDist = d }
      })
      if (nearestIdx >= 0) {
        newIndicator = { type: 'row', insertAfter: nearestIdx - 1, pos: hBorders[nearestIdx], lineStart: tableLeft, lineLength: tableWidth }
      }
    }
    // 上/下外围区域 → 列插入（根据 mouseX 找最近的列边界）
    else if ((mouseY < tableTop || mouseY > tableBottom) && mouseX >= tableLeft && mouseX <= tableRight) {
      let nearestIdx = -1
      let nearestDist = Infinity
      vBorders.forEach((pos, i) => {
        const d = Math.abs(mouseX - pos)
        if (d < nearestDist) { nearestIdx = i; nearestDist = d }
      })
      if (nearestIdx >= 0) {
        newIndicator = { type: 'col', insertAfter: nearestIdx - 1, pos: vBorders[nearestIdx], lineStart: tableTop, lineLength: tableHeight }
      }
    }

    // indicator 没变时跳过 setState
    const prev = prevIndicatorRef.current
    const same = newIndicator && prev
      ? newIndicator.type === prev.type && newIndicator.insertAfter === prev.insertAfter
      : newIndicator === prev
    if (same) return

    prevIndicatorRef.current = newIndicator
    setHoverIndicator(newIndicator)
  }, [readonly, getBorderPositions])

  const handleMouseLeave = useCallback(() => {
    prevIndicatorRef.current = null
    setHoverIndicator(null)
    resizeColRef.current = null
    tableRef.current?.classList.remove('wem-tableblock-resizing')
  }, [])

  /** 点击 "+" 按钮执行插入 */
  const handleIndicatorClick = useCallback(() => {
    const indicator = prevIndicatorRef.current
    if (!indicator) return
    if (indicator.type === 'row') addRow(indicator.insertAfter)
    else addCol(indicator.insertAfter)
    prevIndicatorRef.current = null
    setHoverIndicator(null)
  }, [addRow, addCol])

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
      next[d.col] = Math.max(MIN_COL_WIDTH, d.widths[d.col] + ev.clientX - d.startX)
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
    setCtxMenu({ x: Math.min(e.clientX, window.innerWidth - 200), y: Math.min(e.clientY, window.innerHeight - 520), r: parseInt(cell.dataset.r!), c: parseInt(cell.dataset.c!) })
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

  return (
    <div className="wem-tableblock">
      {/* ── 悬浮检测区域 ── */}
      <div
        ref={outerRef}
        className="wem-tableblock-outer"
        onMouseMove={handleMouseMove}
        onMouseLeave={handleMouseLeave}
      >
        <div className="wem-tableblock-wrapper">
          <table ref={tableRef} className="wem-tableblock-table" onContextMenu={handleTableContextMenu} onMouseDown={handleTableMouseDown}>
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

        {/* ── 行插入指示器（水平辅助线 + 左侧 "+"） ── */}
        {hoverIndicator?.type === 'row' && !readonly && (
          <>
            <div
              className="wem-tableblock-hline"
              style={{
                left: hoverIndicator.lineStart,
                top: hoverIndicator.pos - 0.5,
                width: hoverIndicator.lineLength,
              }}
            />
            <button
              type="button"
              className="wem-tableblock-plus"
              style={{
                left: hoverIndicator.lineStart - 22,
                top: hoverIndicator.pos - 9,
              }}
              onClick={handleIndicatorClick}
              title={hoverIndicator.insertAfter < 0 ? '在上方插入行' : '在下方插入行'}
            >
              <Plus className="h-3 w-3" />
            </button>
          </>
        )}

        {/* ── 列插入指示器（垂直辅助线 + 上方 "+"） ── */}
        {hoverIndicator?.type === 'col' && !readonly && (
          <>
            <div
              className="wem-tableblock-vline"
              style={{
                left: hoverIndicator.pos - 0.5,
                top: hoverIndicator.lineStart,
                height: hoverIndicator.lineLength,
              }}
            />
            <button
              type="button"
              className="wem-tableblock-plus"
              style={{
                left: hoverIndicator.pos - 9,
                top: hoverIndicator.lineStart - 22,
              }}
              onClick={handleIndicatorClick}
              title={hoverIndicator.insertAfter < 0 ? '在左侧插入列' : '在右侧插入列'}
            >
              <Plus className="h-3 w-3" />
            </button>
          </>
        )}
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
          <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => setColumnAlign(ctxMenu.c, 'left'))}>左对齐</button>
          <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => setColumnAlign(ctxMenu.c, 'center'))}>居中对齐</button>
          <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => setColumnAlign(ctxMenu.c, 'right'))}>右对齐</button>
          {aligns[ctxMenu.c] && <button className="wem-tableblock-ctx-item" onClick={() => ctxAction(() => setColumnAlign(ctxMenu.c, ''))}>默认对齐</button>}
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
