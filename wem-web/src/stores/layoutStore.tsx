/**
 * layoutStore — 面板槽位布局管理
 *
 * 类似 VS Code 的面板布局系统：
 * - 三个槽位：left / right / top
 * - 面板可在槽位间拖拽移动
 * - 布局持久化到 localStorage（带版本号，支持迁移）
 *
 * 面板静态定义（图标、标题、渲染）由 panelRegistry 管理，
 * 此 store 只管布局状态（哪个面板在哪个槽位、是否可见、尺寸）。
 */

import { createContext, useContext, useState, useCallback, useEffect, useMemo, type ReactNode } from 'react'
import { type PanelType, PANEL_REGISTRY, getRegisteredPanelTypes } from '@/components/layout/panelRegistry'

// ─── Types ───

/** 槽位位置 */
export type SlotPosition = 'left' | 'right' | 'top'

/** 面板布局配置（只包含运行时布局状态，不含静态元信息） */
export interface PanelConfig {
  /** 面板唯一 ID，与 PanelType 一致 */
  id: string
  /** 面板类型标识 */
  type: PanelType
  /** 当前所在槽位 */
  slot: SlotPosition
  /** 槽位内排序权重（越小越靠前） */
  order: number
  /** 面板是否可见 */
  visible: boolean
}

/** 槽位尺寸状态 */
export interface SlotState {
  /** 槽位宽度（left/right）或高度（top） */
  size: number
}

// ─── 持久化 schema ───

const CURRENT_LAYOUT_VERSION = 2

interface PersistedLayoutState {
  version: number
  panels: PanelConfig[]
  slots: Record<SlotPosition, SlotState>
}

/** 布局状态 */
interface LayoutState {
  panels: PanelConfig[]
  slots: Record<SlotPosition, SlotState>
}

interface LayoutActions {
  /** 移动面板到指定槽位 */
  movePanel: (panelId: string, targetSlot: SlotPosition, targetOrder?: number) => void
  /** 切换面板可见性 */
  togglePanel: (panelId: string) => void
  /** 切换面板可见性，同时确保面板属于指定槽位（点击 ActivityBar 图标时使用） */
  togglePanelToSlot: (panelId: string, slot: SlotPosition) => void
  /** 设置槽位尺寸 */
  setSlotSize: (slot: SlotPosition, size: number) => void
  /** 获取指定槽位的面板（按 order 排序，仅 visible） */
  getSlotPanels: (slot: SlotPosition) => PanelConfig[]
  /** 获取指定槽位的所有面板（含 hidden，按 order 排序） */
  getAllSlotPanels: (slot: SlotPosition) => PanelConfig[]
  /** 重置布局为默认 */
  resetLayout: () => void
}

type LayoutStore = LayoutState & LayoutActions

const LayoutContext = createContext<LayoutStore | null>(null)

// ─── 默认面板配置 ───

function buildDefaultPanels(): PanelConfig[] {
  return getRegisteredPanelTypes().map((type, index) => {
    const def = PANEL_REGISTRY[type]
    return {
      id: type,
      type,
      slot: def.defaultSlot,
      order: index,
      visible: def.defaultVisible,
    }
  })
}

const DEFAULT_SLOT_SIZES: Record<SlotPosition, number> = {
  left: 260,
  right: 224,
  top: 200,
}

function buildDefaultSlots(): Record<SlotPosition, SlotState> {
  return {
    left: { size: DEFAULT_SLOT_SIZES.left },
    right: { size: DEFAULT_SLOT_SIZES.right },
    top: { size: DEFAULT_SLOT_SIZES.top },
  }
}

function getDefaultState(): LayoutState {
  return {
    panels: buildDefaultPanels(),
    slots: buildDefaultSlots(),
  }
}

// ─── 持久化与迁移 ───

const STORAGE_KEY = 'wem-layout'

/** 迁移旧版本布局数据 */
function migrateLayoutState(raw: unknown): LayoutState | null {
  if (!raw || typeof raw !== 'object') return null

  const data = raw as Record<string, unknown>

  // v0（无版本号）：旧格式可能有 icon / title 字段，需要清理
  if (!('version' in data)) {
    return migrateV0(data)
  }

  // 当前版本直接使用
  if (data.version === CURRENT_LAYOUT_VERSION) {
    const state = data as unknown as PersistedLayoutState
    if (!Array.isArray(state.panels) || typeof state.slots !== 'object') return null
    return { panels: state.panels, slots: state.slots as Record<SlotPosition, SlotState> }
  }

  // 旧版本（v0 / v1 / …）统一走迁移，补全新增面板
  return migrateV0(data)
}

