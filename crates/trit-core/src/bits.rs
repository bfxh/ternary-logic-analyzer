//! 位并行真值表引擎。
//!
//! 用位向量同时计算多组变量赋值：每个变量的第 `i` 位表示第 `i` 组赋值中
//! 该变量的取值。一个 `u64` 字一次处理 64 组赋值。
//!
//! 一个三值变量在位并行表示下需要**两个**位平面：
//! - `ones`：该位为 1 表示该组取真；
//! - `unknowns`：该位为 1 表示该组取未知。
//!
//! 于是取值由两个平面唯一确定：`(ones, unknowns)` 为
//! `(0,0) → 0`、`(1,0) → 1`、`(0,1) → X`，`(1,1)` 为非法组合（不会出现）。

use crate::algebra::Algebra;
use crate::value::Ternary;

/// 一个位平面的真值向量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BitPlane(pub u64);

impl BitPlane {
    /// 全 0 平面。
    pub const ZERO: Self = Self(0);
    /// 全 1 平面。
    pub const ONES: Self = Self(u64::MAX);

    /// 构造指定位为 1 的平面。
    pub const fn with_bit(bit: u32) -> Self {
        Self(1u64 << (bit % 64))
    }

    /// 该平面第 `bit` 位是否为 1。
    pub const fn bit(self, bit: u32) -> bool {
        (self.0 >> (bit % 64)) & 1 == 1
    }

    /// 平面内 1 的个数（popcount）。
    pub const fn count_ones(self) -> u32 {
        self.0.count_ones()
    }
}

/// 三值的位并行表示：一对位平面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TriBit {
    /// 1 平面：取真的组。
    pub ones: BitPlane,
    /// X 平面：取未知的组。
    pub unknowns: BitPlane,
}

impl TriBit {
    /// 从二进制平面构造：`ones` 的位就是 `True`，`unknowns = 0`。
    ///
    /// 这是二进制与三值之间的零开销桥：不复制、不分配。
    pub const fn from_binary(ones: u64) -> Self {
        Self {
            ones: BitPlane(ones),
            unknowns: BitPlane::ZERO,
        }
    }

    /// 是否为纯二值（没有 `Unknown`）。
    pub const fn is_binary(self) -> bool {
        self.unknowns.0 == 0
    }

    /// 若是纯二值，取出 `ones` 平面；否则 `None`。
    pub const fn as_binary(self) -> Option<u64> {
        if self.is_binary() {
            Some(self.ones.0)
        } else {
            None
        }
    }

    /// 全假。
    pub const FALSE: Self = Self {
        ones: BitPlane::ZERO,
        unknowns: BitPlane::ZERO,
    };
    /// 全真。
    pub const TRUE: Self = Self {
        ones: BitPlane::ONES,
        unknowns: BitPlane::ZERO,
    };
    /// 全未知。
    pub const UNKNOWN: Self = Self {
        ones: BitPlane::ZERO,
        unknowns: BitPlane::ONES,
    };

    /// 构造常量。
    pub const fn constant(v: Ternary) -> Self {
        match v {
            Ternary::False => Self::FALSE,
            Ternary::Unknown => Self::UNKNOWN,
            Ternary::True => Self::TRUE,
        }
    }

    /// 读出第 `bit` 组的取值。
    pub const fn get(self, bit: u32) -> Ternary {
        if self.unknowns.bit(bit) {
            Ternary::Unknown
        } else if self.ones.bit(bit) {
            Ternary::True
        } else {
            Ternary::False
        }
    }

    /// 该平面代表的“确定真”位数（ones 且非 unknown）。
    const fn crisp_ones(self) -> u64 {
        self.ones.0 & !self.unknowns.0
    }

    /// 该平面代表的“确定假”位数（既非 one 也非 unknown）。
    const fn crisp_zeros(self) -> u64 {
        !self.ones.0 & !self.unknowns.0
    }

    /// 该平面代表的“未知”位数。
    const fn unknowns(self) -> u64 {
        self.unknowns.0
    }

