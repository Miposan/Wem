# Wem 布局系统重构与后续设计报告

日期：2026-04-26

## 1. 背景与目标

本次处理的问题集中在 `wem-web` 的 VS Code 风格侧边布局系统，核心现象是：

1. 左右 ActivityBar 的功能图标长期无法稳定显示。
2. 拖拽移动面板已经可用，但交互反馈仍不完整。
3. 原实现把图标渲染、Tooltip、按钮语义、拖拽源、drop 目标混在一起，导致问题定位困难。
4. 布局系统未来还需要继续扩展更多面板，因此需要重新明确设计边界。

本次重构目标不是继续打补丁，而是将当前不稳定的部分改成更简单、更可控、更容易继续美化的结构。

## 2. 当前问题分析

### 2.1 图标不显示的根因

之前 ActivityBar 图标经过多次尝试仍然不显示，主要暴露出三个设计问题：

#### 2.1.1 TooltipTrigger 被过度用作按钮抽象

`@base-ui/react` 的 `TooltipTrigger` 内部通过 `useRenderElement` 合并 props，并在 `render` 模式下使用 `React.cloneElement` 或 render function。它适合作为可访问性增强层，但不适合作为布局系统中最核心的按钮基础组件。

ActivityBar 图标按钮承担了以下职责：

- 点击切换面板
- 拖拽源
- 显示 active 状态
- 显示 selected indicator
- 接收焦点样式
- 承载 tooltip
- 作为视觉系统的一部分

这些职责全部塞进 TooltipTrigger 后，一旦 TooltipTrigger 的 children/props 合并行为发生变化，就会直接影响图标本体是否渲染。

#### 2.1.2 图标字段使用字符串映射组件，可靠性不足

旧状态中 `PanelConfig.icon` 存储的是 lucide 图标名，例如：

- `Files`
- `List`

这类设计存在几个问题：

1. 字符串和真实组件之间没有强类型绑定。
2. localStorage 持久化后，旧 icon 值会长期存在。
3. 动态组件查找容易被 React 编译器、Tree Shaking、导入路径或版本差异影响。
4. 面板类型本身已经足以决定图标，额外维护 icon 字段是重复状态。

因此，后续应优先使用 `panel.type` 决定图标，而不是使用可变字符串 icon 名称。

#### 2.1.3 过早依赖第三方图标组件增加不确定性

`lucide-react` 本身没有问题，但在当前布局系统还未稳定前，ActivityBar 的图标应该先保证“确定能显示”。

本次重构将 ActivityBar 图标改为本地内置 SVG：

- 不依赖 lucide 动态组件映射
- 不依赖 TooltipTrigger children 合并
- 不受 localStorage 旧 icon 字符串影响
- 视觉尺寸、stroke、颜色完全由 ActivityBar 控制

这是为了先让基础布局系统稳定，而不是继续在多个抽象层之间排查问题。

## 3. 本次重构内容

### 3.1 ActivityBar 图标渲染重构

文件：`wem-web/src/components/layout/ActivityBar.tsx`

本次改动：

1. 移除 ActivityBar 对 `lucide-react` 的依赖。
2. 移除 ActivityBar 对 `TooltipTrigger` 的依赖。
3. 新增本地 SVG 图标组件：
   - `FileTreeIcon`
   - `TocIcon`
4. 新增 `PanelIcon`，通过 `panel.type` 渲染图标。
5. ActivityBar 图标按钮使用原生 `<button>`，由组件自身掌控 children、aria、drag、click 和样式。

重构后的原则：

- `panel.type` 是图标选择的唯一依据。
- ActivityBar 不读取 `panel.icon` 作为渲染条件。
- Tooltip 不再影响图标是否显示。
- 图标 SVG 是按钮 DOM 的直接子元素，渲染路径可预期。

### 3.2 拖拽交互重构

ActivityBar 继续作为 drag source 和 drop target。

已优化：

1. 拖拽开始时写入：
   - `application/wem-panel`
   - `application/wem-panel-source`
2. Drop 时判断来源，避免同侧 drop 导致无意义移动。
3. 使用 React state 管理 `draggingPanelId`，避免直接操作 DOM class。
4. 拖拽中图标使用 `opacity-45 scale-95` 表示正在移动。
5. 空 ActivityBar 显示虚线占位块，提示该区域仍可 drop。
6. Drop overlay 保留为 ActivityBar 层级反馈，而不是 SlotContainer 反馈。

### 3.3 SlotContainer 职责简化

当前 SlotContainer 的方向是正确的：

- 只负责槽位内容渲染
- 只负责 resize
- 不负责 drop

这一点需要保持。ActivityBar 才是稳定存在的 drop 目标，因为 SlotContainer 会根据 visible panel 数量条件渲染，不能承担跨侧移动的 drop target 职责。

## 4. 当前保留但需要后续清理的问题

### 4.1 `PanelConfig.icon` 应废弃

当前 `layoutStore.tsx` 中仍保留：

```ts
icon: string
```

