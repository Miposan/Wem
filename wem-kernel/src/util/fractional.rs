//! Fractional Index 实现
//!
//! 基于 base-62 字符集的分数索引算法。
//! 用字符串的字典序来表示 Block 的排序位置，支持无限精度插入。
//!
//! 核心思想（参考 02-block-tree.md §2）：
//! - 每个 Block 有一个 position 字符串
//! - 字符串按字典序比较，决定兄弟节点间的顺序
//! - 插入新 Block 时，生成一个介于目标位置之间的字符串
//! - 不需要重新编号其他节点（和 f64 相比的核心优势）
//!
//! 示例排序：
//! ```
//! "a0" < "a1" < "a1V" < "a2" < "b0"
//! ```
//!
//! 为什么不用 f64：
//! - f64 约 15 位有效数字，连续二分约 50 次后精度耗尽
//! - 字符串理论上无限精度，极端情况下才需要 renumber

/// Base-62 字符集（按 ASCII 码排列，自然字典序）
///
/// '0' (48) < '9' (57) < 'A' (65) < 'Z' (90) < 'a' (97) < 'z' (122)
/// 标准字符串比较 = 字典序，无需自定义比较器
const CHARS: [char; 62] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H',
    'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
    's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
];

/// 中间字符（索引 30，约在 base-62 的正中间）
///
/// 当需要"随便插入一个中间位置"时使用，前后都有足够空间
const MID_CHAR: char = 'U';

/// 获取字符在字符集中的索引（'0'→0, 'A'→10, 'a'→36）
fn char_index(c: char) -> Option<usize> {
    CHARS.iter().position(|&x| x == c)
}

/// 根据索引获取字符
fn char_at(i: usize) -> char {
    CHARS[i]
}

/// 生成第一个 position
///
/// 返回 `"a0"` — 一个位于字符集中间的值，前后都有足够空间插入。
///
/// ```
/// use crate::util::fractional::generate_first;
/// assert_eq!(generate_first(), "a0");
/// ```
pub fn generate_first() -> String {
    "a0".to_string()
}

/// 生成在指定 position **之后** 的 position
///
/// 策略：从右向左找第一个可以递增的字符，递增它并截断后续。
/// 如果所有字符都是 'z'（最大值），则追加中间字符。
///
/// ```
/// // "a0" → 递增 '0' → "a1"
/// // "az" → 'z' 不可递增，递增 'a' → "b"
/// // "z"  → 全是 max，追加 → "zU"
/// ```
pub fn generate_after(after: &str) -> String {
    let chars: Vec<char> = after.chars().collect();

    // 从右向左找第一个可以递增的字符
    for i in (0..chars.len()).rev() {
        if let Some(idx) = char_index(chars[i]) {
            if idx < CHARS.len() - 1 {
                // 递增这个字符，截断后面的
                let mut result: Vec<char> = chars[..=i].to_vec();
                result[i] = char_at(idx + 1);
                return result.into_iter().collect();
            }
        }
    }

    // 所有字符都是 'z'，追加中间字符
    format!("{}{}", after, MID_CHAR)
}

/// 生成在指定 position **之前** 的 position
///
/// 策略：从右向左找第一个可以递减的字符，递减它并将后续填充为 'z'。
/// 如果所有字符都是 '0'（最小值），则在前面插入 '0'。
///
/// ```
/// // "a1" → 递减 '1' → "a0"
/// // "b0" → '0' 不可递减，递减 'b' → "az"
/// // "0"  → 全是 min，前面加 '0' → "00"
/// ```
pub fn generate_before(before: &str) -> String {
    let chars: Vec<char> = before.chars().collect();

    // 从右向左找第一个可以递减的字符
    for i in (0..chars.len()).rev() {
        if let Some(idx) = char_index(chars[i]) {
            if idx > 0 {
                let mut result: Vec<char> = chars[..=i].to_vec();
                result[i] = char_at(idx - 1);
                // 后续位置填充最大值 'z'（确保在递减后的字符后面排最大的）
                for _ in (i + 1)..chars.len() {
                    result.push('z');
                }
                return result.into_iter().collect();
            }
        }
    }

    // 所有字符都是 '0'，在前面插入 '0'
    format!("0{}", before)
}

