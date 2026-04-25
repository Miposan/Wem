//! Fractional Index 位置计算
//!
//! 基于 base-62 字符集的分数索引算法 + 数据库感知的位置计算。
//!
//! 分两层：
//! - **纯算法层**：`generate_first/after/before/between` — 零依赖的字符串位置生成
//! - **DB 感知层**：`calculate_insert_position/calculate_move_position` — 查询兄弟节点后调用纯算法
//!
//! 核心思想：
//! - 每个 Block 有一个 position 字符串，按字典序决定兄弟节点间的顺序
//! - 插入新 Block 时，生成一个介于目标位置之间的字符串
//! - 不需要重新编号其他节点
//!
//! 示例排序：
//! ```ignore
//! "a0" < "a1" < "a1V" < "a2" < "b0"
//! ```

use crate::repo::block_repo as repo;
use crate::error::AppError;

// ─── Base-62 字符集 ─────────────────────────────────────────────

/// Base-62 字符集（按 ASCII 码排列，自然字典序）
const CHARS: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
    'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
    's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

/// 中间字符（索引 30，约在 base-62 的正中间）
const MID_CHAR: char = 'U';

fn char_index(c: char) -> Option<usize> {
    CHARS.iter().position(|&x| x == c)
}

fn char_at(i: usize) -> char {
    CHARS[i]
}

// ─── 纯算法：位置生成 ──────────────────────────────────────────

/// 生成第一个 position（`"a0"`）
pub fn generate_first() -> String {
    "a0".to_string()
}

/// 生成在指定 position **之后** 的 position
///
/// 从右向左找第一个可以递增的字符，递增它并截断后续。
/// 如果所有字符都是 'z'（最大值），则追加中间字符。
pub fn generate_after(after: &str) -> String {
    let chars: Vec<char> = after.chars().collect();

    for i in (0..chars.len()).rev() {
        if let Some(idx) = char_index(chars[i]) {
            if idx < CHARS.len() - 1 {
                let mut result: Vec<char> = chars[..=i].to_vec();
                result[i] = char_at(idx + 1);
                return result.into_iter().collect();
            }
        }
    }

    format!("{}{}", after, MID_CHAR)
}

/// 生成在指定 position **之前** 的 position
///
/// 从右向左找第一个可以递减的字符，递减它并将后续填充为 'z'。
/// 如果所有字符都是 '0'（最小值），则在前面插入 '0'。
pub fn generate_before(before: &str) -> String {
    let chars: Vec<char> = before.chars().collect();

    for i in (0..chars.len()).rev() {
        if let Some(idx) = char_index(chars[i]) {
            if idx > 0 {
                let mut result: Vec<char> = chars[..=i].to_vec();
                result[i] = char_at(idx - 1);
                for _ in (i + 1)..chars.len() {
                    result.push('z');
                }
                return result.into_iter().collect();
            }
        }
    }

    format!("0{}", before)
}

/// 生成在两个 position **之间** 的 position（要求 `a < b`）
pub fn generate_between(a: &str, b: &str) -> Result<String, AppError> {
    if a >= b {
        return Err(AppError::Internal(format!(
            "generate_between 要求 a < b，收到 a={}, b={}", a, b
        )));
    }

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let min_len = a_chars.len().min(b_chars.len());

    let mut i = 0;
    while i < min_len && a_chars[i] == b_chars[i] {
        i += 1;
    }

    if i < min_len {
        let a_idx = char_index(a_chars[i]).unwrap_or(0);
        let b_idx = char_index(b_chars[i]).unwrap_or(0);

        if b_idx - a_idx > 1 {
            let mid_idx = (a_idx + b_idx) / 2;
            let mut result: Vec<char> = a_chars[..i].to_vec();
            result.push(char_at(mid_idx));
            return Ok(result.into_iter().collect());
        }

        let prefix: String = a_chars[..=i].iter().collect();
        let suffix: String = a_chars[i + 1..].iter().collect();

        if suffix.is_empty() {
            let candidate = format!("{}{}", prefix, MID_CHAR);
            if a < candidate.as_str() && candidate.as_str() < b {
                return Ok(candidate);
            }
        } else {
            let new_suffix = generate_after(&suffix);
            let candidate = format!("{}{}", prefix, new_suffix);
            if a < candidate.as_str() && candidate.as_str() < b {
                return Ok(candidate);
            }
        }
        // suffix 存在但候选超出范围，或 suffix 为空但 MID_CHAR 不在区间内 → 追加中间字符
        let candidate = format!("{}{}", a, MID_CHAR);
        if a < candidate.as_str() && candidate.as_str() < b {
            return Ok(candidate);
        }
    }

    if a_chars.len() < b_chars.len() {
        let rest: String = b_chars[a_chars.len()..].iter().collect();
        let x = generate_before(&rest);
        let candidate = format!("{}{}", a, x);
        if a < candidate.as_str() && candidate.as_str() < b {
            return Ok(candidate);
        }
        let candidate = format!("{}{}", a, MID_CHAR);
        if a < candidate.as_str() && candidate.as_str() < b {
            return Ok(candidate);
        }
    }

    Err(AppError::Internal(format!(
        "无法在 {} 和 {} 之间生成 position", a, b
    )))
}