以及默认配置中的：

```ts
icon: 'Files'
icon: 'List'
```

本次为了减少状态迁移范围，没有立刻删除该字段。但从设计上它已经不应该再作为渲染依据。

后续建议：

1. 删除 `PanelConfig.icon` 字段。
2. 使用 `PanelType` 联合类型替代 `type: string`：
   - `'file-tree'`
   - `'toc'`
3. 建立面板注册表：

```ts
const PANEL_REGISTRY = {
  'file-tree': {
    title: '文件',
    icon: FileTreeIcon,
    render: FileTreePanel,
  },
  toc: {
    title: '目录',
    icon: TocIcon,
    render: TocPanel,
  },
}
```

这样可以避免 title、icon、render 分散在多个文件里。

### 4.2 localStorage 需要版本化

当前 `layoutStore` 直接读取 `wem-layout`，没有 schema version。

问题：

- 旧字段会长期残留。
- 字段变更后无法判断是否需要迁移。
- 错误布局可能永久影响用户 UI。

建议改为：

```ts
interface PersistedLayoutState {
  version: number
  panels: PanelConfig[]
  slots: Record<SlotPosition, SlotState>
}
```

并实现：

- `CURRENT_LAYOUT_VERSION`
- `migrateLayoutState`
- 无法迁移时回退默认布局

### 4.3 App 中面板渲染逻辑重复

当前 `App.tsx` 在 left/right/top 三处重复写了：

- `panel.type === 'file-tree'`
- `panel.type === 'toc'`

这会导致新增面板时需要改多个区域。

建议新增：

```tsx
function PanelContentRenderer({ panel }) {
  switch (panel.type) {
    case 'file-tree': ...
    case 'toc': ...
  }
}
```

或者更进一步使用面板注册表统一渲染。

## 5. 未来布局系统设计方向

### 5.1 组件职责边界

建议将布局系统拆成四层：

#### 5.1.1 LayoutStore

只管理状态：

- 面板在哪个 slot
- 面板是否 visible
- slot 尺寸
- 面板排序
- 持久化与迁移

不要管理具体 UI。

#### 5.1.2 LayoutShell

负责页面布局结构：

- 左 ActivityBar
- 左 Slot
- 中央编辑区
- 右 Slot
- 右 ActivityBar
- 顶部 Slot

不要负责具体面板内容。

#### 5.1.3 ActivityBar

负责：

- 展示某一侧面板入口
- 点击切换 visible
- 作为 drag source
- 作为 drop target
- 展示拖拽反馈

不负责：

- 渲染面板内容
- 改变 slot 尺寸
- 依赖 Tooltip 作为结构基础

#### 5.1.4 SlotContainer

负责：

- 渲染 visible panel
- resize
- overflow 管理

不负责：

- 接收跨侧拖拽
- 管理图标
- 管理面板注册

### 5.2 面板注册表方向

后续建议引入 `panelRegistry.tsx`：

```ts
export type PanelType = 'file-tree' | 'toc'

export const PANEL_REGISTRY = {
  'file-tree': {
    title: '文件',
    icon: FileTreeIcon,
    render: FileTreePanel,
  },
  toc: {
    title: '目录',
    icon: TocIcon,
    render: TocPanel,
  },
} satisfies Record<PanelType, PanelDefinition>
```

收益：

1. 图标、标题、渲染入口集中管理。
2. 新增面板只改一个注册文件。
3. `PanelConfig` 只保存用户布局状态，不保存静态元信息。
4. 可以给每个面板声明能力，例如：
   - 是否允许移动
   - 是否允许隐藏
   - 最小宽度
   - 推荐 slot

### 5.3 拖拽设计方向

短期可继续使用 HTML5 Drag and Drop，因为目前只需要跨 ActivityBar 移动。

中长期建议：

1. 抽出 `usePanelDrag` hook。
2. 抽出 `usePanelDrop` hook。
3. 支持拖拽排序：在 ActivityBar 内拖动改变 order。
4. 支持拖到 top slot：可以考虑在中央顶部增加一个稳定 drop zone，而不是依赖 top SlotContainer 条件渲染。
5. 拖动时显示 ghost preview，例如显示面板标题和图标。

### 5.4 Tooltip 设计方向

Tooltip 应作为增强层，而不是结构层。

建议后续做法：

- ActivityBarIconButton 是稳定原生 button。
- Tooltip 使用 wrapper 或独立轻量实现。
- 不让 Tooltip 决定 button 的 children。

例如：

```tsx
<Tooltip label={panel.title}>
  <ActivityBarIconButton ... />
</Tooltip>
```

如果第三方 Tooltip 会影响 children 或 render 行为，应避免用于 ActivityBar 这种核心交互入口。

## 6. 视觉与美化方向

### 6.1 ActivityBar

建议采用 VS Code + Linear 风格：

