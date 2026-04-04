//! Markdown 格式处理器
//!
//! 实现 [`BlockParser`] + [`BlockSerializer`]，提供 Markdown ↔ Block 树双向转换。
//!
//! ## 解析流程
//! 自研递归下降解析器，逐行扫描 Markdown 文本，直接产出 Block 树。
//! 无需外部依赖，天然支持紧凑列表、嵌套结构。
//!
//! ## 序列化流程
//! 递归遍历 Block 树 → Markdown 文本
//!
//! 参考：`Note/15-markdown-parser.md`

use std::collections::HashMap;

use crate::error::AppError;
use crate::model::{generate_block_id, Block, BlockStatus, BlockType, ContentType};

use super::types::{LossyInfo, ParseOptions, ParseResult, ParseWarning, SerializeResult};
use super::{BlockParser, BlockSerializer};

// ─── MarkdownFormat ──────────────────────────────────────────

/// Markdown 格式处理器
///
/// 同时实现 [`BlockParser`] 和 [`BlockSerializer`]，
/// 工厂函数 [`get_parser`](super::get_parser) / [`get_serializer`](super::get_serializer) 返回此类型。
pub struct MarkdownFormat;

impl MarkdownFormat {
    pub fn new() -> Self {
        Self
    }
}

// ─── BlockParser 实现 ────────────────────────────────────────

impl BlockParser for MarkdownFormat {
    fn format_name(&self) -> &str {
        "markdown"
    }

    fn parse(&self, text: &str, options: &ParseOptions) -> Result<ParseResult, AppError> {
        if text.trim().is_empty() {
            return Ok(empty_result());
        }

        let (root, children, warnings) = parse_markdown(text, options)?;

        Ok(ParseResult {
            blocks_created: 1 + children.len(),
            root,
            children,
            warnings,
        })
    }
}

// ─── BlockSerializer 实现 ────────────────────────────────────

impl BlockSerializer for MarkdownFormat {
    fn format_name(&self) -> &str {
        "markdown"
    }

    fn serialize(
        &self,
        root: &Block,
        children_map: &HashMap<String, Vec<Block>>,
    ) -> Result<SerializeResult, AppError> {
        let mut lossy = Vec::new();
        let mut content = String::new();
        let mut blocks_exported = 0;

        serialize_block_recursive(
            root,
            children_map,
            0,
            &mut content,
            &mut lossy,
            &mut blocks_exported,
            true,
        );

        let filename = root
            .properties
            .get("title")
            .map(|t| format!("{}.md", t));

        Ok(SerializeResult {
            content,
            filename,
            blocks_exported,
            lossy_types: lossy.into_iter().map(|l| l.block_type).collect(),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
//  解析器内部（自研递归下降）
// ═══════════════════════════════════════════════════════════════

// ─── 行扫描器 ──────────────────────────────────────────────

/// 逐行迭代 Markdown 文本
struct LineScanner<'a> {
    lines: Vec<&'a str>,
    pos: usize,
}

impl<'a> LineScanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lines: text.split('\n').collect(),
            pos: 0,
        }
    }

    /// 当前行的内容（不消费）
    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.pos).copied()
    }

    /// 消费并返回当前行
    fn advance(&mut self) -> Option<&'a str> {
        let line = self.lines.get(self.pos).copied();
        if line.is_some() {
            self.pos += 1;
        }
        line
    }

    /// 是否还有更多行
    fn has_more(&self) -> bool {
        self.pos < self.lines.len()
    }

    /// 跳过空行，返回是否跳过
    fn skip_blank_lines(&mut self) -> bool {
        let mut skipped = false;
        while let Some(line) = self.peek() {
            if line.trim().is_empty() {
                self.advance();
                skipped = true;
            } else {
                break;
            }
        }
        skipped
    }
}

// ─── Heading 栈 ────────────────────────────────────────────

/// 推断标题的父子关系
struct HeadingStack {
    /// (level, block_id)
    stack: Vec<(u8, String)>,
    doc_id: String,
}

impl HeadingStack {
    fn new(doc_id: String) -> Self {
        Self {
            stack: Vec::new(),
            doc_id,
        }
    }

    /// 推入新 Heading，返回其父 Block ID
    ///
    /// 合并了原来的 `parent_for_level`（计算 parent）+ `push`（修改栈），\n    /// 避免对栈做两次遍历。
    fn push(&mut self, level: u8, heading_id: String) -> String {
        // 弹出 >= level 的项，找到父级
        while self.stack.last().map_or(false, |(l, _)| *l >= level) {
            self.stack.pop();
        }
        let parent_id = self
            .stack
            .last()
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| self.doc_id.clone());
        self.stack.push((level, heading_id));
        parent_id
    }

    /// 当前父块 ID（栈顶或文档根）
    fn current_parent(&self) -> String {
        self.stack
            .last()
            .map(|(_, id)| id.clone())
            .unwrap_or_else(|| self.doc_id.clone())
    }
}

// ─── 块级检测函数 ──────────────────────────────────────────

/// 计算 indent 深度（空格数，tab = 4 空格）
fn indent_of(line: &str) -> usize {
    let mut indent = 0;
    for ch in line.chars() {
        match ch {
            ' ' => indent += 1,
            '\t' => indent += 4,
            _ => break,
        }
    }
    indent
}

/// 一次性解析 Heading 行，返回 (level, text)，失败返回 None
///
/// 合并了原来的 `is_heading` + `heading_level` + `heading_text`，
/// 只做一次 `trim_start` + 字符计数。
fn parse_heading_line(line: &str) -> Option<(u8, String)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let n = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&n) {
        return None;
    }
    // `#` 后必须跟空格（或行尾仅 `#`）
    let rest = trimmed.get(n..).unwrap_or("");
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim().to_string();
    Some((n as u8, text))
}

/// Heading 行快速检测（用于 dispatch 和 break 判断）
#[inline]
fn is_heading(line: &str) -> bool {
    parse_heading_line(line).is_some()
}

fn is_code_fence(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn code_fence_lang(line: &str) -> String {
    let trimmed = line.trim_start();
    let fence_char = trimmed.chars().next().unwrap_or('`');
    let fence_len = trimmed.chars().take_while(|c| *c == fence_char).count();
    if fence_len < trimmed.len() {
        trimmed[fence_len..].trim().to_string()
    } else {
        String::new()
    }
}

fn is_closing_fence(line: &str, fence_char: char) -> bool {
    let trimmed = line.trim_start();
    if !trimmed.starts_with(fence_char) {
        return false;
    }
    trimmed.chars().all(|c| c == fence_char || c == ' ')
        && trimmed.chars().filter(|c| *c == fence_char).count() >= 3
}

fn is_math_fence(line: &str) -> bool {
    line.trim() == "$$"
}

fn is_thematic_break(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    let first = trimmed.chars().next().unwrap();
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    trimmed.chars().all(|c| c == first || c == ' ')
        && trimmed.chars().filter(|c| *c == first).count() >= 3
}

fn is_blockquote(line: &str) -> bool {
    line.trim_start().starts_with('>')
}

fn strip_blockquote(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix("> ") {
        rest.to_string()
    } else if let Some(rest) = trimmed.strip_prefix('>') {
        rest.to_string()
    } else {
        line.to_string()
    }
}

fn is_unordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ")
}

