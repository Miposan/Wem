//! App — TUI 应用状态机
//!
//! 通过 AgentRuntime 直接调库，不走 HTTP。

use std::collections::HashMap;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::broadcast;

use crate::agent::runtime::AgentRuntime;
use crate::agent::session::{AgentEvent, SessionConfig};

// ─── 对话条目 ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ChatEntry {
    UserMessage { text: String },
    AssistantText { text: String },
    ToolCard {
        name: String,
        args_summary: String,
        status: ToolCallStatus,
        result: Option<String>,
    },
    SystemInfo { text: String },
    Error { text: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallStatus { Running, Done, Error }

// ─── 应用阶段 ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum AppPhase {
    Idle,
    Thinking,
    Streaming,
    ExecutingTools,
    WaitingPermission,
}

impl AppPhase {
    pub fn label(&self) -> &str {
        match self {
            Self::Idle => "",
            Self::Thinking => "Thinking",
            Self::Streaming => "Responding",
            Self::ExecutingTools => "Running tools",
            Self::WaitingPermission => "Awaiting approval",
        }
    }
}

// ─── App ───────────────────────────────────────────────────

pub struct App {
    pub model: String,
    pub entries: Vec<ChatEntry>,
    pub phase: AppPhase,
    pub streaming_text: String,
    pub active_tool_calls: HashMap<String, (String, String)>,
    pub step: u32,
    pub max_steps: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,

    // Input
    pub input: String,
    pub cursor: usize,
    pub input_history: Vec<String>,
    pub history_index: usize,

    // Scroll
    pub scroll_offset: u16,
    pub auto_scroll: bool,

    // Agent (直接调库)
    pub runtime: Arc<AgentRuntime>,
    pub session_id: String,
    agent_rx: Option<broadcast::Receiver<AgentEvent>>,

    pub running: bool,
    pub pending_message: Option<String>,
}

impl App {
    pub fn new(runtime: Arc<AgentRuntime>, model: String) -> Self {
        let mut session_cfg = SessionConfig::default();
        session_cfg.model = model.clone();
        session_cfg.working_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let session_id = runtime.create_session(session_cfg);

        Self {
            model,
            entries: Vec::new(),
            phase: AppPhase::Idle,
            streaming_text: String::new(),
            active_tool_calls: HashMap::new(),
            step: 0, max_steps: 0,
            total_input_tokens: 0, total_output_tokens: 0,
            input: String::new(),
            cursor: 0,
            input_history: Vec::new(),
            history_index: 0,
            scroll_offset: 0,
            auto_scroll: true,
            runtime,
            session_id,
            agent_rx: None,
            running: true,
            pending_message: None,
        }
    }

    /// 字符索引 → 字节偏移
    fn byte_pos(&self, char_idx: usize) -> usize {
        self.input.char_indices().nth(char_idx).map(|(i, _)| i).unwrap_or(self.input.len())
    }

