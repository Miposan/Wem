//! Wem CLI — 内核操作的命令行接口
//!
//! 直接调用 service 层，不经过 HTTP。
//! 两种模式：
//!   1. 批量模式: wem-cli "cmd arg" "cmd arg" ...   — 执行完退出
//!   2. REPL模式: wem-cli                          — 无参数时进入交互
//!
//! 用法:
//!   wem-cli "create-doc 我的文档" "list-docs"
//!   wem-cli                    # 进入交互 REPL

use std::collections::HashMap;
use std::io::{self, Write};

use wem_kernel::api::request::*;
use wem_kernel::api::response::DocumentContentResult;
use wem_kernel::block_system::model::{Block, BlockType};
use wem_kernel::repo;
use wem_kernel::block_system::service::{block, document, oplog};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = repo::init_memory_db();

    if args.is_empty() {
        run_repl(&db);
    } else {
        run_batch(&db, &args);
    }
}

// ─── 批量模式 ──────────────────────────────────────────────────

fn run_batch(db: &repo::Db, commands: &[String]) {
    let mut ctx = CmdContext::new();

    for cmd in commands {
        println!("> {}", cmd);
        let result = execute_command(db, cmd.trim(), &mut ctx);
        match result {
            Ok(()) => {}
            Err(e) => {
                println!("错误: {}", e);
                std::process::exit(1);
            }
        }
    }
}

// ─── REPL 模式 ─────────────────────────────────────────────────

fn run_repl(db: &repo::Db) {
    println!("Wem CLI — 输入 help 查看命令，quit 退出");
    println!();

    let mut ctx = CmdContext::new();

    loop {
        print!("> ");
        io::stdout().flush().unwrap();

        let mut line = String::new();
        if io::stdin().read_line(&mut line).unwrap() == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "quit" || line == "exit" {
            break;
        }

        match execute_command(db, line, &mut ctx) {
            Ok(()) => {}
            Err(e) => println!("错误: {}", e),
        }
    }
}

// ─── 共享上下文 ────────────────────────────────────────────────

struct CmdContext {
    last_doc_id: Option<String>,
    /// 最近输出的 ID 栈：create-doc / add-block 会 push，$1 $2 $3 引用
    id_stack: Vec<String>,
}

impl CmdContext {
    fn new() -> Self {
        Self { last_doc_id: None, id_stack: Vec::new() }
    }

    fn push_id(&mut self, id: String) {
        self.id_stack.push(id);
    }

    /// 替换命令参数中的 $doc / $1 / $2 / ... 变量
    fn subst_vars(&self, s: &str) -> String {
        let mut out = s.to_string();
        out = out.replace("$root", wem_kernel::block_system::model::ROOT_ID);
        if let Some(doc_id) = &self.last_doc_id {
            out = out.replace("$doc", doc_id);
        }
        // $1 = id_stack[last], $2 = id_stack[last-1], ...
        let last = self.id_stack.len();
        if last > 0 {
            for i in 1..=last {
                let var = format!("${}", i);
                let idx = last - i;
                out = out.replace(&var, &self.id_stack[idx]);
            }
        }
        out
    }
}

// ─── 命令分发 ──────────────────────────────────────────────────

/// 尝试将参数解析为文档 ID 或文档名称
///
/// - 以 `@` 开头：在 last_doc_id 或 root 下按名称查找
/// - 否则原样返回（可能是 ID 或其他参数）
fn resolve_doc_arg(db: &repo::Db, arg: &str, ctx: &CmdContext) -> Result<String, String> {
    if arg.starts_with('@') {
        let name = &arg[1..];
        let parent = ctx.last_doc_id.as_deref().unwrap_or(wem_kernel::block_system::model::ROOT_ID);
        let doc = document::find_doc_by_name(db, parent, name)
            .map_err(|e| e.to_string())?;
        match doc {
            Some(d) => Ok(d.id),
            None => {
                // 回退到 root 下查找
                if parent != wem_kernel::block_system::model::ROOT_ID {
                    let doc2 = document::find_doc_by_name(db, wem_kernel::block_system::model::ROOT_ID, name)
                        .map_err(|e| e.to_string())?;
                    doc2.map(|d| d.id).ok_or_else(|| format!("文档 \"{}\" 不存在", name))
                } else {
                    Err(format!("文档 \"{}\" 不存在", name))
                }
            }
        }
    } else {
        Ok(arg.to_string())
    }
}