/// 生成在两个 position **之间** 的 position
///
/// 要求 `a < b`（字典序）。
///
/// 三种情况：
/// 1. 差异位有中间字符 → 直接用中间字符
/// 2. 差异位相邻 → 在 a 的后缀中递增
/// 3. a 是 b 的前缀 → 生成 b 剩余部分的前驱
pub fn generate_between(a: &str, b: &str) -> String {
    assert!(a < b, "generate_between 要求 a < b，收到 a={}, b={}", a, b);

    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let min_len = a_chars.len().min(b_chars.len());

    // 找第一个不同的位置
    let mut i = 0;
    while i < min_len && a_chars[i] == b_chars[i] {
        i += 1;
    }

    if i < min_len {
        // 情况 1 & 2：在位置 i 找到差异
        let a_idx = char_index(a_chars[i]).unwrap_or(0);
        let b_idx = char_index(b_chars[i]).unwrap_or(0);

        if b_idx - a_idx > 1 {
            // 情况 1：有中间字符
            // 例如 "a0" 和 "a2" 之间 → "a1"
            let mid_idx = (a_idx + b_idx) / 2;
            let mut result: Vec<char> = a_chars[..i].to_vec();
            result.push(char_at(mid_idx));
            return result.into_iter().collect();
        }

        // 情况 2：相邻字符（无中间值）
        // 策略：取 a 的前缀到 i（含），对 a 的剩余后缀执行 generate_after
        let prefix: String = a_chars[..=i].iter().collect();
        let suffix: String = a_chars[i + 1..].iter().collect();

        if suffix.is_empty() {
            // a 在 i 之后没有更多字符，直接追加中间字符
            // 例如 "a0" 和 "a1" 之间 → "a0U"
            let candidate = format!("{}{}", prefix, MID_CHAR);
            debug_assert!(
                a < candidate.as_str() && candidate.as_str() < b,
                "生成的 position {} 不在 {} 和 {} 之间",
                candidate,
                a,
                b
            );
            return candidate;
        } else {
            // a 在 i 之后还有字符，对后缀递增
            // 例如 "a0U" 和 "a1" 之间 → "a0V"（后缀 "U" → "V"）
            let new_suffix = generate_after(&suffix);
            let candidate = format!("{}{}", prefix, new_suffix);
            if candidate.as_str() < b {
                debug_assert!(a < candidate.as_str(), "递增后应大于 a");
                return candidate;
            }
            // 递增后超过了 b（极端情况），改用追加策略
            let candidate = format!("{}{}", a, MID_CHAR);
            debug_assert!(
                a < candidate.as_str() && candidate.as_str() < b,
                "无法在 {} 和 {} 之间生成 position",
                a,
                b
            );
            return candidate;
        }
    }

    // 情况 3：a 是 b 的前缀
    // 例如 a="a0", b="a0U" → 在 a 后追加 "T"（"U" 的前驱）
    if a_chars.len() < b_chars.len() {
        let rest: String = b_chars[a_chars.len()..].iter().collect();
        let x = generate_before(&rest);
        let candidate = format!("{}{}", a, x);
        if a < candidate.as_str() && candidate.as_str() < b {
            return candidate;
        }
        // generate_before 不行，尝试追加中间字符
        let candidate = format!("{}{}", a, MID_CHAR);
        debug_assert!(
            a < candidate.as_str() && candidate.as_str() < b,
            "无法在 {} 和 {} 之间生成 position",
            a,
            b
        );
        return candidate;
    }

    // 不应该到达这里（a < b 保证至少有一种情况命中）
    unreachable!("generate_between: a ({}) < b ({}) 但未找到插入点", a, b)
}

// ─── 单元测试 ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_first() {
        assert_eq!(generate_first(), "a0");
    }

    #[test]
    fn test_generate_after_increment() {
        // 递增最后一个字符
        assert_eq!(generate_after("a0"), "a1");
        assert_eq!(generate_after("a5"), "a6");
        assert_eq!(generate_after("b"), "c");
    }

    #[test]
    fn test_generate_after_overflow() {
        // '9' → 'A'（跨数字到字母）
        assert_eq!(generate_after("a9"), "aA");
        // 'Z' → 'a'（跨大写到小写）
        assert_eq!(generate_after("aZ"), "aa");
    }

    #[test]
    fn test_generate_after_max_char() {
        // 'z' 是最大字符，递增前一个字符
        assert_eq!(generate_after("az"), "b");
    }

    #[test]
    fn test_generate_after_all_max() {
        // 全是 'z'，追加中间字符
        let result = generate_after("z");
        assert_eq!(result, "zU");
    }

    #[test]
    fn test_generate_before_decrement() {
        assert_eq!(generate_before("a1"), "a0");
        assert_eq!(generate_before("b"), "a");
    }

    #[test]
    fn test_generate_before_with_fill() {
        // 'b' 递减为 'a'，后续填 'z'
        assert_eq!(generate_before("b0"), "az");
        // '1' 递减为 '0'
        assert_eq!(generate_before("a2"), "a1");
    }

    #[test]
    fn test_generate_before_min() {
        // 全是 '0'，前面加 '0'
        let result = generate_before("0");
        assert_eq!(result, "00");
    }

    #[test]
    fn test_generate_between_midpoint() {
        // "a0" 和 "a2" 之间 → "a1"
        assert_eq!(generate_between("a0", "a2"), "a1");
    }

    #[test]
    fn test_generate_between_adjacent() {
        // "a0" 和 "a1" 之间 → "a0U"（追加中间字符）
        let result = generate_between("a0", "a1");
        assert!("a0" < result.as_str());
        assert!(result.as_str() < "a1");
    }

    #[test]
    fn test_generate_between_prefix() {
        // "a0" 和 "a0U" 之间 → "a0T"（"U" 的前驱）
        let result = generate_between("a0", "a0U");
        assert!("a0" < result.as_str());
        assert!(result.as_str() < "a0U");
    }

    #[test]
    fn test_ordering_consistency() {
        // 连续插入 10 次，验证所有 position 保持字典序
        let mut positions = vec![generate_first()];

        for _ in 0..10 {
            let last = positions.last().unwrap().clone();
            positions.push(generate_after(&last));
        }

        // 验证严格递增
        for i in 0..positions.len() - 1 {
            assert!(positions[i].as_str() < positions[i + 1].as_str(),
                "排序错误: {} >= {}", positions[i], positions[i + 1]);
        }

        // 验证 between 生成的值确实在中间
        let mid = generate_between(&positions[2], &positions[3]);
        assert!(positions[2].as_str() < mid.as_str());
        assert!(mid.as_str() < positions[3].as_str());
    }
}
