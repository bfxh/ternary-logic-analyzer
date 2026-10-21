//! 三值代数的实现。
//!
//! 三种语义共用同一套记号（`¬ ∧ ∨ ⊕ ⊗ ⇒ ≡`），差别只在 `X` 的行为。
//!
//! | 语义 | `X` 的角色 | `X ∧ 0` | `X ∧ 1` | `X ∨ 0` | `X ∨ 1` | `X ⊕ X` | `X ⊗ X` | `X ⇒ X` | `0 ⇒ X` |
//! |---|---|---|---|---|---|---|---|---|---|
//! | Kleene (K3) | 中间元（“不知道”） | `0` | `X` | `X` | `1` | `X` | `X` | `X` | `1` |
//! | Łukasiewicz (Ł3) | 0 与 1 之间的模糊值 | `0` | `X` | `X` | `1` | `1` | `0` | `1` | `1` |
//! | Bochvar (B3) | 毒值（吸收元） | `X` | `X` | `X` | `X` | `X` | `X` | `X` | `X` |
//!
//! - `∧` / `∨` 在 K3 与 Ł3 下是格序的 min / max（`X` 是链的中点）；
//!   在 B3 下 `X` 吸收一切，沾上 `X` 即为 `X`。
//! - `⊕` 是 Łukasiewicz **强加法** `min(1, a + b)`：Ł3 下 `X ⊕ X = 1`；
//!   K3 与 B3 下 `X` 传播为 `X`。
//! - `⊗` 是 Łukasiewicz **强合取** `max(0, a + b - 1)`：Ł3 下 `X ⊗ X = 0`；
//!   K3 与 B3 下 `X` 传播为 `X`。
//! - `⇒` 在 K3 下是材料蕴含 `¬a ∨ b`；在 Ł3 下是 MV 蕴含
//!   `min(1, 1 - a + b)`，故 `X ⇒ X = 1`；在 B3 下 `X` 仍是毒值。

use crate::value::Ternary;

/// 三值逻辑的语义体系。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Algebra {
    /// Kleene 强三值逻辑：`X` 表示“不知道”，运算在不确定时产出 `X`。
    Kleene,
    /// Łukasiewicz 三值模糊逻辑：`X` 是 0 与 1 之间的不确定值。
    Lukasiewicz,
    /// Bochvar 弱三值逻辑：`X` 是“毒值”，沾上 `X` 即为 `X`。
    Bochvar,
}

impl Algebra {
    /// 全部语义体系。
    pub const ALL: [Self; 3] = [Self::Kleene, Self::Lukasiewicz, Self::Bochvar];