fn execute_command(db: &repo::Db, line: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let expanded = ctx.subst_vars(line);
    let (cmd, args) = split_cmd(&expanded);

    match cmd {
        "create-doc" => cmd_create_doc(db, args, ctx),
        "list-docs" => cmd_list_docs(db),
        "get-doc" | "tree" => cmd_get_doc(db, args, ctx),
        "add-block" => cmd_add_block(db, args, ctx),
        "update-block" => cmd_update_block(db, args),
        "delete-block" => cmd_delete_block(db, args),
        "delete-tree" => cmd_delete_tree(db, args),
        "restore-block" => cmd_restore_block(db, args),
        "move-block" => cmd_move_block(db, args),
        "move-heading" => cmd_move_heading(db, args),
        "move-doc" => cmd_move_doc(db, args),
        "split" => cmd_split(db, args),
        "merge" => cmd_merge(db, args),
        "undo" => cmd_undo(db, args, ctx),
        "redo" => cmd_redo(db, args, ctx),
        "history" => cmd_history(db, args, ctx),
        "export" => cmd_export(db, args, ctx),
        "import" => cmd_import(db, args),
        "echo" => { println!("{}", args); Ok(()) }
        "help" => { print_help(); Ok(()) }
        _ => Err(format!("未知命令: {}，输入 help 查看帮助", cmd)),
    }
}

fn split_cmd(line: &str) -> (&str, &str) {
    match line.find(' ') {
        Some(i) => (&line[..i], line[i + 1..].trim()),
        None => (line, ""),
    }
}

fn print_help() {
    println!("命令列表:");
    println!("  create-doc <title>                      创建文档");
    println!("  list-docs                               列出根文档");
    println!("  get-doc <id|@名称>                      查看文档内容");
    println!("  tree <id|@名称>                         同 get-doc");
    println!("  add-block <parent> <type> [content]     添加块");
    println!("  update-block <id> <content>             更新块内容");
    println!("  delete-block <id>                       删除块");
    println!("  restore-block <id>                      恢复块");
    println!("  move-block <id> [after:<id>]            移动块");
    println!("  move-heading <id> [after:<id>]          移动 heading 子树");
    println!("  move-doc <id> [parent:<id>] [after:<id>] 移动文档");
    println!("  split <id> <before>|<after>             拆分块");
    println!("  merge <id>                              合并到前一个兄弟");
    println!("  undo <doc_id|@名称>                     撤销");
    println!("  redo <doc_id|@名称>                     重做");
    println!("  history <doc_id|@名称>                  查看操作历史");
    println!("  export <doc_id|@名称>                   导出 Markdown");
    println!("  import <markdown-text>                  导入 Markdown");
    println!("  echo <text>                             输出文本（脚本标注用）");
    println!("  help                                    显示帮助");
    println!("  quit                                    退出");
    println!();
    println!("变量替换:");
    println!("  $doc   最近创建的文档 ID");
    println!("  $root  全局根块 ID");
    println!("  $1/$2/... 最近 push 的 ID（$1=最新，$2=次新）");
}

// ─── 辅助函数 ──────────────────────────────────────────────────

fn content_str(block: &Block) -> String {
    String::from_utf8_lossy(&block.content).to_string()
}