    /// 按位取反。`¬X = X`。
    pub const fn not(self) -> Self {
        Self {
            ones: BitPlane(!self.crisp_ones() & !self.unknowns()),
            unknowns: self.unknowns,
        }
    }

    /// 按位与。
    ///
    /// K3 / Ł3：格序 min（`X` 是中间元）。B3：`X` 吸收。
    pub const fn and(self, other: Self, alg: Algebra) -> Self {
        if alg.is_poisonous() {
            return Self::poison_and(self, other);
        }
        self.and_lattice(other)
    }

    /// 按位或。
    ///
    /// K3 / Ł3：格序 max（`X` 是中间元）。B3：`X` 吸收。
    pub const fn or(self, other: Self, alg: Algebra) -> Self {
        if alg.is_poisonous() {
            return Self::poison_or(self, other);
        }
        self.or_lattice(other)
    }

    /// Bochvar 毒值与：任一为 `X` 则结果为 `X`，否则按二值与运算。
    const fn poison_and(self, other: Self) -> Self {
        let resx = self.unknowns() | other.unknowns();
        let a1 = self.crisp_ones();
        let b1 = other.crisp_ones();
        let res1 = a1 & b1;
        Self {
            ones: BitPlane(res1),
            unknowns: BitPlane(resx),
        }
    }

    /// Bochvar 毒值或：任一为 `X` 则结果为 `X`，否则按二值或运算。
    const fn poison_or(self, other: Self) -> Self {
        let resx = self.unknowns() | other.unknowns();
        let a1 = self.crisp_ones();
        let b1 = other.crisp_ones();
        // 1 平面必须扣掉未知位，否则会出现被禁止的 (1,1)。
        let res1 = (a1 | b1) & !resx;
        Self {
            ones: BitPlane(res1),
            unknowns: BitPlane(resx),
        }
    }

    /// 按位与：格序 min（K3 / Ł3）。
    pub const fn and_lattice(self, other: Self) -> Self {
        let a0 = self.crisp_zeros();
        let b0 = other.crisp_zeros();
        let a1 = self.crisp_ones();
        let b1 = other.crisp_ones();
        let res0 = a0 | b0;
        let res1 = a1 & b1;
        let resx = !res0 & !res1;
        Self {
            ones: BitPlane(res1),
            unknowns: BitPlane(resx),
        }
    }

    /// 按位或：格序 max（K3 / Ł3）。
    pub const fn or_lattice(self, other: Self) -> Self {
        let a0 = self.crisp_zeros();
        let b0 = other.crisp_zeros();
        let a1 = self.crisp_ones();
        let b1 = other.crisp_ones();
        let res1 = a1 | b1;
        let res0 = a0 & b0;
        let resx = !res1 & !res0;
        Self {
            ones: BitPlane(res1),
            unknowns: BitPlane(resx),
        }
    }

    /// 按位强加法 `⊕`，语义相关。
    pub const fn strong_add(self, other: Self, alg: Algebra) -> Self {
        match alg {
            // Łukasiewicz: min(1, a + b)
            Algebra::Lukasiewicz => {
                let a0 = self.crisp_zeros();
                let a1 = self.crisp_ones();
                let ax = self.unknowns();
                let b0 = other.crisp_zeros();
                let b1 = other.crisp_ones();
                let bx = other.unknowns();
                let res1 = (a0 & b1) | (a1 & b0) | (a1 & b1) | (ax & bx) | (ax & b1) | (a1 & bx);
                let res0 = a0 & b0;
                let resx = !res1 & !res0;
                Self {
                    ones: BitPlane(res1),
                    unknowns: BitPlane(resx),
                }
            }
            // Kleene / Bochvar: 0/1 上为强加法，X 传播
            Algebra::Kleene | Algebra::Bochvar => {
                let a1 = self.crisp_ones();
                let b1 = other.crisp_ones();
                let res1 = a1 & b1;
                let resx = self.unknowns() | other.unknowns();
                let _res0 = !res1 & !resx;
                Self {
                    ones: BitPlane(res1),
                    unknowns: BitPlane(resx),
                }
            }
        }
    }