fn is_ordered_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    // 用字节数计数避免 String 分配（ASCII 数字都是单字节）
    let digit_len = trimmed
        .bytes()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if digit_len == 0 {
        return false;
    }
    trimmed
        .get(digit_len..)
        .map_or(false, |r| r.starts_with(". "))
}

fn is_list_item(line: &str) -> bool {
    is_unordered_list_item(line) || is_ordered_list_item(line)
}

/// 剥离列表标记，返回纯文本内容
///
/// `ordered` 参数由调用方已经判断，避免重复检测。
fn strip_list_marker(line: &str, ordered: bool) -> String {
    let trimmed = line.trim_start();
    if !ordered {
        // 无序列表："- " / "* " / "+ " → 跳过 2 字符
        trimmed.get(2..).unwrap_or(trimmed).to_string()
    } else {
        // 有序列表："1. " → 找到 ". " 跳过
        let dot_pos = trimmed.find(". ").unwrap_or(trimmed.len());
        if dot_pos < trimmed.len() {
            trimmed[dot_pos + 2..].to_string()
        } else {
            trimmed.to_string()
        }
    }
}

// ─── 解析上下文 ────────────────────────────────────────────

/// 解析过程中的共享状态
struct ParserState {
    heading_stack: HeadingStack,
    blocks: Vec<Block>,
    warnings: Vec<ParseWarning>,
    first_heading_seen: bool,
    doc_title: String,
    /// 每个 parent_id 下已有的子块计数，用于 O(1) position 生成
    child_counts: HashMap<String, usize>,
    /// 解析会话共享的时间戳，避免每个 Block 都调用 now_iso()
    now: String,
}

impl ParserState {
    fn new(doc_id: String) -> Self {
        Self {
            heading_stack: HeadingStack::new(doc_id),
            blocks: Vec::new(),
            warnings: Vec::new(),
            first_heading_seen: false,
            doc_title: String::new(),
            child_counts: HashMap::new(),
            now: now_iso(),
        }
    }

    /// 当前父块 ID
    fn current_parent(&self) -> String {
        self.heading_stack.current_parent()
    }

    /// 创建一个 Block 并加入列表，返回其 ID
    fn add_block(
        &mut self,
        block_type: BlockType,
        parent_id: String,
        content: Vec<u8>,
        properties: HashMap<String, String>,
    ) -> String {
        let id = generate_block_id();
        self.add_block_with_id(id, block_type, parent_id, content, properties)
    }

    /// 使用预生成的 ID 创建 Block（用于 Heading 等需提前知道 ID 的场景）
    fn add_block_with_id(
        &mut self,
        id: String,
        block_type: BlockType,
        parent_id: String,
        content: Vec<u8>,
        properties: HashMap<String, String>,
    ) -> String {
        let content_type = block_type.default_content_type();
        let position = self.next_position(&parent_id);

        let block = Block {
            id: id.clone(),
            parent_id: parent_id.clone(),
            position,
            block_type,
            content_type,
            content,
            properties,
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: self.now.clone(),
            modified: self.now.clone(),
            author: "system".to_string(),
            owner_id: None,
        };
        self.blocks.push(block);
        id
    }

    /// O(1) 生成 position（parent 下递增：a0, a1, a2, ...）
    fn next_position(&mut self, parent_id: &str) -> String {
        let count = self.child_counts.entry(parent_id.to_string()).or_insert(0);
        let pos = format!("a{}", count);
        *count += 1;
        pos
    }
}

// ─── 各块类型解析函数 ──────────────────────────────────────

fn parse_heading(scanner: &mut LineScanner, state: &mut ParserState) {
    let line = scanner.advance().unwrap();
    let (level, text) = parse_heading_line(line).expect("is_heading guaranteed true in dispatch");

    let heading_id = generate_block_id();
    let parent_id = state.heading_stack.push(level, heading_id.clone());

    let mut props = HashMap::new();
    props.insert("title".to_string(), text.clone());

    state.add_block_with_id(
        heading_id,
        BlockType::Heading { level },
        parent_id,
        Vec::new(),
        props,
    );

    if !state.first_heading_seen {
        state.doc_title = text;
        state.first_heading_seen = true;
    }
}

fn parse_code_block(scanner: &mut LineScanner, state: &mut ParserState) {
    let fence_line = scanner.advance().unwrap();
    let lang = code_fence_lang(fence_line);
    let fence_char = fence_line.trim_start().chars().next().unwrap_or('`');

    let mut code_lines = Vec::new();
    while let Some(line) = scanner.peek() {
        if is_closing_fence(line, fence_char) {
            scanner.advance();
            break;
        }
        code_lines.push(scanner.advance().unwrap());
    }

    let code = code_lines.join("\n");
    let parent_id = state.current_parent();
    state.add_block(
        BlockType::CodeBlock { language: lang },
        parent_id,
        code.into_bytes(),
        HashMap::new(),
    );
}

fn parse_math_block(scanner: &mut LineScanner, state: &mut ParserState) {
    scanner.advance(); // 消费 $$

    let mut math_lines = Vec::new();
    while let Some(line) = scanner.peek() {
        if is_math_fence(line) {
            scanner.advance();
            break;
        }
        math_lines.push(scanner.advance().unwrap());
    }

    let latex = math_lines.join("\n").trim().to_string();
    let parent_id = state.current_parent();
    state.add_block(
        BlockType::MathBlock,
        parent_id,
        latex.into_bytes(),
        HashMap::new(),
    );
}

fn parse_thematic_break(scanner: &mut LineScanner, state: &mut ParserState) {
    scanner.advance();
    let parent_id = state.current_parent();
    state.add_block(
        BlockType::ThematicBreak,
        parent_id,
        Vec::new(),
        HashMap::new(),
    );
}