    /// 字符总数
    fn char_count(&self) -> usize {
        self.input.chars().count()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.phase != AppPhase::Idle {
                    self.runtime.abort_session(&self.session_id);
                    self.finalize_streaming();
                    self.phase = AppPhase::Idle;
                    self.agent_rx = None;
                    self.entries.push(ChatEntry::SystemInfo { text: "(cancelled)".into() });
                } else {
                    self.running = false;
                }
                return true;
            }
            KeyCode::Enter => self.submit_input(),
            KeyCode::Char(c) => { self.input.insert(self.byte_pos(self.cursor), c); self.cursor += 1; }
            KeyCode::Backspace => { if self.cursor > 0 { self.cursor -= 1; self.input.remove(self.byte_pos(self.cursor)); } }
            KeyCode::Delete => { if self.cursor < self.char_count() { self.input.remove(self.byte_pos(self.cursor)); } }
            KeyCode::Left => { if self.cursor > 0 { self.cursor -= 1; } }
            KeyCode::Right => { if self.cursor < self.char_count() { self.cursor += 1; } }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.char_count(),
            KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
                self.auto_scroll = false;
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
                if self.scroll_offset == 0 { self.auto_scroll = true; }
            }
            KeyCode::PageUp => { self.scroll_offset = self.scroll_offset.saturating_add(10); self.auto_scroll = false; }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                if self.scroll_offset == 0 { self.auto_scroll = true; }
            }
            KeyCode::Up if self.phase == AppPhase::Idle => {
                if !self.input_history.is_empty() {
                    if self.history_index < self.input_history.len() { self.history_index += 1; }
                    let idx = self.input_history.len().saturating_sub(self.history_index);
                    self.input = self.input_history[idx].clone();
                    self.cursor = self.char_count();
                }
            }
            KeyCode::Down if self.phase == AppPhase::Idle => {
                if self.history_index > 1 {
                    self.history_index -= 1;
                    let idx = self.input_history.len().saturating_sub(self.history_index);
                    self.input = self.input_history[idx].clone();
                    self.cursor = self.char_count();
                } else { self.history_index = 0; self.input.clear(); self.cursor = 0; }
            }
            _ => {}
        }
        true
    }

    fn submit_input(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() { return; }
        if text.starts_with('/') {
            self.handle_slash_command(&text);
            self.input.clear(); self.cursor = 0; self.history_index = 0;
            return;
        }
        if self.phase != AppPhase::Idle { return; }

        self.entries.push(ChatEntry::UserMessage { text: text.clone() });
        self.input_history.push(text.clone());
        self.history_index = 0;
        self.input.clear(); self.cursor = 0;

        self.phase = AppPhase::Thinking;
        self.streaming_text.clear();
        self.active_tool_calls.clear();

        // Store for async processing in main loop
        self.pending_message = Some(text);
    }

    /// Process pending message (called from async main loop)
    pub async fn send_pending_message(&mut self) {
        let Some(text) = self.pending_message.take() else { return };
        match self.runtime.start_chat_stream(&self.session_id, text, None).await {
            Ok(rx) => self.agent_rx = Some(rx),
            Err(e) => {
                self.entries.push(ChatEntry::Error { text: format!("Failed to start chat: {}", e) });
                self.phase = AppPhase::Idle;
            }
        }
    }

    pub fn poll_agent_events(&mut self) {
        let events: Vec<AgentEvent> = {
            let Some(rx) = &mut self.agent_rx else { return };
            let mut collected = Vec::new();
            loop {
                match rx.try_recv() {
                    Ok(event) => collected.push(event),
                    Err(broadcast::error::TryRecvError::Empty) => break,
                    Err(broadcast::error::TryRecvError::Closed) => { self.agent_rx = None; break; }
                    Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                }
            }
            collected
        };
        for event in events { self.handle_agent_event(event); }
    }

    fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::TextDelta { text } => {
                self.streaming_text.push_str(&text);
                self.phase = AppPhase::Streaming;
            }
            AgentEvent::ToolCallBegin { id, name, args } => {
                let args_str = args.to_string();
                let summary = if args_str.len() > 80 {
                    format!("{}...", &args_str[..args_str.floor_char_boundary(80)])
                } else { args_str };
                self.active_tool_calls.insert(id, (name.clone(), summary.clone()));
                self.finalize_streaming();
                self.entries.push(ChatEntry::ToolCard {
                    name, args_summary: summary, status: ToolCallStatus::Running, result: None,
                });
                self.phase = AppPhase::ExecutingTools;
            }
            AgentEvent::ToolCallEnd { id, result_summary } => {
                if let Some((name, args_summary)) = self.active_tool_calls.remove(&id) {
                    let is_error = result_summary.starts_with("Error:");
                    self.entries.push(ChatEntry::ToolCard {
                        name, args_summary,
                        status: if is_error { ToolCallStatus::Error } else { ToolCallStatus::Done },
                        result: Some(result_summary),
                    });
                }
            }
            AgentEvent::StepProgress { step, max_steps } => { self.step = step; self.max_steps = max_steps; }
            AgentEvent::PhaseChanged { phase } => {
                use crate::agent::session::Phase;
                match phase {
                    Phase::PreparingTurn => self.phase = AppPhase::Thinking,
                    Phase::StreamingModel => self.phase = AppPhase::Streaming,
                    Phase::ExecutingTools => self.phase = AppPhase::ExecutingTools,
                    _ => {}
                }
            }
            AgentEvent::PermissionRequired { tool_name, .. } => {
                self.phase = AppPhase::WaitingPermission;
                self.entries.push(ChatEntry::SystemInfo {
                    text: format!("Permission required: {} (auto-approved)", tool_name),
                });
                // Auto-approve for now; Phase 4 will add interactive approval
            }
            AgentEvent::Done => {
                self.finalize_streaming();
                self.phase = AppPhase::Idle;
                self.agent_rx = None;
            }
            AgentEvent::Error { message } => {
                self.finalize_streaming();
                self.entries.push(ChatEntry::Error { text: message });
                self.phase = AppPhase::Idle;
                self.agent_rx = None;
            }
        }
    }

    fn finalize_streaming(&mut self) {
        if !self.streaming_text.is_empty() {
            let text = std::mem::take(&mut self.streaming_text);
            self.entries.push(ChatEntry::AssistantText { text });
        }
    }

    fn handle_slash_command(&mut self, cmd: &str) {
        match cmd {
            "/quit" | "/exit" => self.running = false,
            "/help" => {
                self.entries.push(ChatEntry::SystemInfo {
                    text: "Commands: /help, /quit, /clear, /model, /cost, /history, /sessions".into(),
                });
            }
            "/clear" => {
                self.entries.clear();
                let mut session_cfg = SessionConfig::default();
                session_cfg.model = self.model.clone();
                session_cfg.working_dir = std::env::current_dir().unwrap_or_else(|_| ".".into());
                self.session_id = self.runtime.create_session(session_cfg);
                self.entries.push(ChatEntry::SystemInfo { text: "Conversation cleared.".into() });
            }
            "/model" => {
                self.entries.push(ChatEntry::SystemInfo { text: format!("Model: {}", self.model) });
            }
            "/cost" => {
                self.entries.push(ChatEntry::SystemInfo {
                    text: format!("Tokens — input: {}, output: {}", self.total_input_tokens, self.total_output_tokens),
                });
            }
            "/history" => {
                self.entries.push(ChatEntry::SystemInfo { text: format!("{} entries", self.entries.len()) });
            }
            "/sessions" => {
                let sessions = self.runtime.list_sessions();
                self.entries.push(ChatEntry::SystemInfo {
                    text: format!("Sessions: {} (current: {})", sessions.len(), &self.session_id[..8]),
                });
            }
            _ => {
                self.entries.push(ChatEntry::Error { text: format!("Unknown command: {}", cmd) });
            }
        }
    }
}