    /// 按位强合取 `⊗`，语义相关。
    pub const fn strong_and(self, other: Self, alg: Algebra) -> Self {
        match alg {
            // Łukasiewicz: max(0, a + b - 1)
            Algebra::Lukasiewicz => {
                let a1 = self.crisp_ones();
                let b1 = other.crisp_ones();
                let res1 = a1 & b1;
                let res0 =
                    self.crisp_zeros() | other.crisp_zeros() | (self.unknowns() & other.unknowns());
                let resx = !res1 & !res0;
                Self {
                    ones: BitPlane(res1),
                    unknowns: BitPlane(resx),
                }
            }
            // Kleene / Bochvar: 0/1 上为强合取，X 传播
            Algebra::Kleene | Algebra::Bochvar => {
                let a1 = self.crisp_ones();
                let b1 = other.crisp_ones();
                let res1 = a1 & b1;
                let resx = self.unknowns() | other.unknowns();
                let _res0 = !res1 & !resx;
                Self {
                    ones: BitPlane(res1),
                    unknowns: BitPlane(resx),
                }
            }
        }
    }

    /// 按位蕴含 `⇒`，语义相关。
    pub const fn implies(self, other: Self, alg: Algebra) -> Self {
        match alg {
            // Kleene: 材料蕴含 ¬a ∨ b
            Algebra::Kleene => self.not().or_lattice(other),
            // Bochvar: 毒值语义，任一 X 出现即 X；否则 a=1 取 b，a=0 取 1
            Algebra::Bochvar => {
                let a0 = self.crisp_zeros();
                let a1 = self.crisp_ones();
                let b0 = other.crisp_zeros();
                let b1 = other.crisp_ones();
                let res1 = (a0 & (b0 | b1)) | (a1 & b1);
                let resx = self.unknowns() | other.unknowns();
                Self {
                    ones: BitPlane(res1),
                    unknowns: BitPlane(resx),
                }
            }
            // Łukasiewicz: MV 蕴含 min(1, 1 - a + b)
            Algebra::Lukasiewicz => {
                let a0 = self.crisp_zeros();
                let a1 = self.crisp_ones();
                let ax = self.unknowns();
                let b0 = other.crisp_zeros();
                let b1 = other.crisp_ones();
                let bx = other.unknowns();
                let res1 = (a0 & (b0 | bx | b1)) | (ax & (bx | b1)) | (a1 & b1);
                let res0 = a1 & b0;
                let resx = !res1 & !res0;
                Self {
                    ones: BitPlane(res1),
                    unknowns: BitPlane(resx),
                }
            }
        }
    }

    /// 按位等价 `≡`。
    pub const fn equiv(self, other: Self, alg: Algebra) -> Self {
        self.implies(other, alg).and(other.implies(self, alg), alg)
    }
}

/// 一个表达式的完整真值表结果。
#[derive(Debug, Clone)]
pub struct TruthTable {
    /// 变量名（按编号顺序）。
    pub variables: Vec<String>,
    /// 每组赋值的结果值。
    pub values: Vec<Ternary>,
}

impl TruthTable {
    /// 取出指定赋值下的结果。
    pub fn value_at(&self, index: usize) -> Option<Ternary> {
        self.values.get(index).copied()
    }

    /// 结果中出现过的不同取值集合（升序）。
    pub fn distinct_values(&self) -> Vec<Ternary> {
        let mut seen = [false; 3];
        for v in &self.values {
            seen[match v {
                Ternary::False => 0,
                Ternary::Unknown => 1,
                Ternary::True => 2,
            }] = true;
        }
        let mut out = Vec::new();
        if seen[0] {
            out.push(Ternary::False);
        }
        if seen[1] {
            out.push(Ternary::Unknown);
        }
        if seen[2] {
            out.push(Ternary::True);
        }
        out
    }

    /// 结果全为假。
    pub fn is_always_false(&self) -> bool {
        self.values.iter().all(|v| v.is_false())
    }

    /// 结果全为真。
    pub fn is_always_true(&self) -> bool {
        self.values.iter().all(|v| v.is_true())
    }

    /// 结果是否含未知。
    pub fn has_unknown(&self) -> bool {
        self.values.iter().any(|v| v.is_unknown())
    }
}