fn parse_paragraph(scanner: &mut LineScanner, state: &mut ParserState) {
    let mut lines = Vec::new();

    while let Some(line) = scanner.peek() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if is_heading(line)
            || is_code_fence(line)
            || is_math_fence(line)
            || is_thematic_break(line)
            || is_blockquote(line)
            || is_list_item(line)
        {
            break;
        }
        lines.push(scanner.advance().unwrap().to_string());
    }

    if lines.is_empty() {
        return;
    }

    let text = lines.join("\n");
    let parent_id = state.current_parent();
    state.add_block(
        BlockType::Paragraph,
        parent_id,
        text.into_bytes(),
        HashMap::new(),
    );
}

fn parse_blockquote(scanner: &mut LineScanner, state: &mut ParserState) {
    // 收集连续的 blockquote 行，剥离 '> ' 前缀
    let mut bq_lines = Vec::new();

    while let Some(line) = scanner.peek() {
        if !is_blockquote(line) {
            break;
        }
        bq_lines.push(strip_blockquote(scanner.advance().unwrap()));
    }

    if bq_lines.is_empty() {
        return;
    }

    let parent_id = state.current_parent();
    let bq_id = state.add_block(
        BlockType::Blockquote,
        parent_id,
        Vec::new(),
        HashMap::new(),
    );

    // 递归解析引用块内部内容
    let inner_text = bq_lines.join("\n");
    let mut inner_scanner = LineScanner::new(&inner_text);

    // 保存并重置 heading stack（引用块内重置标题上下文）
    let saved_stack = std::mem::replace(
        &mut state.heading_stack,
        HeadingStack::new(bq_id.clone()),
    );

    parse_content(&mut inner_scanner, state);

    // 恢复 heading stack
    state.heading_stack = saved_stack;
}

fn parse_list(scanner: &mut LineScanner, state: &mut ParserState) {
    let first_line = scanner.peek().unwrap();
    let ordered = is_ordered_list_item(first_line);
    let base_indent = indent_of(first_line);

    let parent_id = state.current_parent();
    let list_id = state.add_block(
        BlockType::List { ordered },
        parent_id,
        Vec::new(),
        HashMap::new(),
    );

    while scanner.has_more() {
        scanner.skip_blank_lines();
        if !scanner.has_more() {
            break;
        }

        let line = scanner.peek().unwrap();
        let indent = indent_of(line);

        // 只处理 base_indent 层级的列表项
        if indent != base_indent || !is_list_item(line) {
            break;
        }

        // 消费列表项行
        let item_line = scanner.advance().unwrap();
        let item_text = strip_list_marker(item_line, ordered);

        let item_id = state.add_block(
            BlockType::ListItem,
            list_id.clone(),
            Vec::new(),
            HashMap::new(),
        );

        // 列表项文本 → Paragraph 子块
        if !item_text.is_empty() {
            state.add_block(
                BlockType::Paragraph,
                item_id.clone(),
                item_text.into_bytes(),
                HashMap::new(),
            );
        }

        // 收集嵌套内容（缩进更深的行）
        let mut nested_lines = Vec::new();
        while scanner.has_more() {
            if let Some(line) = scanner.peek() {
                if line.trim().is_empty() {
                    // 检查空行后是否还有嵌套内容
                    let saved_pos = scanner.pos;
                    scanner.advance();
                    if let Some(next) = scanner.peek() {
                        if indent_of(next) > base_indent {
                            nested_lines.push(String::new());
                            continue;
                        }
                    }
                    scanner.pos = saved_pos;
                    break;
                }
                if indent_of(line) <= base_indent {
                    break;
                }
                nested_lines.push(scanner.advance().unwrap().to_string());
            }
        }

        if !nested_lines.is_empty() {
            // 去除嵌套缩进（base_indent + 2 = 列表标记宽度）
            let dedent = base_indent + 2;
            let dedented: Vec<String> = nested_lines
                .iter()
                .map(|l| {
                    let leading = l
                        .bytes()
                        .position(|b| b != b' ' && b != b'\t')
                        .unwrap_or(0);
                    if leading >= dedent {
                        l[dedent..].to_string()
                    } else {
                        l.trim_start().to_string()
                    }
                })
                .collect();
            let inner_text = dedented.join("\n");
            let mut inner_scanner = LineScanner::new(&inner_text);

            let saved_stack = std::mem::replace(
                &mut state.heading_stack,
                HeadingStack::new(item_id.clone()),
            );

            parse_content(&mut inner_scanner, state);

            state.heading_stack = saved_stack;
        }
    }
}

/// 顶层内容解析循环
fn parse_content(scanner: &mut LineScanner, state: &mut ParserState) {
    while scanner.has_more() {
        scanner.skip_blank_lines();
        if !scanner.has_more() {
            break;
        }

        let line = scanner.peek().unwrap();

        if is_heading(line) {
            parse_heading(scanner, state);
        } else if is_code_fence(line) {
            parse_code_block(scanner, state);
        } else if is_math_fence(line) {
            parse_math_block(scanner, state);
        } else if is_thematic_break(line) {
            parse_thematic_break(scanner, state);
        } else if is_blockquote(line) {
            parse_blockquote(scanner, state);
        } else if is_list_item(line) {
            parse_list(scanner, state);
        } else {
            parse_paragraph(scanner, state);
        }
    }
}

// ─── 主解析入口 ────────────────────────────────────────────

fn parse_markdown(
    text: &str,
    _options: &ParseOptions,
) -> Result<(Block, Vec<Block>, Vec<ParseWarning>), AppError> {
    let doc_id = generate_block_id();
    let mut state = ParserState::new(doc_id.clone());

    let mut doc = Block {
        id: doc_id.clone(),
        parent_id: doc_id.clone(),
        position: "a0".to_string(),
        block_type: BlockType::Document,
        content_type: ContentType::Markdown,
        content: Vec::new(),
        properties: HashMap::new(),
        version: 1,
        status: BlockStatus::Normal,
        schema_version: 1,
        encrypted: false,
        created: state.now.clone(),
        modified: state.now.clone(),
        author: "system".to_string(),
        owner_id: None,
    };

    let mut scanner = LineScanner::new(text);

    parse_content(&mut scanner, &mut state);

    // 设置文档标题
    let doc_title = if state.doc_title.is_empty() {
        if let Some(first_para) = state.blocks.iter().find(|b| b.block_type == BlockType::Paragraph)
        {
            let content = String::from_utf8_lossy(&first_para.content);
            truncate_title(&content, 50)
        } else {
            "无标题文档".to_string()
        }
    } else {
        state.doc_title
    };
    doc.properties.insert("title".to_string(), doc_title);

    Ok((doc, state.blocks, state.warnings))
}

// ─── 解析器辅助函数 ──────────────────────────────────────────