/** 迁移 v0（无版本号）数据：清理旧字段，确保所有已注册面板存在 */
function migrateV0(data: Record<string, unknown>): LayoutState {
  const panels = (data.panels as Array<Record<string, unknown>> | undefined) ?? []

  const migratedPanels: PanelConfig[] = panels
    .map((p): PanelConfig | null => {
      const type = p.type as string
      // 过滤掉未注册的面板类型
      if (!PANEL_REGISTRY[type as PanelType]) return null
      return {
        id: (p.id as string) || type,
        type: type as PanelType,
        slot: (p.slot as SlotPosition) ?? 'left',
        order: (p.order as number) ?? 0,
        visible: (p.visible as boolean) ?? true,
        // 旧 icon / title 字段直接丢弃
      }
    })
    .filter((p): p is PanelConfig => p !== null)

  // 确保所有已注册面板都存在
  const registeredTypes = getRegisteredPanelTypes()
  const existingIds = new Set(migratedPanels.map((p) => p.id))
  let nextOrder = migratedPanels.length > 0 ? Math.max(...migratedPanels.map((p) => p.order)) + 1 : 0

  for (const type of registeredTypes) {
    if (!existingIds.has(type)) {
      const def = PANEL_REGISTRY[type]
      migratedPanels.push({
        id: type,
        type,
        slot: def.defaultSlot,
        order: nextOrder++,
        visible: def.defaultVisible,
      })
    }
  }

  const slots = (data.slots as Record<SlotPosition, SlotState> | undefined) ?? buildDefaultSlots()

  return { panels: migratedPanels, slots }
}

function loadState(): LayoutState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return null
    const parsed = JSON.parse(raw)
    return migrateLayoutState(parsed)
  } catch {
    return null
  }
}

function saveState(state: LayoutState) {
  try {
    const persisted: PersistedLayoutState = { version: CURRENT_LAYOUT_VERSION, ...state }
    localStorage.setItem(STORAGE_KEY, JSON.stringify(persisted))
  } catch {
    // localStorage 不可用时静默失败
  }
}

// ─── Provider ───

export function LayoutProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<LayoutState>(() => loadState() || getDefaultState())

  // 持久化
  useEffect(() => {
    saveState(state)
  }, [state])

  const movePanel = useCallback((panelId: string, targetSlot: SlotPosition, targetOrder?: number) => {
    setState((prev) => {
      const panels = prev.panels.map((p) => {
        if (p.id !== panelId) return p
        const slotPanels = prev.panels.filter((sp) => sp.slot === targetSlot && sp.id !== panelId)
        const maxOrder = slotPanels.length > 0 ? Math.max(...slotPanels.map((sp) => sp.order)) : -1
        return { ...p, slot: targetSlot, order: targetOrder ?? maxOrder + 1, visible: true }
      })
      return { ...prev, panels }
    })
  }, [])

  const togglePanel = useCallback((panelId: string) => {
    setState((prev) => ({
      ...prev,
      panels: prev.panels.map((p) =>
        p.id === panelId ? { ...p, visible: !p.visible } : p,
      ),
    }))
  }, [])

  const togglePanelToSlot = useCallback((panelId: string, slot: SlotPosition) => {
    setState((prev) => {
      const target = prev.panels.find((p) => p.id === panelId)
      if (!target) return prev
      if (target.slot === slot) {
        return {
          ...prev,
          panels: prev.panels.map((p) =>
            p.id === panelId ? { ...p, visible: !p.visible } : p,
          ),
        }
      }
      const slotPanels = prev.panels.filter((sp) => sp.slot === slot && sp.id !== panelId)
      const maxOrder = slotPanels.length > 0 ? Math.max(...slotPanels.map((sp) => sp.order)) : -1
      return {
        ...prev,
        panels: prev.panels.map((p) =>
          p.id === panelId ? { ...p, slot, order: maxOrder + 1, visible: true } : p,
        ),
      }
    })
  }, [])

  const setSlotSize = useCallback((slot: SlotPosition, size: number) => {
    setState((prev) => ({
      ...prev,
      slots: { ...prev.slots, [slot]: { ...prev.slots[slot], size } },
    }))
  }, [])

  const getSlotPanels = useCallback((slot: SlotPosition): PanelConfig[] => {
    return state.panels.filter((p) => p.slot === slot && p.visible).sort((a, b) => a.order - b.order)
  }, [state.panels])

  const getAllSlotPanels = useCallback((slot: SlotPosition): PanelConfig[] => {
    return state.panels.filter((p) => p.slot === slot).sort((a, b) => a.order - b.order)
  }, [state.panels])

  const resetLayout = useCallback(() => {
    setState(getDefaultState())
  }, [])

  const store: LayoutStore = useMemo(() => ({
    ...state,
    movePanel,
    togglePanel,
    togglePanelToSlot,
    setSlotSize,
    getSlotPanels,
    getAllSlotPanels,
    resetLayout,
  }), [state, movePanel, togglePanel, togglePanelToSlot, setSlotSize, getSlotPanels, getAllSlotPanels, resetLayout])

  return (
    <LayoutContext.Provider value={store}>
      {children}
    </LayoutContext.Provider>
  )
}

export function useLayoutStore(): LayoutStore {
  const store = useContext(LayoutContext)
  if (!store) throw new Error('useLayoutStore must be used within LayoutProvider')
  return store
}
