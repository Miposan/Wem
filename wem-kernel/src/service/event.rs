//! EventBus — 全局事件广播通道
//!
//! 使用 `tokio::sync::broadcast` 实现一对多事件分发。
//!
//! **使用方式**：
//! - Handler 层：mutation 成功后调用 `EventBus::global().emit(BlockEvent::...)`
//! - SSE 端点：调用 `EventBus::global().subscribe()` 获取事件流
//!
//! 发送是同步的（`broadcast::Sender::send()` 不需要 async），可以在
//! `spawn_blocking` 内的 service 函数中直接调用。

use std::sync::LazyLock;

use tokio::sync::broadcast;

use crate::model::event::BlockEvent;

/// 事件广播通道容量
///
/// 超过此容量时，慢消费者会收到 `RecvError::Lagged`（旧事件被丢弃）。
/// 256 足以应对高频操作场景（如快速连续 Enter）。
const CHANNEL_CAPACITY: usize = 256;

// ─── EventBus ─────────────────────────────────────────────────

/// 全局事件总线
///
/// 单例模式：整个进程只有一个 EventBus 实例。
/// Handler 层 emit，SSE 端点 subscribe，解耦生产者和消费者。
pub struct EventBus {
    sender: broadcast::Sender<BlockEvent>,
}

/// 全局单例（Rust 2024 edition 的 LazyLock）
static EVENT_BUS: LazyLock<EventBus> = LazyLock::new(|| {
    let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
    EventBus { sender: tx }
});

impl EventBus {
    /// 获取全局 EventBus 实例
    pub fn global() -> &'static Self {
        &EVENT_BUS
    }

    /// 广播一个事件
    ///
    /// 非阻塞：将事件发送到 broadcast channel，所有 `subscribe()` 的接收者都能收到。
    /// 如果没有订阅者，事件被静默丢弃（`send` 返回 `Err(RecvError::Closed)`）。
    pub fn emit(&self, event: BlockEvent) {
        if let Err(e) = self.sender.send(event) {
            // 没有订阅者时静默忽略，这是正常情况（前端未连接 SSE）
            tracing::debug!("EventBus emit skipped: {}", e);
        }
    }

    /// 订阅事件流
    ///
    /// 返回一个 `broadcast::Receiver`，在 async 上下文中调用 `recv()` 获取事件。
    /// 新订阅者只会收到订阅之后的事件。
    pub fn subscribe(&self) -> broadcast::Receiver<BlockEvent> {
        self.sender.subscribe()
    }
}