    /// 语义体系的稳定短名，用于 CLI 与序列化。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kleene => "kleene",
            Self::Lukasiewicz => "lukasiewicz",
            Self::Bochvar => "bochvar",
        }
    }

    /// 解析语义名称，接受 `kleene` / `k3`、`lukasiewicz` / `l3` / `luk` / `bochvar` / `b3`。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "kleene" | "k3" => Some(Self::Kleene),
            "lukasiewicz" | "luk" | "l3" => Some(Self::Lukasiewicz),
            "bochvar" | "b3" => Some(Self::Bochvar),
            _ => None,
        }
    }

    /// 该语义下 `X` 是否是吸收元（Bochvar 毒值语义）。
    pub const fn is_poisonous(self) -> bool {
        matches!(self, Self::Bochvar)
    }

    /// 逻辑否定 `¬`。三种语义一致：`¬X = X`。
    pub const fn not(self, a: Ternary) -> Ternary {
        match a {
            Ternary::False => Ternary::True,
            Ternary::Unknown => Ternary::Unknown,
            Ternary::True => Ternary::False,
        }
    }

    /// 逻辑与 `∧`。
    ///
    /// K3 / Ł3：格序 min（`X` 是中间元）。B3：`X` 吸收。
    pub const fn and(self, a: Ternary, b: Ternary) -> Ternary {
        match self {
            Self::Bochvar => {
                if a.is_unknown() || b.is_unknown() {
                    Ternary::Unknown
                } else if a.is_true() && b.is_true() {
                    Ternary::True
                } else {
                    Ternary::False
                }
            }
            Self::Kleene | Self::Lukasiewicz => match (a, b) {
                (Ternary::False, _) | (_, Ternary::False) => Ternary::False,
                (Ternary::True, Ternary::True) => Ternary::True,
                _ => Ternary::Unknown,
            },
        }
    }

    /// 逻辑或 `∨`。
    ///
    /// K3 / Ł3：格序 max（`X` 是中间元）。B3：`X` 吸收。
    pub const fn or(self, a: Ternary, b: Ternary) -> Ternary {
        match self {
            Self::Bochvar => {
                if a.is_unknown() || b.is_unknown() {
                    Ternary::Unknown
                } else if a.is_true() || b.is_true() {
                    Ternary::True
                } else {
                    Ternary::False
                }
            }
            Self::Kleene | Self::Lukasiewicz => match (a, b) {
                (Ternary::True, _) | (_, Ternary::True) => Ternary::True,
                (Ternary::False, Ternary::False) => Ternary::False,
                _ => Ternary::Unknown,
            },
        }
    }

    /// 强加法 `⊕`。
    ///
    /// - Łukasiewicz：`min(1, a + b)`，`X ⊕ X = 1`。
    /// - Kleene：`0/1` 上为 `max(0, a + b - 1)`，`X` 传播为 `X`。
    /// - Bochvar：`X` 是毒值，沾上即 `X`。
    pub const fn strong_add(self, a: Ternary, b: Ternary) -> Ternary {
        match self {
            Self::Lukasiewicz => {
                // min(1, a + b)，a,b ∈ {0, 0.5, 1}
                let s = a.as_f64() + b.as_f64();
                if s >= 1.0 {
                    Ternary::True
                } else if s <= 0.0 {
                    Ternary::False
                } else {
                    Ternary::Unknown
                }
            }
            Self::Kleene | Self::Bochvar => {
                if a.is_unknown() || b.is_unknown() {
                    Ternary::Unknown
                } else if a.is_true() && b.is_true() {
                    Ternary::True
                } else {
                    Ternary::False
                }
            }
        }
    }

    /// 强合取 `⊗`。
    ///
    /// - Łukasiewicz：`max(0, a + b - 1)`，`X ⊗ X = 0`。
    /// - Kleene / Bochvar：`X` 传播为 `X`。
    pub const fn strong_and(self, a: Ternary, b: Ternary) -> Ternary {
        match self {
            Self::Lukasiewicz => {
                let s = a.as_f64() + b.as_f64() - 1.0;
                if s <= 0.0 {
                    Ternary::False
                } else if s >= 1.0 {
                    Ternary::True
                } else {
                    Ternary::Unknown
                }
            }
            Self::Kleene | Self::Bochvar => {
                if a.is_unknown() || b.is_unknown() {
                    Ternary::Unknown
                } else if a.is_true() && b.is_true() {
                    Ternary::True
                } else {
                    Ternary::False
                }
            }
        }
    }

    /// 蕴含 `⇒`。
    ///
    /// - Kleene：材料蕴含 `¬a ∨ b`。
    /// - Łukasiewicz：MV 蕴含 `min(1, 1 - a + b)`，故 `X ⇒ X = 1`。
    /// - Bochvar：`X` 是毒值，沾上即 `X`。
    pub const fn implies(self, a: Ternary, b: Ternary) -> Ternary {
        match self {
            Self::Kleene => self.or(self.not(a), b),
            Self::Bochvar => {
                if a.is_unknown() || b.is_unknown() {
                    Ternary::Unknown
                } else if a.is_true() {
                    b
                } else {
                    Ternary::True
                }
            }
            Self::Lukasiewicz => {
                // min(1, 1 - a + b)
                let s = 1.0 - a.as_f64() + b.as_f64();
                if s >= 1.0 {
                    Ternary::True
                } else if s <= 0.0 {
                    Ternary::False
                } else {
                    Ternary::Unknown
                }
            }
        }
    }

    /// 等价 `≡`：`(a ⇒ b) ∧ (b ⇒ a)`。
    pub const fn equiv(self, a: Ternary, b: Ternary) -> Ternary {
        self.and(self.implies(a, b), self.implies(b, a))
    }
}