/// 位并行求值器：把表达式在全部 `3^n` 组赋值上求值。
#[derive(Debug, Clone)]
pub struct Evaluator {
    algebra: Algebra,
    variables: Vec<String>,
}

impl Evaluator {
    /// 新建求值器。
    pub fn new(algebra: Algebra) -> Self {
        Self {
            algebra,
            variables: Vec::new(),
        }
    }

    /// 语义体系。
    pub fn algebra(&self) -> Algebra {
        self.algebra
    }

    /// 变量名列表。
    pub fn variables(&self) -> &[String] {
        &self.variables
    }

    /// 登记变量（重复登记返回原编号）。
    pub fn declare(&mut self, name: impl Into<String>) -> usize {
        let name = name.into();
        if let Some(idx) = self.variables.iter().position(|v| *v == name) {
            return idx;
        }
        self.variables.push(name);
        self.variables.len() - 1
    }

    /// 赋值总数 `3^n`。
    pub fn assignment_count(&self) -> u64 {
        3u64.saturating_pow(self.variables.len() as u32)
    }

    /// 构造一个 64 组分块中各变量的位平面。
    ///
    /// `base` 是该分块首个赋值的编号（必为 64 的倍数）。
    /// 变量 `i` 的取值由赋值编号的第 `i` 个三进制位决定
    /// （`0 → 0`，`1 → 1`，`2 → X`）。
    pub fn chunk_planes(&self, base: u64) -> Vec<TriBit> {
        let mut planes = Vec::with_capacity(self.variables.len());
        for i in 0..self.variables.len() {
            let mut ones = 0u64;
            let mut unknowns = 0u64;
            let pow = 3u64.saturating_pow(i as u32);
            for bit in 0..64u32 {
                let group = base + bit as u64;
                let digit = (group / pow) % 3;
                match digit {
                    1 => ones |= 1u64 << bit,
                    2 => unknowns |= 1u64 << bit,
                    _ => {}
                }
            }
            planes.push(TriBit {
                ones: BitPlane(ones),
                unknowns: BitPlane(unknowns),
            });
        }
        planes
    }

