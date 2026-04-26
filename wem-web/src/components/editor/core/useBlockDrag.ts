/**
 * useBlockDrag — 块拖拽 Hook
 *
 * 基于 HTML5 Drag and Drop API 实现块拖拽移动。
 * 所有块对外平铺为一行行，拖拽只有 before / after 两种放置位置。
 * 块的层级关系（heading 子块、listItem 归属 list）由后端自动处理。
 */

import { useCallback, useMemo, useRef, useState } from "react";
import type { BlockNode } from "@/types/api";
import type {
  BlockAction,
  DragHandlers,
  DragState,
  DropTarget,
} from "./types";
import { findBlockById, flattenTree } from "./BlockOperations";

// ─── Hook 参数 ───

export interface UseBlockDragOptions {
  /** 获取当前块树快照 */
  getTree: () => BlockNode[];
  /** 拖拽完成时发出 action */
  onAction: (action: BlockAction) => void;
  /** 判断块是否处于折叠状态（保留接口，当前未使用） */
  isCollapsed: (blockId: string) => boolean;
}

// ─── Hook ───

export function useBlockDrag(options: UseBlockDragOptions) {
  // Keep latest option values in refs so callbacks can be stable
  const getTreeRef = useRef(options.getTree)
  getTreeRef.current = options.getTree
  const onActionRef = useRef(options.onAction)
  onActionRef.current = options.onAction
  const isCollapsedRef = useRef(options.isCollapsed)
  isCollapsedRef.current = options.isCollapsed

  const [dragState, setDragState] = useState<DragState>({
    draggingBlockId: null,
    dropTarget: null,
  });

  /** 拖拽开始时的块 ID（通过 dataTransfer 持久化） */
  const dragBlockIdRef = useRef<string | null>(null);

  /**
   * 根据鼠标位置计算放置位置：上半区 → before，下半区 → after。
   */
  const computeDropPosition = useCallback(
    (e: React.DragEvent): 'before' | 'after' => {
      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
      const midY = rect.top + rect.height / 2;
      return e.clientY < midY ? "before" : "after";
    },
    [],
  );

  // ─── 拖拽事件处理 ───

  const onDragStart = useCallback(
    (e: React.DragEvent, blockId: string) => {
      const tree = getTreeRef.current();
      const flat = flattenTree(tree);
      if (flat.length <= 1) {
        e.preventDefault();
        return;
      }

      dragBlockIdRef.current = blockId;
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", blockId);

      setDragState({ draggingBlockId: blockId, dropTarget: null });
    },
    [],
  );

  const onDragOver = useCallback(
    (e: React.DragEvent, targetBlockId: string) => {
      e.preventDefault();
      e.stopPropagation();

      const dragBlockId = dragBlockIdRef.current;
      if (!dragBlockId || dragBlockId === targetBlockId) return;

      const position = computeDropPosition(e);
      e.dataTransfer.dropEffect = "move";

      setDragState((prev) => {
        if (
          prev.dropTarget?.blockId === targetBlockId &&
          prev.dropTarget?.position === position
        ) {
          return prev;
        }
        return { draggingBlockId: dragBlockId, dropTarget: { blockId: targetBlockId, position } };
      });
    },
    [computeDropPosition],
  );

  const onDragLeave = useCallback((e: React.DragEvent, _blockId: string) => {
    // 只有离开当前块时才清除（不是进入子元素）
    const relatedTarget = e.relatedTarget as HTMLElement | null;
    const currentTarget = e.currentTarget as HTMLElement;
    if (relatedTarget && currentTarget.contains(relatedTarget)) {
      return;
    }

    setDragState((prev) => {
      if (prev.dropTarget?.blockId === _blockId) {
        return { ...prev, dropTarget: null };
      }
      return prev;
    });
  }, []);

  const onDrop = useCallback(
    (e: React.DragEvent, targetBlockId: string) => {
      e.preventDefault();
      e.stopPropagation();

      const dragBlockId = dragBlockIdRef.current;
      if (!dragBlockId || dragBlockId === targetBlockId) return;

      const position = computeDropPosition(e);
      const target: DropTarget = { blockId: targetBlockId, position };

      dragBlockIdRef.current = null;
      setDragState({ draggingBlockId: null, dropTarget: null });

      const tree = getTreeRef.current();
      const draggedBlock = findBlockById(tree, dragBlockId);

      // ── 折叠的 heading（有子块）或 list（有子项）→ 整棵子树整体移动 ──
      if (draggedBlock && draggedBlock.children.length > 0) {
        const isHeading = draggedBlock.block_type.type === 'heading';
        const isList = draggedBlock.block_type.type === 'list';
        if (
          (isHeading && isCollapsedRef.current(dragBlockId)) ||
          isList
        ) {
          onActionRef.current({ type: 'move-heading-tree', blockId: dragBlockId, target });
          return;
        }
      }

      // ── 普通块 → 单块移动 ──
      onActionRef.current({ type: "move-block", blockId: dragBlockId, target });
    },
    [computeDropPosition],
  );

  const onDragEnd = useCallback(() => {
    dragBlockIdRef.current = null;
    setDragState({ draggingBlockId: null, dropTarget: null });
  }, []);

  const dragHandlers: DragHandlers = useMemo(() => ({
    onDragStart,
    onDragOver,
    onDragLeave,
    onDrop,
    onDragEnd,
  }), [onDragStart, onDragOver, onDragLeave, onDrop, onDragEnd]);

  return { dragState, dragHandlers };
}