impl std::fmt::Display for Algebra {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kleene_and_truth_table() {
        use Ternary::*;
        assert_eq!(Algebra::Kleene.and(False, Unknown), False);
        assert_eq!(Algebra::Kleene.and(Unknown, Unknown), Unknown);
        assert_eq!(Algebra::Kleene.and(Unknown, True), Unknown);
        assert_eq!(Algebra::Kleene.and(True, True), True);
    }

    #[test]
    fn bochvar_and_is_poisonous() {
        use Ternary::*;
        assert_eq!(Algebra::Bochvar.and(False, Unknown), Unknown);
        assert_eq!(Algebra::Bochvar.and(True, Unknown), Unknown);
        assert_eq!(Algebra::Bochvar.and(True, True), True);
        assert_eq!(Algebra::Bochvar.and(False, False), False);
    }

    #[test]
    fn bochvar_or_is_poisonous() {
        use Ternary::*;
        assert_eq!(Algebra::Bochvar.or(True, Unknown), Unknown);
        assert_eq!(Algebra::Bochvar.or(False, Unknown), Unknown);
        assert_eq!(Algebra::Bochvar.or(True, True), True);
    }

    #[test]
    fn kleene_and_or_are_lattice_ops() {
        for a in Ternary::ALL {
            for b in Ternary::ALL {
                assert_eq!(Algebra::Kleene.and(a, b), Algebra::Lukasiewicz.and(a, b));
                assert_eq!(Algebra::Kleene.or(a, b), Algebra::Lukasiewicz.or(a, b));
            }
        }
    }

    #[test]
    fn strong_add_differs_between_semantics() {
        use Ternary::*;
        assert_eq!(Algebra::Kleene.strong_add(Unknown, Unknown), Unknown);
        assert_eq!(Algebra::Lukasiewicz.strong_add(Unknown, Unknown), True);
        assert_eq!(Algebra::Bochvar.strong_add(Unknown, Unknown), Unknown);
        assert_eq!(Algebra::Lukasiewicz.strong_add(False, True), True);
        assert_eq!(Algebra::Lukasiewicz.strong_add(False, False), False);
        assert_eq!(Algebra::Kleene.strong_add(True, True), True);
    }

    #[test]
    fn strong_and_lukasiewicz_matches_formula() {
        use Ternary::*;
        // max(0, a + b - 1)
        assert_eq!(Algebra::Lukasiewicz.strong_and(Unknown, Unknown), False);
        assert_eq!(Algebra::Lukasiewicz.strong_and(Unknown, True), Unknown);
        assert_eq!(Algebra::Lukasiewicz.strong_and(True, True), True);
        assert_eq!(Algebra::Lukasiewicz.strong_and(False, True), False);
        assert_eq!(Algebra::Kleene.strong_and(Unknown, True), Unknown);
        assert_eq!(Algebra::Bochvar.strong_and(Unknown, True), Unknown);
    }

    #[test]
    fn implication_differs_between_semantics() {
        use Ternary::*;
        // K3: X ⇒ X = X；Ł3: X ⇒ X = 1；B3: X ⇒ X = X
        assert_eq!(Algebra::Kleene.implies(Unknown, Unknown), Unknown);
        assert_eq!(Algebra::Lukasiewicz.implies(Unknown, Unknown), True);
        assert_eq!(Algebra::Bochvar.implies(Unknown, Unknown), Unknown);
        assert_eq!(Algebra::Kleene.implies(True, False), False);
        assert_eq!(Algebra::Kleene.implies(False, True), True);
        assert_eq!(Algebra::Kleene.implies(False, Unknown), True);
        assert_eq!(Algebra::Bochvar.implies(False, Unknown), Unknown);
        assert_eq!(Algebra::Lukasiewicz.implies(False, Unknown), True);
    }

    #[test]
    fn algebra_parsing() {
        assert_eq!(Algebra::parse("K3"), Some(Algebra::Kleene));
        assert_eq!(Algebra::parse("luk"), Some(Algebra::Lukasiewicz));
        assert_eq!(Algebra::parse("b3"), Some(Algebra::Bochvar));
        assert_eq!(Algebra::parse("nope"), None);
    }
}