    /// 在全部 `3^n` 组赋值上求值。
    ///
    /// 内部按 64 组一块做位并行计算，再展开为逐组结果。
    pub fn evaluate<F>(&self, f: F) -> TruthTable
    where
        F: Fn(&[TriBit]) -> TriBit,
    {
        let total = self.assignment_count();
        let mut values = Vec::with_capacity(total as usize);
        let mut base = 0u64;
        while base < total {
            let planes = self.chunk_planes(base);
            let res = f(&planes);
            let take = (total - base).min(64) as u32;
            for bit in 0..take {
                values.push(res.get(bit));
            }
            base += 64;
        }
        TruthTable {
            variables: self.variables.clone(),
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(v: Ternary) -> TriBit {
        TriBit::constant(v)
    }

    #[test]
    fn not_matches_scalar() {
        for a in Ternary::ALL {
            assert_eq!(tri(a).not().get(0), Algebra::Kleene.not(a), "NOT {a}");
        }
    }

    #[test]
    fn and_or_match_scalar_for_all_algebras() {
        for alg in Algebra::ALL {
            for a in Ternary::ALL {
                for b in Ternary::ALL {
                    assert_eq!(
                        tri(a).and(tri(b), alg).get(0),
                        alg.and(a, b),
                        "{alg} {a}&{b}"
                    );
                    assert_eq!(tri(a).or(tri(b), alg).get(0), alg.or(a, b), "{alg} {a}|{b}");
                }
            }
        }
    }

    #[test]
    fn strong_add_matches_scalar_for_all_algebras() {
        for alg in Algebra::ALL {
            for a in Ternary::ALL {
                for b in Ternary::ALL {
                    assert_eq!(
                        tri(a).strong_add(tri(b), alg).get(0),
                        alg.strong_add(a, b),
                        "{alg} {a}+{b}"
                    );
                }
            }
        }
    }

    #[test]
    fn strong_and_matches_scalar_for_all_algebras() {
        for alg in Algebra::ALL {
            for a in Ternary::ALL {
                for b in Ternary::ALL {
                    assert_eq!(
                        tri(a).strong_and(tri(b), alg).get(0),
                        alg.strong_and(a, b),
                        "{alg} {a}*{b}"
                    );
                }
            }
        }
    }

    #[test]
    fn ops_never_produce_illegal_rails() {
        for alg in Algebra::ALL {
            for a in Ternary::ALL {
                for b in Ternary::ALL {
                    for r in [
                        tri(a).and(tri(b), alg),
                        tri(a).or(tri(b), alg),
                        tri(a).strong_add(tri(b), alg),
                    ] {
                        assert_eq!(r.ones.0 & r.unknowns.0, 0, "{alg} {a} {b} 产生了 (1,1)");
                    }
                }
            }
        }
    }

    #[test]
    fn implies_matches_scalar_for_all_algebras() {
        for alg in Algebra::ALL {
            for a in Ternary::ALL {
                for b in Ternary::ALL {
                    assert_eq!(
                        tri(a).implies(tri(b), alg).get(0),
                        alg.implies(a, b),
                        "{alg} {a}=>{b}"
                    );
                }
            }
        }
    }

    #[test]
    fn equiv_matches_scalar_for_all_algebras() {
        for alg in Algebra::ALL {
            for a in Ternary::ALL {
                for b in Ternary::ALL {
                    assert_eq!(
                        tri(a).equiv(tri(b), alg).get(0),
                        alg.equiv(a, b),
                        "{alg} {a}<=>{b}"
                    );
                }
            }
        }
    }

    #[test]
    fn bitwise_ops_agree_across_64_lanes() {
        // 构造一个混合 0/1/X 的 64 位平面，逐位与标量结果比对
        let mut ones = 0u64;
        let mut unknowns = 0u64;
        for i in 0..64u32 {
            match i % 3 {
                1 => ones |= 1u64 << i,
                2 => unknowns |= 1u64 << i,
                _ => {}
            }
        }
        let t = TriBit {
            ones: BitPlane(ones),
            unknowns: BitPlane(unknowns),
        };
        let other = TriBit {
            ones: BitPlane(0x5555_5555_5555_5555),
            unknowns: BitPlane(0x0f0f_0f0f_0f0f_0f0f),
        };
        for alg in Algebra::ALL {
            let got = t.and(other, alg);
            for i in 0..64u32 {
                assert_eq!(
                    got.get(i),
                    alg.and(t.get(i), other.get(i)),
                    "{alg} lane {i}"
                );
            }
        }
    }

    #[test]
    fn evaluator_covers_all_assignments() {
        let mut ev = Evaluator::new(Algebra::Kleene);
        ev.declare("a");
        ev.declare("b");
        assert_eq!(ev.assignment_count(), 9);
        let tt = ev.evaluate(|p| p[0]);
        assert_eq!(tt.values.len(), 9);
        // 三进制混合序：第 0 组 a=0，第 1 组 a=1，第 2 组 a=X
        assert_eq!(tt.value_at(0), Some(Ternary::False));
        assert_eq!(tt.value_at(1), Some(Ternary::True));
        assert_eq!(tt.value_at(2), Some(Ternary::Unknown));
        assert_eq!(tt.value_at(3), Some(Ternary::False));
    }

    #[test]
    fn evaluator_spans_multiple_chunks() {
        // 4 个变量 → 81 组赋值，跨越 64 组边界
        let mut ev = Evaluator::new(Algebra::Kleene);
        for name in ["a", "b", "c", "d"] {
            ev.declare(name);
        }
        assert_eq!(ev.assignment_count(), 81);
        let tt = ev.evaluate(|p| p[3]);
        assert_eq!(tt.values.len(), 81);
        for (i, v) in tt.values.iter().enumerate() {
            let digit = (i as u64 / 27) % 3;
            let expect = match digit {
                0 => Ternary::False,
                1 => Ternary::True,
                _ => Ternary::Unknown,
            };
            assert_eq!(*v, expect, "group {i}");
        }
    }
}