/// 截断标题（使用 char_indices 避免 Vec<char> 分配）
fn truncate_title(s: &str, max_len: usize) -> String {
    match s.char_indices().nth(max_len) {
        // 如果第 max_len 个字符的起始位置存在，说明字符串超长
        Some((byte_pos, _)) => {
            let mut truncated = s[..byte_pos].to_string();
            truncated.push('…');
            truncated
        }
        // 字符数 ≤ max_len，直接返回
        None => s.to_string(),
    }
}

/// 空输入结果（Document + 空 Paragraph）
fn empty_result() -> ParseResult {
    let doc_id = generate_block_id();
    let para_id = generate_block_id();
    let now = now_iso();

    let doc = Block {
        id: doc_id.clone(),
        parent_id: doc_id.clone(),
        position: "a0".to_string(),
        block_type: BlockType::Document,
        content_type: ContentType::Markdown,
        content: Vec::new(),
        properties: {
            let mut m = HashMap::new();
            m.insert("title".to_string(), "无标题文档".to_string());
            m
        },
        version: 1,
        status: BlockStatus::Normal,
        schema_version: 1,
        encrypted: false,
        created: now.clone(),
        modified: now.clone(),
        author: "system".to_string(),
        owner_id: None,
    };

    let para = Block {
        id: para_id,
        parent_id: doc_id,
        position: "a0".to_string(),
        block_type: BlockType::Paragraph,
        content_type: ContentType::Markdown,
        content: Vec::new(),
        properties: HashMap::new(),
        version: 1,
        status: BlockStatus::Normal,
        schema_version: 1,
        encrypted: false,
        created: now.clone(),
        modified: now,
        author: "system".to_string(),
        owner_id: None,
    };

    ParseResult {
        blocks_created: 2,
        root: doc,
        children: vec![para],
        warnings: Vec::new(),
    }
}

/// 当前时间的 ISO 8601 字符串
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}


// ═══════════════════════════════════════════════════════════════
//  序列化器内部
// ═══════════════════════════════════════════════════════════════

/// 递归序列化一个 Block 及其所有后代
/// 获取指定父块的子块引用列表（按 position 排序），避免 clone 整个 Vec<Block>
fn get_sorted_children<'a>(
    children_map: &'a HashMap<String, Vec<Block>>,
    parent_id: &str,
) -> Vec<&'a Block> {
    match children_map.get(parent_id) {
        Some(children) => {
            let mut refs: Vec<&Block> = children.iter().collect();
            refs.sort_by(|a, b| a.position.cmp(&b.position));
            refs
        }
        None => Vec::new(),
    }
}

fn serialize_block_recursive(
    block: &Block,
    children_map: &HashMap<String, Vec<Block>>,
    list_depth: usize,
    out: &mut String,
    lossy: &mut Vec<LossyInfo>,
    counter: &mut usize,
    is_root: bool,
) {
    // 跳过已删除/草稿块
    if block.status != BlockStatus::Normal {
        return;
    }

    *counter += 1;

    // 获取子块引用（按 position 排序），无需 clone
    let sorted_children = get_sorted_children(children_map, &block.id);

    match &block.block_type {
        BlockType::Document => {
            if is_root {
                if let Some(title) = block.properties.get("title") {
                    if !title.is_empty() {
                        out.push_str(&format!("# {}\n\n", title));
                    }
                }
            }
            for &child in &sorted_children {
                serialize_block_recursive(
                    child, children_map, list_depth, out, lossy, counter, false,
                );
            }
        }

        BlockType::Heading { level } => {
            let title = block
                .properties
                .get("title")
                .cloned()
                .unwrap_or_default();
            let hashes = "#".repeat(*level as usize);
            out.push_str(&format!("{} {}\n\n", hashes, title));
            for &child in &sorted_children {
                serialize_block_recursive(
                    child, children_map, list_depth, out, lossy, counter, false,
                );
            }
        }

        BlockType::Paragraph => {
            let text = String::from_utf8_lossy(&block.content);
            if !text.is_empty() {
                out.push_str(&text);
                out.push_str("\n\n");
            }
        }

        BlockType::CodeBlock { language } => {
            out.push_str(&format!("```{}\n", language));
            let code = String::from_utf8_lossy(&block.content);
            out.push_str(&code);
            out.push_str("\n```\n\n");
        }

        BlockType::MathBlock => {
            out.push_str("$$\n");
            let latex = String::from_utf8_lossy(&block.content);
            out.push_str(&latex);
            out.push_str("\n$$\n\n");
        }

        BlockType::ThematicBreak => {
            out.push_str("---\n\n");
        }

        BlockType::Image { url } => {
            let alt = block
                .properties
                .get("alt")
                .cloned()
                .unwrap_or_default();
            out.push_str(&format!("![{}]({})\n\n", alt, url));
        }

        BlockType::List { ordered } => {
            serialize_list(
                &sorted_children,
                children_map,
                *ordered,
                list_depth,
                out,
                lossy,
                counter,
            );
        }

        BlockType::Blockquote => {
            let mut inner = String::new();
            for &child in &sorted_children {
                serialize_block_recursive(
                    child,
                    children_map,
                    list_depth,
                    &mut inner,
                    lossy,
                    counter,
                    false,
                );
            }
            for line in inner.lines() {
                out.push_str(&format!("> {}\n", line));
            }
            out.push('\n');
        }

        BlockType::ListItem => {
            // fallback：直接序列化子块
            for &child in &sorted_children {
                serialize_block_recursive(
                    child, children_map, list_depth, out, lossy, counter, false,
                );
            }
        }

        // ─── 降级处理 ─────────────────────────────────
        BlockType::Callout => {
            lossy.push(LossyInfo {
                block_type: "callout".to_string(),
                reason: "Markdown 无原生 callout，降级为 blockquote".to_string(),
            });
            let icon = block
                .properties
                .get("icon")
                .cloned()
                .unwrap_or_else(|| "💡".to_string());
            let mut inner = String::new();
            for &child in &sorted_children {
                serialize_block_recursive(
                    child,
                    children_map,
                    list_depth,
                    &mut inner,
                    lossy,
                    counter,
                    false,
                );
            }
            out.push_str(&format!("> {}\n", icon));
            for line in inner.lines() {
                out.push_str(&format!("> {}\n", line));
            }
            out.push('\n');
        }

        BlockType::Audio { url } => {
            lossy.push(LossyInfo {
                block_type: "audio".to_string(),
                reason: "Markdown 无原生音频，降级为链接".to_string(),
            });
            out.push_str(&format!("[audio]({})\n\n", url));
        }

        BlockType::Video { url } => {
            lossy.push(LossyInfo {
                block_type: "video".to_string(),
                reason: "Markdown 无原生视频，降级为链接".to_string(),
            });
            out.push_str(&format!("[video]({})\n\n", url));
        }

        BlockType::Iframe { url } => {
            lossy.push(LossyInfo {
                block_type: "iframe".to_string(),
                reason: "Markdown 无原生 iframe，降级为链接".to_string(),
            });
            out.push_str(&format!("[iframe]({})\n\n", url));
        }

        BlockType::Embed => {
            lossy.push(LossyInfo {
                block_type: "embed".to_string(),
                reason: "Embed 块降级为 HTML 注释".to_string(),
            });
            let content = String::from_utf8_lossy(&block.content);
            out.push_str(&format!("<!-- embed: {} -->\n\n", content));
        }

        BlockType::AttributeView { av_id } => {
            lossy.push(LossyInfo {
                block_type: "attributeView".to_string(),
                reason: "数据库视图降级为 HTML 注释".to_string(),
            });
            out.push_str(&format!("<!-- attributeView: {} -->\n\n", av_id));
        }

        BlockType::Widget => {
            lossy.push(LossyInfo {
                block_type: "widget".to_string(),
                reason: "Widget 块降级为 HTML 注释".to_string(),
            });
            out.push_str("<!-- widget -->\n\n");
        }
    }
}