- 宽度保持 48px。
- 图标按钮 44px。
- active 状态使用左/右侧 2px 指示条。
- hover 使用轻量背景，不使用过重阴影。
- 拖拽时图标透明并缩小。
- drop over 时 ActivityBar 背景轻微染色，并显示虚线边框。

### 6.2 SlotContainer

建议：

- 背景用 `bg-muted/10` 或 `bg-sidebar/40`。
- resize handle 默认透明，hover 时变为主题色。
- resize 时可以添加全局 cursor 与 user-select none。
- 面板隐藏后 ActivityBar 仍可见，避免布局消失导致无法恢复。

### 6.3 PanelContainer

建议后续优化：

- 标题栏高度统一。
- 支持 close / pin / collapse 操作。
- 支持面板内部 tab 化。
- 空状态要有说明，例如 TOC 为空时提示“当前文档暂无标题”。

### 6.4 主题一致性

所有布局组件应只使用设计 token：

- `bg-background`
- `bg-sidebar`
- `bg-muted`
- `text-foreground`
- `text-muted-foreground`
- `border-border`
- `ring-ring`
- `bg-primary`

避免写死颜色，方便之后做深色主题和自定义主题。

## 7. 建议的后续迭代顺序

### 第一阶段：稳定性 ✅ 已完成（2026-04-26 第二轮）

1. ✅ 删除 `PanelConfig.icon`。
2. ✅ 引入 `PanelType` 联合类型。
3. ✅ 给 localStorage 加 version 和 migration。
4. ✅ 抽出 `PanelContentRenderer`，消除 App 重复逻辑。
5. ✅ 建立面板注册表 `panelRegistry.tsx`。
6. ✅ PanelContainer 增加面板标题栏。
7. ✅ SlotContainer resize 时添加全局 cursor 和 user-select 反馈。
8. ✅ ActivityBar 使用注册表图标，不再内置 SVG 定义。

### 第二阶段：交互完善

1. ActivityBar 内拖拽排序。
2. 支持拖到顶部槽位。
3. 拖拽 ghost preview。
4. ~~resize 时全局反馈~~ ✅ 已完成。
5. 快捷键控制左右面板显示。

### 第三阶段：视觉美化

1. ActivityBar tooltip 恢复，但不使用 TooltipTrigger 作为按钮结构。
2. ~~面板标题栏统一设计~~ ✅ 已完成（PanelContainer 从 registry 获取图标+标题）。
3. 面板空状态设计。
4. 活动栏图标 hover/active 动画统一。
5. 支持主题切换。

### 第四阶段：架构升级

1. ~~建立 `panelRegistry`~~ ✅ 已完成。
2. 建立 `LayoutShell`。
3. ~~建立 `PanelRenderer`~~ ✅ 已完成（`PanelContentRenderer`）。
4. ~~建立 layout persistence migration~~ ✅ 已完成。
5. 为布局 store 增加单元测试。

## 8. 本次结论

### 第一轮（2026-04-26）

当前最不合理的地方是：把 ActivityBar 图标显示依赖在 TooltipTrigger、lucide 动态映射和 localStorage 字符串 icon 上。这个设计链路过长，任何一环异常都会导致图标消失。

已将 ActivityBar 的核心显示链路改为：

```text
panel.type -> PanelIcon -> 本地 SVG -> 原生 button
```

### 第二轮（2026-04-26）

第一轮只解决了图标显示问题，但留下了大量设计债务：`PanelConfig.icon` 死字段残留、面板渲染逻辑在 App.tsx 三处重复、localStorage 无版本号、PanelContainer 没有标题栏。

第二轮完成了设计报告第一阶段全部目标：

1. **面板注册表**（`panelRegistry.tsx`）：`PanelType` 联合类型 + `PanelDefinition` + `PanelContentRenderer`，新增面板只需改一个文件。
2. **layoutStore 重构**：删除 `icon`/`title` 字段，PanelConfig 只管布局状态；localStorage 加 `version: 1` + `migrateV0` 迁移函数，自动清理旧 icon 字段、补全新面板。
3. **ActivityBar 精简**：图标和标题从注册表获取，不再内置 SVG 定义；使用 `getAllSlotPanels` 替代直接过滤 `panels`。
4. **PanelContainer 标题栏**：从注册表获取图标+标题，统一 32px 高度标题栏。
5. **App.tsx 去重**：三处 slot 渲染全部使用 `<PanelContentRenderer panel={panel} {...contentProps} />`，新增面板无需修改 App.tsx。
6. **SlotContainer resize 体验**：拖拽期间设置全局 `cursor: col-resize` + `user-select: none`，松开后恢复。

整体架构从「散落」变为：

```text
panelRegistry（静态定义）
  ↓ type/panel.icon/panel.title/panel.render
layoutStore（运行时布局状态 + 持久化 + 迁移）
  ↓ PanelConfig(slot/order/visible)
ActivityBar ← registry 图标 + store 状态
SlotContainer ← store 尺寸
PanelContainer ← registry 标题 + 内容渲染
App.tsx ← PanelContentRenderer（一行搞定面板渲染）
```
