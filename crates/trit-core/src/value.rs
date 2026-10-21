//! 三值逻辑的核心值类型。
//!
//! 三值逻辑（ternary logic）中的每个值属于 `{0, 1, X}`，其中 `X` 表示
//! “未知 / 无关 / 不确定”。不同的三值代数对 `X` 的解释不同：
//!
//! - **Kleene 强三值逻辑**（K3）：`X` 是“不知道”，是格序的中点。
//! - **Łukasiewicz 三值模糊逻辑**（Ł3）：`X` 是“介于 0 与 1 之间的不确定值”。
//! - **Bochvar 弱三值逻辑**（B3）：`X` 是“毒值”，吸收一切运算。

use serde::{Deserialize, Serialize};
use std::fmt;

/// 三值逻辑中的单个取值。
///
/// 顺序即枚举顺序，`Ord` 的派生保证 `False < Unknown < True`，
/// 方便用 `min` / `max` 表达格序的 `AND` / `OR`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Ternary {
    /// 假（0）。
    False,
    /// 未知（X）。
    Unknown,
    /// 真（1）。
    True,
}

impl Ternary {
    /// 逻辑常量假。
    pub const FALSE: Self = Self::False;
    /// 逻辑常量未知。
    pub const UNKNOWN: Self = Self::Unknown;
    /// 逻辑常量真。
    pub const TRUE: Self = Self::True;

    /// 三值的所有取值，按 `False < Unknown < True` 顺序。
    pub const ALL: [Self; 3] = [Self::False, Self::Unknown, Self::True];

    /// `bool -> Ternary`：`true -> True`，`false -> False`（无分支 Select）。
    pub const fn from_bool(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    /// `Ternary -> Option<bool>`：`Unknown` 返回 `None`。
    pub const fn to_bool(self) -> Option<bool> {
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            Self::Unknown => None,
        }
    }

    /// 是否为二值（非 `Unknown`）。
    pub const fn is_binary(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// 数值形式：`False -> 0`，`Unknown -> 0.5`，`True -> 1`。
    ///
    /// 这是把 `X` 视为“0 与 1 之间的不确定值”时的自然映射，
    /// 也是 Łukasiewicz 强加法 / 强合取的定义基础。
    pub const fn as_f64(self) -> f64 {
        match self {
            Self::False => 0.0,
            Self::Unknown => 0.5,
            Self::True => 1.0,
        }
    }

    /// 经典二值真值（把 `X` 当作假）。
    pub const fn is_false(self) -> bool {
        matches!(self, Self::False)
    }

    /// 经典二值真值（把 `X` 当作真）。
    pub const fn is_true(self) -> bool {
        matches!(self, Self::True)
    }

    /// 是否为未知值。
    pub const fn is_unknown(self) -> bool {
        matches!(self, Self::Unknown)
    }

    /// 是否为确定值（`0` 或 `1`）。
    pub const fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }

    /// 经典二值化：把 `X` 折成假。
    pub const fn to_bool_strict(self) -> bool {
        matches!(self, Self::True)
    }

    /// 经典二值化：把 `X` 折成真。
    pub const fn to_bool_loose(self) -> bool {
        !matches!(self, Self::False)
    }

    /// 解析单个字符：`0`/`f`/`F` → 假，`1`/`t`/`T` → 真，`x`/`X`/`?` → 未知。
    pub fn parse_char(c: char) -> Option<Self> {
        match c {
            '0' | 'f' | 'F' => Some(Self::False),
            '1' | 't' | 'T' => Some(Self::True),
            'x' | 'X' | '?' => Some(Self::Unknown),
            _ => None,
        }
    }

    /// 单字符表示：假为 `0`，未知为 `X`，真为 `1`。
    pub const fn as_char(self) -> char {
        match self {
            Self::False => '0',
            Self::Unknown => 'X',
            Self::True => '1',
        }
    }

    /// 单字符紧凑表示（用于真值表列头，未知记作小写 `x`）。
    pub const fn as_compact_char(self) -> char {
        match self {
            Self::False => '0',
            Self::Unknown => 'x',
            Self::True => '1',
        }
    }
}

impl fmt::Display for Ternary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::False => "0",
            Self::Unknown => "X",
            Self::True => "1",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordering_is_false_unknown_true() {
        assert!(Ternary::False < Ternary::Unknown);
        assert!(Ternary::Unknown < Ternary::True);
    }

    #[test]
    fn parse_char_round_trips() {
        for v in Ternary::ALL {
            assert_eq!(Ternary::parse_char(v.as_char()), Some(v));
        }
        assert_eq!(Ternary::parse_char('?'), Some(Ternary::Unknown));
        assert_eq!(Ternary::parse_char('z'), None);
    }

    #[test]
    fn numeric_mapping() {
        assert!((Ternary::False.as_f64() - 0.0).abs() < f64::EPSILON);
        assert!((Ternary::Unknown.as_f64() - 0.5).abs() < f64::EPSILON);
        assert!((Ternary::True.as_f64() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn binarization() {
        assert!(!Ternary::Unknown.to_bool_strict());
        assert!(Ternary::Unknown.to_bool_loose());
        assert!(Ternary::True.to_bool_strict());
        assert!(Ternary::False.is_known());
        assert!(!Ternary::Unknown.is_known());
    }
}