// ─── DB 感知层：位置计算 ───────────────────────────────────────

/// 在指定父块内，根据前后邻居计算新的 fractional index position。
///
/// - 同时指定 after 和 before → 插在两者之间
/// - 只指定 after → 插在其后（如有后继兄弟则插在中间）
/// - 只指定 before → 插在其前（如有前驱兄弟则插在中间）
/// - 都不指定 → 追加到末尾
///
/// `after` / `before` 是兄弟块的 position 值（已验证属于 target_parent）。
fn calculate_position(
    _parent_id: &str,
    after: Option<&str>,
    before: Option<&str>,
) -> Result<String, AppError> {
    match (after, before) {
        (Some(ap), Some(bp)) => {
            if ap >= bp {
                return Err(AppError::BadRequest(
                    "after 位置必须在 before 位置之前".to_string(),
                ));
            }
            generate_between(ap, bp)
        }
        (Some(ap), None) => {
            // 理论上应该查 after 的下一个兄弟来 generate_between，
            // 但调用方已处理了 sibling 查询，这里不可能走到。
            // 此分支保留作为安全兜底。
            Ok(generate_after(ap))
        }
        (None, Some(bp)) => {
            Ok(generate_before(bp))
        }
        (None, None) => {
            // 调用方已处理了空列表 → generate_first 的场景
            // 此处为兜底，不应走到
            Ok(generate_first())
        }
    }
}

// ─── 语义化封装 ────────────────────────────────────────────────

/// 计算新 Block 的插入 position。
///
/// - 有 after_id → 插在 after_id 之后（如有后继兄弟则插入两者之间）
/// - 无 after_id → 追加到末尾
pub(crate) fn calculate_insert_position(
    conn: &rusqlite::Connection,
    parent_id: &str,
    after_id: Option<&str>,
) -> Result<String, AppError> {
    match after_id {
        Some(aid) => {
            let after_pos = resolve_sibling_position(conn, aid, parent_id)?;

            let next_pos = repo::get_next_sibling_position(conn, parent_id, &after_pos)
                .map_err(|e| AppError::Internal(format!("查询后继兄弟失败: {}", e)))?;

            calculate_position(
                parent_id,
                Some(&after_pos),
                next_pos.as_deref(),
            )
        }
        None => {
            let max_pos = repo::get_max_position(conn, parent_id)
                .map_err(|e| AppError::Internal(format!("查询最大 position 失败: {}", e)))?;

            match max_pos {
                Some(mp) => Ok(generate_after(&mp)),
                None => Ok(generate_first()),
            }
        }
    }
}