fn block_type_short(bt: &BlockType) -> String {
    match bt {
        BlockType::Document => "doc".into(),
        BlockType::Heading { level } => format!("h{}", level),
        BlockType::Paragraph => "p".into(),
        BlockType::CodeBlock { language } => format!("code({})", language),
        BlockType::Blockquote => "quote".into(),
        BlockType::List { ordered: true } => "ol".into(),
        BlockType::List { ordered: false } => "ul".into(),
        BlockType::ListItem => "li".into(),
        _ => format!("{:?}", bt),
    }
}

fn print_doc_tree(result: &DocumentContentResult) {
    println!("[doc] {} \"{}\"", result.document.id, content_str(&result.document));
    for node in &result.blocks {
        print_tree_node(node, 1);
    }
}

fn print_tree_node(node: &wem_kernel::api::response::BlockNode, indent: usize) {
    let prefix = "  ".repeat(indent);
    let b = &node.block;
    let content = content_str(b);
    let preview = if content.len() > 50 { &content[..50] } else { &content };
    println!("{}[{}] {} \"{}\"", prefix, block_type_short(&b.block_type), b.id, preview);
    for child in &node.children {
        print_tree_node(child, indent + 1);
    }
}

fn parse_kv(args: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in args.split_whitespace() {
        if let Some((k, v)) = part.split_once(':') {
            map.insert(k.to_string(), v.to_string());
        }
    }
    map
}

fn parse_block_type(s: &str) -> Result<BlockType, String> {
    match s {
        "paragraph" | "p" => Ok(BlockType::Paragraph),
        "doc" | "document" => Ok(BlockType::Document),
        "blockquote" | "quote" => Ok(BlockType::Blockquote),
        s if s.starts_with("heading") || s.starts_with("h") => {
            let level: u8 = s.trim_start_matches("heading").trim_start_matches('h').parse()
                .map_err(|_| format!("无效 heading level: {}", s))?;
            Ok(BlockType::Heading { level })
        }
        s if s.starts_with("code") => {
            let lang = s.trim_start_matches("code").trim_start_matches('(').trim_end_matches(')').to_string();
            Ok(BlockType::CodeBlock { language: if lang.is_empty() { "text".into() } else { lang } })
        }
        _ => Err(format!("未知 block type: {}", s)),
    }
}

// ─── 命令实现 ──────────────────────────────────────────────────

fn cmd_create_doc(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let title = if args.is_empty() { "Untitled" } else { args };
    let doc = document::create_document(db, title.to_string(), None, None, None)
        .map_err(|e| e.to_string())?;
    ctx.last_doc_id = Some(doc.id.clone());
    ctx.push_id(doc.id.clone());
    println!("{}", doc.id);
    Ok(())
}

fn cmd_list_docs(db: &repo::Db) -> Result<(), String> {
    let docs = document::list_root_documents(db).map_err(|e| e.to_string())?;
    if docs.is_empty() {
        println!("(无文档)");
        return Ok(());
    }
    for doc in &docs {
        println!("{} \"{}\"", doc.id, content_str(doc));
    }
    Ok(())
}

fn cmd_get_doc(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let doc_id = if args.is_empty() {
        ctx.last_doc_id.as_deref().ok_or("需要 doc_id 或 @名称")?.to_string()
    } else {
        resolve_doc_arg(db, args, ctx)?
    };
    let result = document::get_document_content(db, &doc_id).map_err(|e| e.to_string())?;
    print_doc_tree(&result);
    ctx.last_doc_id = Some(doc_id);
    Ok(())
}

fn cmd_add_block(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let parts: Vec<&str> = args.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err("用法: add-block <parent_id> <type> [content]".to_string());
    }
    let parent_id = parts[0];
    let block_type = parse_block_type(parts[1])?;
    let content = parts.get(2).unwrap_or(&"").to_string();

    let block = block::create_block(db, CreateBlockReq {
        editor_id: Some("cli".to_string()),
        parent_id: parent_id.to_string(),
        block_type,
        content,
        properties: HashMap::new(),
        after_id: None,
    }).map_err(|e| e.to_string())?;

    ctx.push_id(block.id.clone());
    println!("{}", block.id);
    Ok(())
}

