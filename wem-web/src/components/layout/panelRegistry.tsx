/**
 * panelRegistry — 面板注册表
 *
 * 所有面板的「静态定义」集中在此：类型、图标、标题、渲染组件。
 * 新增面板只需在此文件添加一条注册记录 + 在 PanelType 联合中增加类型。
 */

import type { ComponentType } from 'react'
import type { TocItem } from './TocPanel'
import { Sidebar } from './Sidebar'
import { TocPanel } from './TocPanel'
import { CopilotPanel } from './CopilotPanel'

// ─── 面板类型联合 ───

export type PanelType = 'file-tree' | 'toc' | 'copilot'

// ─── 面板图标组件 ───

function FileTreeIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M4 20h16" />
      <path d="M6 20V6a2 2 0 0 1 2-2h5l5 5v11" />
      <path d="M13 4v5h5" />
      <path d="M8 13h8" />
      <path d="M8 17h5" />
    </svg>
  )
}

function TocIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M8 6h12" />
      <path d="M8 12h12" />
      <path d="M8 18h12" />
      <path d="M4 6h.01" />
      <path d="M4 12h.01" />
      <path d="M4 18h.01" />
    </svg>
  )
}

function CopilotIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 24 24"
      className={className}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M12 2a8 8 0 0 1 8 8c0 3-1.5 5-3.5 6.5L16 21H8l-.5-4.5C5.5 15 4 13 4 10a8 8 0 0 1 8-8z" />
      <path d="M9 21v1" />
      <path d="M15 21v1" />
      <path d="M10 14h4" />
    </svg>
  )
}

// ─── 面板注册类型 ───

/** 渲染面板内容所需的 props */
export interface PanelContentProps {
  tocItems: TocItem[]
  activeDocId: string | null
  setActiveDocId: (id: string | null) => void
  onTocHeadingClick: (blockId: string) => void
}

export interface PanelDefinition {
  /** 面板类型标识 */
  type: PanelType
  /** 显示标题 */
  title: string
  /** 图标组件 */
  icon: ComponentType<{ className?: string }>
  /** 面板内容渲染组件 */
  render: ComponentType<PanelContentProps>
  /** 默认所在槽位 */
  defaultSlot: 'left' | 'right' | 'top'
  /** 默认可见 */
  defaultVisible: boolean
}

// ─── 面板内容组件 ───

/**
 * 面板内容包装器
 *
 * 每个 render 组件从 PanelContentProps 中取自己需要的 props。
 */

export function FileTreeContent(props: PanelContentProps) {
  return <Sidebar activeId={props.activeDocId} onActiveChange={props.setActiveDocId} embedded />
}

export function TocContent(props: PanelContentProps) {
  return <TocPanel items={props.tocItems} onHeadingClick={props.onTocHeadingClick} />
}

/** Copilot 面板：自管理状态，不需要 PanelContentProps */
export function CopilotContent(_props: PanelContentProps) {
  return <CopilotPanel />
}

// ─── 注册表 ───

const registryMap = {
  'file-tree': {
    type: 'file-tree' as const,
    title: '文件',
    icon: FileTreeIcon,
    render: FileTreeContent,
    defaultSlot: 'left' as const,
    defaultVisible: true,
  },
  toc: {
    type: 'toc' as const,
    title: '目录',
    icon: TocIcon,
    render: TocContent,
    defaultSlot: 'right' as const,
    defaultVisible: true,
  },
  copilot: {
    type: 'copilot' as const,
    title: 'Copilot',
    icon: CopilotIcon,
    render: CopilotContent,
    defaultSlot: 'right' as const,
    defaultVisible: false,
  },
} satisfies Record<PanelType, PanelDefinition>

export const PANEL_REGISTRY = registryMap

// ─── 工具函数 ───

/** 获取面板定义，不存在时返回 undefined */
export function getPanelDefinition(type: string): PanelDefinition | undefined {
  return registryMap[type as PanelType]
}

/** 获取所有已注册的面板 ID */
export function getRegisteredPanelTypes(): PanelType[] {
  return Object.keys(registryMap) as PanelType[]
}

/** 获取面板图标组件 */
export function getPanelIcon(type: string): ComponentType<{ className?: string }> | null {
  return registryMap[type as PanelType]?.icon ?? null
}

// ─── 面板内容渲染器 ───

interface PanelContentRendererProps extends PanelContentProps {
  panel: { type: string }
}

/**
 * 面板内容统一渲染入口
 *
 * 根据 panel.type 从注册表中查找对应的 render 组件并调用。
 * App.tsx 中三处 slot 渲染都使用此组件，新增面板时无需修改 App.tsx。
 */
export function PanelContentRenderer({ panel, ...props }: PanelContentRendererProps) {
  const def = getPanelDefinition(panel.type)
  if (!def) {
    return (
      <div className="p-4 text-sm text-muted-foreground">
        未知面板类型：{panel.type}
      </div>
    )
  }
  const Render = def.render
  return <Render {...props} />
}
