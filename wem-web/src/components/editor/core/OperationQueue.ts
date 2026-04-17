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
 * - 超过 MAX_PENDING 积压时丢弃新操作，防止快速操作无限堆积
 */

/** 队列允许的最大积压操作数 */
const MAX_PENDING = 3

export class OperationQueue {
  private queue: QueueEntry[] = []
  private running = false
  private onError?: (error: unknown, label: string) => void

  constructor(onError?: (error: unknown, label: string) => void) {
    this.onError = onError
  }

  /**
   * 将操作加入队列
   * @returns true 表示入队成功，false 表示队列积压已满被丢弃
   */
  enqueue(entry: QueueEntry): boolean {
    if (this.queue.length >= MAX_PENDING) {
      // 队列积压过多，丢弃（避免快速操作无限堆积）
      console.warn(`[OperationQueue] 丢弃操作 "${entry.label}"，积压 ${this.queue.length}/${MAX_PENDING}`)
      return false
    }
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
      await entry.execute()
    } catch (err) {
      this.onError?.(err, entry.label)
    }

    // 继续下一个（即使当前失败）
    await this.runNext()
  }
}