/// 计算移动操作的新 position。
///
/// 支持三种定位方式（优先级从高到低）：
/// 1. 同时指定 before_id 和 after_id → 插在两者之间
/// 2. 只指定 after_id → 插在之后
/// 3. 只指定 before_id → 插在之前
/// 4. 都不指定 → 追加到末尾
pub(crate) fn calculate_move_position(
    conn: &rusqlite::Connection,
    target_parent_id: &str,
    before_id: Option<&str>,
    after_id: Option<&str>,
) -> Result<String, AppError> {
    match (before_id, after_id) {
        (Some(bid), Some(aid)) => {
            let after_pos = resolve_sibling_position(conn, aid, target_parent_id)?;
            let before_pos = resolve_sibling_position(conn, bid, target_parent_id)?;
            calculate_position(target_parent_id, Some(&after_pos), Some(&before_pos))
        }
        (Some(bid), None) => {
            let before_pos = resolve_sibling_position(conn, bid, target_parent_id)?;

            let prev_pos = repo::get_prev_sibling_position(conn, target_parent_id, &before_pos)
                .map_err(|e| AppError::Internal(format!("查询前驱兄弟失败: {}", e)))?;

            calculate_position(
                target_parent_id,
                prev_pos.as_deref(),
                Some(&before_pos),
            )
        }
        (None, Some(aid)) => {
            let after_pos = resolve_sibling_position(conn, aid, target_parent_id)?;

            let next_pos = repo::get_next_sibling_position(conn, target_parent_id, &after_pos)
                .map_err(|e| AppError::Internal(format!("查询后继兄弟失败: {}", e)))?;

            calculate_position(
                target_parent_id,
                Some(&after_pos),
                next_pos.as_deref(),
            )
        }
        (None, None) => {
            let max_pos = repo::get_max_position(conn, target_parent_id)
                .map_err(|e| AppError::Internal(format!("查询最大 position 失败: {}", e)))?;

            match max_pos {
                Some(mp) => Ok(generate_after(&mp)),
                None => Ok(generate_first()),
            }
        }
    }
}

// ─── 辅助函数 ──────────────────────────────────────────────────

/// 获取指定兄弟 Block 的 position。
///
/// 验证该 Block 是 target_parent 的子块且未删除。
fn resolve_sibling_position(
    conn: &rusqlite::Connection,
    sibling_id: &str,
    target_parent_id: &str,
) -> Result<String, AppError> {
    repo::get_position(conn, sibling_id, target_parent_id)
        .map_err(|_| {
            AppError::BadRequest(format!(
                "Block {} 不是 {} 的有效子块",
                sibling_id, target_parent_id
            ))
        })
}

// ─── 单元测试（纯算法） ──────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_first() {
        assert_eq!(generate_first(), "a0");
    }

    #[test]
    fn test_generate_after_increment() {
        assert_eq!(generate_after("a0"), "a1");
        assert_eq!(generate_after("a5"), "a6");
        assert_eq!(generate_after("b"), "c");
    }

    #[test]
    fn test_generate_after_overflow() {
        assert_eq!(generate_after("a9"), "aA");
        assert_eq!(generate_after("aZ"), "aa");
    }

    #[test]
    fn test_generate_after_max_char() {
        assert_eq!(generate_after("az"), "b");
    }

    #[test]
    fn test_generate_after_all_max() {
        assert_eq!(generate_after("z"), "zU");
    }

    #[test]
    fn test_generate_before_decrement() {
        assert_eq!(generate_before("a1"), "a0");
        assert_eq!(generate_before("b"), "a");
    }

    #[test]
    fn test_generate_before_with_fill() {
        assert_eq!(generate_before("b0"), "az");
        assert_eq!(generate_before("a2"), "a1");
    }

    #[test]
    fn test_generate_before_min() {
        assert_eq!(generate_before("0"), "00");
    }

    #[test]
    fn test_generate_between_midpoint() {
        assert_eq!(generate_between("a0", "a2").unwrap(), "a1");
    }

    #[test]
    fn test_generate_between_adjacent() {
        let result = generate_between("a0", "a1").unwrap();
        assert!("a0" < result.as_str());
        assert!(result.as_str() < "a1");
    }

    #[test]
    fn test_generate_between_prefix() {
        let result = generate_between("a0", "a0U").unwrap();
        assert!("a0" < result.as_str());
        assert!(result.as_str() < "a0U");
    }

    #[test]
    fn test_generate_between_invalid_order() {
        assert!(generate_between("a2", "a0").is_err());
        assert!(generate_between("a1", "a1").is_err());
    }

    #[test]
    fn test_ordering_consistency() {
        let mut positions = vec![generate_first()];

        for _ in 0..10 {
            let last = positions.last().unwrap().clone();
            positions.push(generate_after(&last));
        }

        for i in 0..positions.len() - 1 {
            assert!(positions[i].as_str() < positions[i + 1].as_str(),
                "排序错误: {} >= {}", positions[i], positions[i + 1]);
        }

        let mid = generate_between(&positions[2], &positions[3]).unwrap();
        assert!(positions[2].as_str() < mid.as_str());
        assert!(mid.as_str() < positions[3].as_str());
    }
}