fn cmd_update_block(db: &repo::Db, args: &str) -> Result<(), String> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return Err("用法: update-block <id> <content>".to_string());
    }
    let id = parts[0];
    let content = parts[1].to_string();

    let block = block::update_block(db, id, UpdateBlockReq {
        editor_id: Some("cli".to_string()),
        id: id.to_string(),
        content: Some(content),
        block_type: None,
        properties: None,
        properties_mode: PropertiesMode::Merge,
    }).map_err(|e| e.to_string())?;

    println!("ok v{}", block.version);
    Ok(())
}

fn cmd_delete_block(db: &repo::Db, args: &str) -> Result<(), String> {
    let id = args.trim();
    if id.is_empty() { return Err("用法: delete-block <id>".to_string()); }
    let result = block::delete_block(db, id, Some("cli".to_string())).map_err(|e| e.to_string())?;
    println!("deleted {} (children promoted)", result.id);
    Ok(())
}

fn cmd_delete_tree(db: &repo::Db, args: &str) -> Result<(), String> {
    let id = args.trim();
    if id.is_empty() { return Err("用法: delete-tree <id>".to_string()); }
    let result = block::delete_tree(db, id, Some("cli".to_string())).map_err(|e| e.to_string())?;
    println!("deleted {} (cascade: {})", result.id, result.cascade_count);
    Ok(())
}

fn cmd_restore_block(db: &repo::Db, args: &str) -> Result<(), String> {
    let id = args.trim();
    if id.is_empty() { return Err("用法: restore-block <id>".to_string()); }
    let result = block::restore_block(db, id, Some("cli".to_string())).map_err(|e| e.to_string())?;
    println!("restored {} (cascade: {})", result.id, result.cascade_count);
    Ok(())
}

fn cmd_move_block(db: &repo::Db, args: &str) -> Result<(), String> {
    let kv = parse_kv(args);
    let id = args.split_whitespace().next().ok_or("用法: move-block <id> [after:<id>] [parent:<id>]")?;
    let block = block::move_block(db, id, MoveBlockReq {
        editor_id: Some("cli".to_string()),
        id: id.to_string(),
        target_parent_id: kv.get("parent").cloned(),
        before_id: kv.get("before").cloned(),
        after_id: kv.get("after").cloned(),
    }).map_err(|e| e.to_string())?;
    println!("moved {} -> parent={}", block.id, block.parent_id);
    Ok(())
}

fn cmd_move_heading(db: &repo::Db, args: &str) -> Result<(), String> {
    let kv = parse_kv(args);
    let id = args.split_whitespace().next().ok_or("用法: move-heading <id> [after:<id>]")?;
    let block = block::move_heading_tree(db, MoveHeadingTreeReq {
        editor_id: Some("cli".to_string()),
        id: id.to_string(),
        before_id: kv.get("before").cloned(),
        after_id: kv.get("after").cloned(),
    }).map_err(|e| e.to_string())?;
    println!("heading-tree moved {}", block.id);
    Ok(())
}

fn cmd_move_doc(db: &repo::Db, args: &str) -> Result<(), String> {
    let kv = parse_kv(args);
    let id = args.split_whitespace().next().ok_or("用法: move-doc <id> [parent:<id>] [after:<id>]")?;
    let block = document::move_document_tree(db, MoveDocumentTreeReq {
        editor_id: Some("cli".to_string()),
        id: id.to_string(),
        target_parent_id: kv.get("parent").cloned(),
        before_id: kv.get("before").cloned(),
        after_id: kv.get("after").cloned(),
    }).map_err(|e| e.to_string())?;
    println!("doc moved {} -> parent={}", block.id, block.parent_id);
    Ok(())
}

