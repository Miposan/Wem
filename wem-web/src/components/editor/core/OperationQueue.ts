/**
 * 操作队列 — 序列化所有编辑器结构变更操作
 *
 * 保证快速连续操作（如快速 Enter）时每个操作严格串行执行，
 * 避免并发修改导致数据不一致。
 *
 * 设计参考：BlockNote 的 Transaction 模式 + ProseMirror 的同步事务
 */

export interface QueueEntry {
  /** 操作描述（用于调试） */
  label: string
  /** 异步执行函数 */
  execute: () => Promise<void>
}

/**
 * 编辑器操作队列
 *
 * - enqueue() 将操作加入队列末尾，如果队列空闲则立即执行
 * - 队列严格串行：每个操作完成后才执行下一个
 * - 操作失败不影响后续操作（catch 后继续）
 * - 不设容量上限：每个操作都是用户的明确意图，不应丢弃
 */

/** 单个操作的超时时间（ms），防止异常操作永久阻塞队列 */
const OP_TIMEOUT_MS = 3_000

export class OperationQueue {
  private queue: QueueEntry[] = []
  private running = false
  private onError?: (error: unknown, label: string) => void

  constructor(onError?: (error: unknown, label: string) => void) {
    this.onError = onError
  }

  /**
   * 将操作加入队列
   * @returns true（始终入队成功）
   */
  enqueue(entry: QueueEntry): boolean {
    this.queue.push(entry)
    if (!this.running) {
      this.runNext()
    }
    return true
  }

  /** 当前队列中等待的操作数（不含正在执行的） */
  get pendingCount(): number {
    return this.queue.length
  }

  /** 是否正在执行操作 */
  isRunning(): boolean {
    return this.running
  }

  /** 清空队列中未执行的操作 */
  clear(): void {
    this.queue = []
  }

  private async runNext(): Promise<void> {
    if (this.queue.length === 0) {
      this.running = false
      return
    }

    this.running = true
    const entry = this.queue.shift()!

    try {
      await Promise.race([
        entry.execute(),
        new Promise<never>((_, reject) =>
          setTimeout(() => reject(new Error(`操作 "${entry.label}" 超时`)), OP_TIMEOUT_MS),
        ),
      ])
    } catch (err) {
      this.onError?.(err, entry.label)
    }

    // 继续下一个（即使当前失败）
    await this.runNext()
  }
}