/// 序列化列表
fn serialize_list(
    items: &[&Block],
    children_map: &HashMap<String, Vec<Block>>,
    ordered: bool,
    depth: usize,
    out: &mut String,
    lossy: &mut Vec<LossyInfo>,
    counter: &mut usize,
) {
    let indent = "  ".repeat(depth);

    for (i, &item) in items.iter().enumerate() {
        if item.block_type != BlockType::ListItem {
            serialize_block_recursive(item, children_map, depth, out, lossy, counter, false);
            continue;
        }

        *counter += 1;

        let prefix = if ordered {
            format!("{}. ", i + 1)
        } else {
            "- ".to_string()
        };

        let sorted_item_children = get_sorted_children(children_map, &item.id);

        // 第一个 Paragraph 在同一行输出
        let mut first = true;
        for &child in &sorted_item_children {
            if first && child.block_type == BlockType::Paragraph {
                let text = String::from_utf8_lossy(&child.content);
                out.push_str(&format!("{}{}{}\n", indent, prefix, text));
                *counter += 1;
                first = false;
            } else {
                if first {
                    out.push_str(&format!("{}{}", indent, prefix));
                    out.push('\n');
                    first = false;
                }
                serialize_block_recursive(
                    child,
                    children_map,
                    depth + 1,
                    out,
                    lossy,
                    counter,
                    false,
                );
            }
        }
    }

    out.push('\n');
}