fn cmd_split(db: &repo::Db, args: &str) -> Result<(), String> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    if parts.len() < 2 { return Err("用法: split <id> <before>|<after>".to_string()); }
    let pipe: Vec<&str> = parts[1].splitn(2, '|').collect();
    if pipe.len() < 2 { return Err("用法: split <id> <before>|<after>".to_string()); }
    let result = block::split_block(db, parts[0], SplitReq {
        editor_id: Some("cli".to_string()),
        id: parts[0].to_string(),
        content_before: pipe[0].to_string(),
        content_after: pipe[1].to_string(),
        new_block_type: None,
        nest_under_parent: None,
    }).map_err(|e| e.to_string())?;
    println!("split {} -> new {}", result.updated_block.id, result.new_block.id);
    Ok(())
}

fn cmd_merge(db: &repo::Db, args: &str) -> Result<(), String> {
    let id = args.trim();
    if id.is_empty() { return Err("用法: merge <id>".to_string()); }
    let result = block::merge_block(db, id, MergeReq {
        editor_id: Some("cli".to_string()),
        id: id.to_string(),
    }).map_err(|e| e.to_string())?;
    println!("merged -> {} (deleted {})", result.merged_block.id, result.deleted_block_id);
    Ok(())
}

fn cmd_undo(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let doc_id = if args.is_empty() {
        ctx.last_doc_id.as_deref().ok_or("需要 doc_id 或 @名称")?.to_string()
    } else {
        resolve_doc_arg(db, args, ctx)?
    };
    let result = oplog::undo(db, &doc_id).map_err(|e| e.to_string())?;
    println!("undo {} [{}] blocks:{:?}", result.operation_id, result.action, result.affected_block_ids);
    ctx.last_doc_id = Some(doc_id);
    Ok(())
}

fn cmd_redo(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let doc_id = if args.is_empty() {
        ctx.last_doc_id.as_deref().ok_or("需要 doc_id 或 @名称")?.to_string()
    } else {
        resolve_doc_arg(db, args, ctx)?
    };
    let result = oplog::redo(db, &doc_id).map_err(|e| e.to_string())?;
    println!("redo {} [{}] blocks:{:?}", result.operation_id, result.action, result.affected_block_ids);
    ctx.last_doc_id = Some(doc_id);
    Ok(())
}

fn cmd_history(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let doc_id = if args.is_empty() {
        ctx.last_doc_id.as_deref().ok_or("需要 doc_id 或 @名称")?.to_string()
    } else {
        resolve_doc_arg(db, args, ctx)?
    };
    let entries = oplog::get_history(db, &doc_id, 20, 0).map_err(|e| e.to_string())?;
    if entries.is_empty() { println!("(无历史记录)"); return Ok(()); }
    for entry in &entries {
        let undone = if entry.undone { " [已撤销]" } else { "" };
        println!("{} [{}] {} changes:{}{}", entry.operation_id, entry.action, entry.timestamp, entry.changes.len(), undone);
    }
    ctx.last_doc_id = Some(doc_id);
    Ok(())
}

fn cmd_export(db: &repo::Db, args: &str, ctx: &mut CmdContext) -> Result<(), String> {
    let doc_id = if args.is_empty() {
        ctx.last_doc_id.as_deref().ok_or("需要 doc_id 或 @名称")?.to_string()
    } else {
        resolve_doc_arg(db, args, ctx)?
    };
    let result = document::export_text(db, &doc_id, "markdown").map_err(|e| e.to_string())?;
    println!("{}", result.content);
    ctx.last_doc_id = Some(doc_id);
    Ok(())
}

fn cmd_import(db: &repo::Db, args: &str) -> Result<(), String> {
    if args.is_empty() { return Err("用法: import <markdown-text>".to_string()); }
    let result = document::import_text(db, ImportTextReq {
        editor_id: Some("cli".to_string()),
        format: "markdown".to_string(),
        content: args.to_string(),
        parent_id: None,
        after_id: None,
        title: None,
    }).map_err(|e| e.to_string())?;
    println!("imported {} ({} blocks)", result.root.id, result.blocks_imported);
    Ok(())
}