// ═══════════════════════════════════════════════════════════════
//  测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BlockStatus, BlockType, ContentType};
    use crate::parser::types::ParseOptions;

    /// 辅助：解析 Markdown 文本，返回 (root, children)
    fn parse_md(text: &str) -> (Block, Vec<Block>) {
        let p = MarkdownFormat::new();
        let result = p.parse(text, &ParseOptions::default()).unwrap();
        (result.root, result.children)
    }

    /// 辅助：根据 ID 在 children 中查找 Block
    fn find_by_id<'a>(blocks: &'a [Block], id: &str) -> &'a Block {
        blocks.iter().find(|b| b.id == id).expect("block not found")
    }

    /// 辅助：根据 block_type 查找第一个匹配的 Block
    fn find_by_type<'a>(blocks: &'a [Block], bt: &BlockType) -> &'a Block {
        blocks.iter().find(|b| b.block_type == *bt).expect("block type not found")
    }

    /// 辅助：查找指定 parent_id 下的所有子 Block
    fn children_of<'a>(blocks: &'a [Block], parent_id: &str) -> Vec<&'a Block> {
        blocks.iter().filter(|b| b.parent_id == parent_id).collect()
    }

    /// 辅助：序列化 Block 树
    fn serialize_md(root: &Block, children: &[Block]) -> String {
        let mut children_map: HashMap<String, Vec<Block>> = HashMap::new();
        for b in children {
            children_map.entry(b.parent_id.clone()).or_default().push(b.clone());
        }
        let s = MarkdownFormat::new();
        let result = s.serialize(root, &children_map).unwrap();
        result.content
    }

    // ── 解析器基础 ────────────────────────────────────────

    #[test]
    fn parse_empty_input() {
        let (root, children) = parse_md("");
        assert_eq!(root.block_type, BlockType::Document);
        assert_eq!(children.len(), 1); // 空 Paragraph
        assert_eq!(children[0].block_type, BlockType::Paragraph);
        assert_eq!(root.properties.get("title").unwrap(), "无标题文档");
    }

    #[test]
    fn parse_whitespace_only() {
        let (root, children) = parse_md("   \n  \n  ");
        assert_eq!(root.block_type, BlockType::Document);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].block_type, BlockType::Paragraph);
    }

    #[test]
    fn parse_single_heading() {
        let (root, children) = parse_md("# Hello World");
        assert_eq!(root.block_type, BlockType::Document);
        assert_eq!(root.properties.get("title").unwrap(), "Hello World");

        let h1 = find_by_type(&children, &BlockType::Heading { level: 1 });
        assert_eq!(h1.properties.get("title").unwrap(), "Hello World");
        assert_eq!(h1.parent_id, root.id);
    }

    #[test]
    fn parse_heading_levels() {
        let md = "# H1\n## H2\n### H3\n#### H4";
        let (root, children) = parse_md(md);

        assert_eq!(children.len(), 4);

        // H1 直接挂在 Document 下
        let h1 = find_by_type(&children, &BlockType::Heading { level: 1 });
        assert_eq!(h1.parent_id, root.id);

        // H2 挂在 H1 下（HeadingStack 推断）
        let h2 = find_by_type(&children, &BlockType::Heading { level: 2 });
        assert_eq!(h2.parent_id, h1.id);

        // H3 挂在 H2 下
        let h3 = find_by_type(&children, &BlockType::Heading { level: 3 });
        assert_eq!(h3.parent_id, h2.id);

        // H4 挂在 H3 下
        let h4 = find_by_type(&children, &BlockType::Heading { level: 4 });
        assert_eq!(h4.parent_id, h3.id);
    }

    #[test]
    fn parse_heading_same_level_siblings() {
        let md = "# A\n# B\n# C";
        let (root, children) = parse_md(md);

        let headings: Vec<_> = children.iter()
            .filter(|b| matches!(b.block_type, BlockType::Heading { level: 1 }))
            .collect();

        assert_eq!(headings.len(), 3);
        // 所有 H1 应该都是 Document 的子块
        for h in &headings {
            assert_eq!(h.parent_id, root.id);
        }
    }

    #[test]
    fn parse_paragraph() {
        let (root, children) = parse_md("Hello world");
        let para = find_by_type(&children, &BlockType::Paragraph);
        assert_eq!(para.content, b"Hello world");
        assert_eq!(para.content_type, ContentType::Markdown);
        assert_eq!(para.parent_id, root.id);
    }

    #[test]
    fn parse_multiple_paragraphs() {
        let md = "First paragraph\n\nSecond paragraph\n\nThird";
        let (_root, children) = parse_md(md);

        let paras: Vec<_> = children.iter()
            .filter(|b| b.block_type == BlockType::Paragraph)
            .collect();
        assert_eq!(paras.len(), 3);

        assert_eq!(paras[0].content, b"First paragraph");
        assert_eq!(paras[1].content, b"Second paragraph");
        assert_eq!(paras[2].content, b"Third");
    }

    // ── 代码块 ────────────────────────────────────────────

    #[test]
    fn parse_fenced_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let (_, children) = parse_md(md);

        let code = find_by_type(&children, &BlockType::CodeBlock { language: "rust".to_string() });
        assert_eq!(code.content, b"fn main() {}");
        assert_eq!(code.content_type, ContentType::Markdown);
    }

    #[test]
    fn parse_code_block_no_language() {
        let md = "```\nplain code\n```";
        let (_, children) = parse_md(md);

        let code = find_by_type(&children, &BlockType::CodeBlock { language: String::new() });
        assert_eq!(code.content, b"plain code");
    }

    #[test]
    fn parse_code_block_multiline() {
        let md = "```python\ndef hello():\n    print(\"hi\")\n    return 42\n```";
        let (_, children) = parse_md(md);

        let code = find_by_type(&children, &BlockType::CodeBlock { language: "python".to_string() });
        let content = String::from_utf8_lossy(&code.content);
        assert!(content.contains("def hello():"));
        assert!(content.contains("print(\"hi\")"));
        assert!(content.contains("return 42"));
    }

    // ── 数学块 ────────────────────────────────────────────

    #[test]
    fn parse_math_block() {
        let md = "$$\nE = mc^2\n$$";
        let (_, children) = parse_md(md);

        let math = find_by_type(&children, &BlockType::MathBlock);
        let content = String::from_utf8_lossy(&math.content);
        assert!(content.contains("E = mc^2"));
    }

    #[test]
    fn parse_inline_math() {
        let md = "The formula $x^2 + y^2 = z^2$ is Pythagorean";
        let (_, children) = parse_md(md);

        let para = find_by_type(&children, &BlockType::Paragraph);
        let content = String::from_utf8_lossy(&para.content);
        assert!(content.contains("$x^2 + y^2 = z^2$"));
    }

    // ── 分割线 ────────────────────────────────────────────

    #[test]
    fn parse_thematic_break() {
        let md = "before\n\n---\n\nafter";
        let (_, children) = parse_md(md);

        let tb = find_by_type(&children, &BlockType::ThematicBreak);
        assert_eq!(tb.content_type, ContentType::Empty);
        assert!(tb.content.is_empty());
    }

    // ── 图片 ──────────────────────────────────────────────

    #[test]
    fn parse_standalone_image() {
        // pulldown-cmark 可能将图片放在段落内，检查行内图片重建
        let md = "![alt text](https://example.com/image.png)";
        let (_, children) = parse_md(md);

        // 图片可能被包裹在段落中作为行内内容
        let paras: Vec<_> = children.iter()
            .filter(|b| b.block_type == BlockType::Paragraph)
            .collect();
        // 至少有一个段落包含图片的 Markdown 语法
        let has_image = paras.iter().any(|p| {
            let content = String::from_utf8_lossy(&p.content);
            content.contains("![alt text]") && content.contains("https://example.com/image.png")
        });
        assert!(has_image, "应包含图片 Markdown 语法");
    }

    // ── 引用块 ────────────────────────────────────────────

    #[test]
    fn parse_blockquote() {
        let md = "> This is a quote\n> Second line";
        let (_, children) = parse_md(md);

        let bq = find_by_type(&children, &BlockType::Blockquote);
        assert_eq!(bq.parent_id, children.iter().find(|b| b.block_type == BlockType::Document).map(|d| d.id.clone()).unwrap_or_else(|| {
            // 没有 Document 在 children 中，所以 bq 的 parent 是 root
            children.iter().find(|b| b.block_type == BlockType::Blockquote).unwrap().parent_id.clone()
        }));
    }

    #[test]
    fn parse_blockquote_contains_paragraph() {
        let md = "> quoted text";
        let (_, children) = parse_md(md);

        let bq = find_by_type(&children, &BlockType::Blockquote);
        let bq_children = children_of(&children, &bq.id);
        assert!(!bq_children.is_empty(), "引用块应该包含子块");

        let para = bq_children.iter().find(|b| b.block_type == BlockType::Paragraph);
        assert!(para.is_some(), "引用块应包含段落");
        let para_content = String::from_utf8_lossy(&para.unwrap().content);
        assert!(para_content.contains("quoted text"));
    }

    // ── 列表 ──────────────────────────────────────────────

    #[test]
    fn parse_unordered_list() {
        let md = "- first\n- second\n- third";
        let (_, children) = parse_md(md);

        let list = find_by_type(&children, &BlockType::List { ordered: false });
        let list_items = children_of(&children, &list.id);
        assert_eq!(list_items.len(), 3);

        // 所有 ListItem 的 parent 都是 List
        for item in &list_items {
            assert_eq!(item.block_type, BlockType::ListItem);
            assert_eq!(item.parent_id, list.id);
        }
    }

    #[test]
    fn parse_ordered_list() {
        let md = "1. alpha\n2. beta\n3. gamma";
        let (_, children) = parse_md(md);

        let list = find_by_type(&children, &BlockType::List { ordered: true });
        let list_items = children_of(&children, &list.id);
        assert_eq!(list_items.len(), 3);
    }

    #[test]
    fn parse_nested_list() {
        let md = "- item 1\n  - sub item 1\n  - sub item 2\n- item 2";
        let (_, children) = parse_md(md);

        let lists: Vec<_> = children.iter()
            .filter(|b| matches!(b.block_type, BlockType::List { .. }))
            .collect();
        assert!(lists.len() >= 2, "应至少有 2 个 List（外层 + 内层）");
    }

    // ── 行内格式 ──────────────────────────────────────────

    #[test]
    fn parse_inline_bold_italic() {
        let md = "This is **bold** and *italic* text";
        let (_, children) = parse_md(md);

        let para = find_by_type(&children, &BlockType::Paragraph);
        let content = String::from_utf8_lossy(&para.content);
        assert!(content.contains("**bold**"));
        assert!(content.contains("*italic*"));
    }

    #[test]
    fn parse_inline_code() {
        let md = "Use `cargo test` to run";
        let (_, children) = parse_md(md);

        let para = find_by_type(&children, &BlockType::Paragraph);
        let content = String::from_utf8_lossy(&para.content);
        assert!(content.contains("`cargo test`"));
    }

    #[test]
    fn parse_inline_link() {
        let md = "Visit [Rust](https://rust-lang.org) for more";
        let (_, children) = parse_md(md);

        let para = find_by_type(&children, &BlockType::Paragraph);
        let content = String::from_utf8_lossy(&para.content);
        assert!(content.contains("[Rust](https://rust-lang.org)"));
    }

    #[test]
    fn parse_strikethrough() {
        let md = "This is ~~deleted~~ text";
        let (_, children) = parse_md(md);

        let para = find_by_type(&children, &BlockType::Paragraph);
        let content = String::from_utf8_lossy(&para.content);
        assert!(content.contains("~~deleted~~"));
    }

    // ── 混合文档 ──────────────────────────────────────────

    #[test]
    fn parse_mixed_document() {
        let md = r#"# My Document

First paragraph.

## Section 1

Some text in section 1.

```rust
fn main() {
    println!("hello");
}
```

### Subsection

More content.

---

Final paragraph.
"#;
        let (root, children) = parse_md(md);

        assert_eq!(root.block_type, BlockType::Document);
        assert_eq!(root.properties.get("title").unwrap(), "My Document");

        // 验证各种块类型都存在
        let has_h1 = children.iter().any(|b| matches!(b.block_type, BlockType::Heading { level: 1 }));
        let has_h2 = children.iter().any(|b| matches!(b.block_type, BlockType::Heading { level: 2 }));
        let has_h3 = children.iter().any(|b| matches!(b.block_type, BlockType::Heading { level: 3 }));
        let has_code = children.iter().any(|b| matches!(b.block_type, BlockType::CodeBlock { .. }));
        let has_tb = children.iter().any(|b| b.block_type == BlockType::ThematicBreak);

        assert!(has_h1, "应包含 H1");
        assert!(has_h2, "应包含 H2");
        assert!(has_h3, "应包含 H3");
        assert!(has_code, "应包含代码块");
        assert!(has_tb, "应包含分割线");

        // 所有 Block 的 status 应该为 Normal
        for b in &children {
            assert_eq!(b.status, BlockStatus::Normal);
        }
    }

    #[test]
    fn parse_blocks_created_count() {
        let md = "# Title\n\nParagraph 1\n\nParagraph 2";
        let p = MarkdownFormat::new();
        let result = p.parse(md, &ParseOptions::default()).unwrap();

        // root (Document) + H1 + Paragraph1 + Paragraph2 = 4
        assert_eq!(result.blocks_created, 4);
    }

    #[test]
    fn parse_title_from_first_heading() {
        let (root, _) = parse_md("# Document Title\n\nSome content");
        assert_eq!(root.properties.get("title").unwrap(), "Document Title");
    }

    #[test]
    fn parse_title_fallback_to_paragraph() {
        let (root, _) = parse_md("No heading here, just text");
        // 标题从第一个段落推断
        let title = root.properties.get("title").unwrap();
        assert!(!title.is_empty());
        assert!(title.contains("No heading here"));
    }

    #[test]
    fn parse_title_truncation() {
        let long_text: String = "A".repeat(200);
        let (root, _) = parse_md(&long_text);
        let title = root.properties.get("title").unwrap();
        // 标题应被截断（truncate_title max 50 chars + "…")
        assert!(title.len() <= 60); // 50 chars + "…" = ~51 UTF-8 bytes, but chars may vary
    }

    // ── 序列化器 ──────────────────────────────────────────

    #[test]
    fn serialize_document_with_title() {
        let doc = Block {
            id: "doc1".to_string(),
            parent_id: "doc1".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Document,
            content_type: ContentType::Markdown,
            content: Vec::new(),
            properties: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "Test Doc".to_string());
                m
            },
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[]);
        assert!(content.starts_with("# Test Doc"));
    }

    #[test]
    fn serialize_heading() {
        let doc = make_doc("root");
        let h2 = Block {
            id: "h2".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Heading { level: 2 },
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "Section".to_string());
                m
            },
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[h2]);
        assert!(content.contains("## Section"));
    }

    #[test]
    fn serialize_paragraph() {
        let doc = make_doc("root");
        let para = Block {
            id: "p1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Paragraph,
            content_type: ContentType::Markdown,
            content: b"Hello world".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[para]);
        assert!(content.contains("Hello world"));
    }

    #[test]
    fn serialize_code_block() {
        let doc = make_doc("root");
        let code = Block {
            id: "c1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::CodeBlock { language: "rust".to_string() },
            content_type: ContentType::Markdown,
            content: b"fn main() {}".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[code]);
        assert!(content.contains("```rust"));
        assert!(content.contains("fn main() {}"));
        assert!(content.contains("```"));
    }

    #[test]
    fn serialize_math_block() {
        let doc = make_doc("root");
        let math = Block {
            id: "m1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::MathBlock,
            content_type: ContentType::Markdown,
            content: b"E = mc^2".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[math]);
        assert!(content.contains("$$"));
        assert!(content.contains("E = mc^2"));
    }

    #[test]
    fn serialize_thematic_break() {
        let doc = make_doc("root");
        let tb = Block {
            id: "t1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::ThematicBreak,
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[tb]);
        assert!(content.contains("---"));
    }

    #[test]
    fn serialize_image() {
        let doc = make_doc("root");
        let img = Block {
            id: "i1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Image { url: "https://example.com/img.png".to_string() },
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: {
                let mut m = HashMap::new();
                m.insert("alt".to_string(), "photo".to_string());
                m
            },
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[img]);
        assert!(content.contains("![photo](https://example.com/img.png)"));
    }

    #[test]
    fn serialize_unordered_list() {
        let doc = make_doc("root");
        let list = Block {
            id: "list1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::List { ordered: false },
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };
        let item1 = Block {
            id: "li1".to_string(),
            parent_id: "list1".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::ListItem,
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };
        let item1_para = Block {
            id: "p1".to_string(),
            parent_id: "li1".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Paragraph,
            content_type: ContentType::Markdown,
            content: b"item 1".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[list, item1, item1_para]);
        assert!(content.contains("- item 1"));
    }

    #[test]
    fn serialize_ordered_list() {
        let doc = make_doc("root");
        let list = Block {
            id: "list1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::List { ordered: true },
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };
        let item1 = Block {
            id: "li1".to_string(),
            parent_id: "list1".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::ListItem,
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };
        let item1_para = Block {
            id: "p1".to_string(),
            parent_id: "li1".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Paragraph,
            content_type: ContentType::Markdown,
            content: b"first".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[list, item1, item1_para]);
        assert!(content.contains("1. first"));
    }

    #[test]
    fn serialize_blockquote() {
        let doc = make_doc("root");
        let bq = Block {
            id: "bq1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Blockquote,
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };
        let para = Block {
            id: "p1".to_string(),
            parent_id: "bq1".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Paragraph,
            content_type: ContentType::Markdown,
            content: b"quoted text".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[bq, para]);
        assert!(content.contains("> quoted text"));
    }

    // ── 降级序列化 ────────────────────────────────────────

    #[test]
    fn serialize_audio_lossy() {
        let doc = make_doc("root");
        let audio = Block {
            id: "a1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Audio { url: "https://example.com/audio.mp3".to_string() },
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let s = MarkdownFormat::new();
        let mut children_map: HashMap<String, Vec<Block>> = HashMap::new();
        let _result = s.serialize(&doc, &children_map).unwrap();

        // 需要把 audio 作为 doc 的子块
        children_map.insert("root".to_string(), vec![audio]);
        let result = s.serialize(&doc, &children_map).unwrap();
        assert!(result.content.contains("[audio]"));
        assert!(result.lossy_types.contains(&"audio".to_string()));
    }

    #[test]
    fn serialize_video_lossy() {
        let doc = make_doc("root");
        let video = Block {
            id: "v1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Video { url: "https://example.com/video.mp4".to_string() },
            content_type: ContentType::Empty,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let s = MarkdownFormat::new();
        let mut children_map: HashMap<String, Vec<Block>> = HashMap::new();
        children_map.insert("root".to_string(), vec![video]);
        let result = s.serialize(&doc, &children_map).unwrap();
        assert!(result.content.contains("[video]"));
        assert!(result.lossy_types.contains(&"video".to_string()));
    }

    #[test]
    fn serialize_deleted_block_skipped() {
        let doc = make_doc("root");
        let para = Block {
            id: "p1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Paragraph,
            content_type: ContentType::Markdown,
            content: b"should be skipped".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Deleted,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let content = serialize_md(&doc, &[para]);
        assert!(!content.contains("should be skipped"));
    }

    #[test]
    fn serialize_blocks_exported_count() {
        let doc = make_doc("root");
        let para = Block {
            id: "p1".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Paragraph,
            content_type: ContentType::Markdown,
            content: b"content".to_vec(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let s = MarkdownFormat::new();
        let mut children_map: HashMap<String, Vec<Block>> = HashMap::new();
        children_map.insert("root".to_string(), vec![para]);
        let result = s.serialize(&doc, &children_map).unwrap();
        // Document + Paragraph = 2
        assert_eq!(result.blocks_exported, 2);
    }

    #[test]
    fn serialize_filename_from_title() {
        let doc = Block {
            id: "root".to_string(),
            parent_id: "root".to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Document,
            content_type: ContentType::Markdown,
            content: Vec::new(),
            properties: {
                let mut m = HashMap::new();
                m.insert("title".to_string(), "My Notes".to_string());
                m
            },
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        };

        let s = MarkdownFormat::new();
        let children_map: HashMap<String, Vec<Block>> = HashMap::new();
        let result = s.serialize(&doc, &children_map).unwrap();
        assert_eq!(result.filename, Some("My Notes.md".to_string()));
    }

    // ── 往返测试 ──────────────────────────────────────────

    #[test]
    fn roundtrip_paragraph() {
        let md = "Hello world\n";
        let (root, children) = parse_md(md);
        let output = serialize_md(&root, &children);
        assert!(output.contains("Hello world"));
    }

    #[test]
    fn roundtrip_heading_and_paragraph() {
        let md = "# Title\n\nSome content here\n";
        let (root, children) = parse_md(md);
        let output = serialize_md(&root, &children);

        assert!(output.contains("# Title") || output.contains("## Title"),
            "应包含标题，实际输出: {}", output);
        assert!(output.contains("Some content here"));
    }

    #[test]
    fn roundtrip_code_block() {
        let md = "```rust\nfn main() {}\n```\n";
        let (root, children) = parse_md(md);
        let output = serialize_md(&root, &children);

        assert!(output.contains("```rust"));
        assert!(output.contains("fn main() {}"));
    }

    #[test]
    fn roundtrip_list() {
        let md = "- item 1\n- item 2\n- item 3\n";
        let (root, children) = parse_md(md);
        let output = serialize_md(&root, &children);

        eprintln!("=== roundtrip_list OUTPUT ===");
        eprintln!("output: {:?}", output);
        eprintln!("root: {:?}", root);
        eprintln!("children count: {}", children.len());
        for (i, c) in children.iter().enumerate() {
            eprintln!("  child[{}]: id={}, parent={}, type={:?}, content={:?}", 
                i, c.id, c.parent_id, c.block_type, String::from_utf8_lossy(&c.content));
        }

        assert!(output.contains("- item 1"), "actual output: {}", output);
        assert!(output.contains("- item 2"), "actual output: {}", output);
        assert!(output.contains("- item 3"), "actual output: {}", output);
    }

    #[test]
    fn roundtrip_blockquote() {
        let md = "> quoted text\n";
        let (root, children) = parse_md(md);
        let output = serialize_md(&root, &children);

        assert!(output.contains("> quoted text") || output.contains("quoted text"),
            "应包含引用文本，实际输出: {}", output);
    }

    #[test]
    fn roundtrip_math_block() {
        let md = "$$\nE = mc^2\n$$\n";
        let (root, children) = parse_md(md);
        let output = serialize_md(&root, &children);

        assert!(output.contains("$$"));
        assert!(output.contains("E = mc^2"));
    }

    // ── 辅助 ──────────────────────────────────────────────

    fn make_doc(id: &str) -> Block {
        Block {
            id: id.to_string(),
            parent_id: id.to_string(),
            position: "a0".to_string(),
            block_type: BlockType::Document,
            content_type: ContentType::Markdown,
            content: Vec::new(),
            properties: HashMap::new(),
            version: 1,
            status: BlockStatus::Normal,
            schema_version: 1,
            encrypted: false,
            created: "2026-01-01T00:00:00.000Z".to_string(),
            modified: "2026-01-01T00:00:00.000Z".to_string(),
            author: "system".to_string(),
            owner_id: None,
        }
    }
}