//! 位切片三值向量：与二进制位平面零拷贝互操作。
//!
//! 三值是双导轨 `(ones, unknowns)`：`(0,0)=False`、`(1,0)=True`、`(0,1)=Unknown`。
//! 二进制数据就是 `unknowns` 缺失（`None`）的 `TriVec`：
//!
//! ```
//! use trit_core::{TriVec, Ternary};
//! let words = vec![0b1010u64];
//! let v = TriVec::from_binary(words, 4);
//! assert_eq!(v.unknowns(), None);
//! assert_eq!(v.get(1), Ternary::True);
//! assert_eq!(v.get(0), Ternary::False);
//! assert_eq!(v.to_bools().unwrap(), vec![false, true, false, true]);
//! ```
//!
//! `from_binary` 直接接管 `Vec<u64>`，不复制、不分配 unknown 平面；
//! 只有显式写入 `Unknown` 时才分配第二个平面。

use crate::algebra::Algebra;
use crate::bits::TriBit;
use crate::value::Ternary;

/// `TriVec` 形状错误。
#[derive(Debug, thiserror::Error)]
#[error("TriVec 长度不匹配：{left} vs {right}")]
pub struct ShapeError {
    /// 左操作数长度。
    pub left: usize,
    /// 右操作数长度。
    pub right: usize,
}

/// 位切片的三值向量。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriVec {
    len: usize,
    ones: Vec<u64>,
    unknowns: Option<Vec<u64>>,
}

impl TriVec {
    /// 零拷贝接管二进制位平面：`ones` 的位为 1 表示 `True`，0 表示 `False`。
    ///
    /// `ones` 必须至少覆盖 `len` 位；末尾多余位必须是 0。
    pub fn from_binary(ones: Vec<u64>, len: usize) -> Self {
        assert!(
            ones.len() * 64 >= len,
            "位平面不足以覆盖 {len} 位：{} words",
            ones.len()
        );
        Self {
            len,
            ones,
            unknowns: None,
        }
    }

    /// 从 `bool` 切片打包。
    pub fn from_bools(bits: &[bool]) -> Self {
        let mut ones = vec![0u64; bits.len().div_ceil(64)];
        for (i, &bit) in bits.iter().enumerate() {
            if bit {
                ones[i / 64] |= 1u64 << (i % 64);
            }
        }
        Self {
            len: bits.len(),
            ones,
            unknowns: None,
        }
    }

    /// 从标量三值切片打包。
    pub fn from_ternary(values: &[Ternary]) -> Self {
        let mut ones = vec![0u64; values.len().div_ceil(64)];
        let mut unknowns = vec![0u64; ones.len()];
        let mut any_unknown = false;
        for (i, &value) in values.iter().enumerate() {
            match value {
                Ternary::False => {}
                Ternary::True => ones[i / 64] |= 1u64 << (i % 64),
                Ternary::Unknown => {
                    unknowns[i / 64] |= 1u64 << (i % 64);
                    any_unknown = true;
                }
            }
        }
        Self {
            len: values.len(),
            ones,
            unknowns: any_unknown.then_some(unknowns),
        }
    }

    /// 若全部是二值，解包成 `bool`；含 `Unknown` 时返回 `None`。
    pub fn to_bools(&self) -> Option<Vec<bool>> {
        if !self.is_binary() {
            return None;
        }
        let mut out = Vec::with_capacity(self.len);
        for i in 0..self.len {
            out.push(self.get(i).is_true());
        }
        Some(out)
    }

    /// 长度。
    pub fn len(&self) -> usize {
        self.len
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 位平面字数。
    pub fn words(&self) -> usize {
        self.ones.len()
    }

    /// 是否为纯二值（没有 `Unknown`）。
    pub fn is_binary(&self) -> bool {
        self.unknowns
            .as_ref()
            .is_none_or(|planes| planes.iter().all(|&w| w == 0))
    }

    /// `ones` 位平面（只读）。
    pub fn ones(&self) -> &[u64] {
        &self.ones
    }

    /// `unknowns` 位平面；`None` 表示纯二值。
    pub fn unknowns(&self) -> Option<&[u64]> {
        self.unknowns.as_deref()
    }

    /// 取第 `i` 个三值。
    pub fn get(&self, i: usize) -> Ternary {
        assert!(i < self.len, "索引 {i} 越界（len={}）", self.len);
        let ones = (self.ones[i / 64] >> (i % 64)) & 1;
        let unknowns = self
            .unknowns
            .as_ref()
            .map_or(0, |planes| (planes[i / 64] >> (i % 64)) & 1);
        match (ones, unknowns) {
            (1, 0) => Ternary::True,
            (0, 1) => Ternary::Unknown,
            _ => Ternary::False,
        }
    }

    /// 写第 `i` 个三值；首次写 `Unknown` 时分配 unknown 平面。
    pub fn set(&mut self, i: usize, value: Ternary) {
        assert!(i < self.len, "索引 {i} 越界（len={}）", self.len);
        let word = i / 64;
        let bit = 1u64 << (i % 64);
        match value {
            Ternary::False => {
                self.ones[word] &= !bit;
                if let Some(planes) = &mut self.unknowns {
                    planes[word] &= !bit;
                }
            }
            Ternary::True => {
                self.ones[word] |= bit;
                if let Some(planes) = &mut self.unknowns {
                    planes[word] &= !bit;
                }
            }
            Ternary::Unknown => {
                let planes = self
                    .unknowns
                    .get_or_insert_with(|| vec![0u64; self.ones.len()]);
                planes[word] |= bit;
                self.ones[word] &= !bit;
            }
        }
    }

    /// `Unknown` 的个数。
    pub fn count_unknowns(&self) -> usize {
        self.unknowns.as_ref().map_or(0, |planes| {
            planes.iter().map(|w| w.count_ones() as usize).sum()
        })
    }

    /// 取整个字（64 路）的位并行表示。
    pub fn word(&self, word: usize) -> TriBit {
        assert!(word < self.ones.len(), "字索引越界");
        TriBit {
            ones: crate::bits::BitPlane(self.ones[word]),
            unknowns: crate::bits::BitPlane(
                self.unknowns.as_ref().map_or(0, |planes| planes[word]),
            ),
        }
    }

    /// 原地取反：`True`/`False` 互换，`Unknown` 不变。
    pub fn not(&mut self) {
        let mask = last_mask(self.len);
        for (w, ones) in self.ones.iter_mut().enumerate() {
            let x = self.unknowns.as_ref().map_or(0, |planes| planes[w]);
            *ones = !(*ones | x) & mask_of(w, self.len, mask);
        }
    }

    /// 原地与。K3/Ł3 用格序，B3 用毒值。
    pub fn and_assign(&mut self, other: &Self, alg: Algebra) -> Result<(), ShapeError> {
        self.check_shape(other)?;
        let mask = last_mask(self.len);
        let mut new_unknowns = Vec::with_capacity(self.ones.len());
        for w in 0..self.ones.len() {
            let o1 = self.ones[w];
            let o2 = other.ones[w];
            let x1 = self.unknowns.as_ref().map_or(0, |p| p[w]);
            let x2 = other.unknowns.as_ref().map_or(0, |p| p[w]);
            let m = mask_of(w, self.len, mask);
            let (o, x) = match alg {
                Algebra::Bochvar => {
                    let x_out = (x1 | x2) & m;
                    ((o1 & o2) & !x_out & m, x_out)
                }
                Algebra::Kleene | Algebra::Lukasiewicz => {
                    let f1 = !o1 & !x1;
                    let f2 = !o2 & !x2;
                    let o_out = (o1 & o2) & m;
                    let f_out = (f1 | f2) & m;
                    (o_out, !(o_out | f_out) & m)
                }
            };
            self.ones[w] = o;
            new_unknowns.push(x);
        }
        self.replace_unknowns(new_unknowns);
        Ok(())
    }

    /// 原地或。K3/Ł3 用格序，B3 用毒值。
    pub fn or_assign(&mut self, other: &Self, alg: Algebra) -> Result<(), ShapeError> {
        self.check_shape(other)?;
        let mask = last_mask(self.len);
        let mut new_unknowns = Vec::with_capacity(self.ones.len());
        for w in 0..self.ones.len() {
            let o1 = self.ones[w];
            let o2 = other.ones[w];
            let x1 = self.unknowns.as_ref().map_or(0, |p| p[w]);
            let x2 = other.unknowns.as_ref().map_or(0, |p| p[w]);
            let m = mask_of(w, self.len, mask);
            let (o, x) = match alg {
                Algebra::Bochvar => {
                    let x_out = (x1 | x2) & m;
                    ((o1 | o2) & !x_out & m, x_out)
                }
                Algebra::Kleene | Algebra::Lukasiewicz => {
                    let f1 = !o1 & !x1;
                    let f2 = !o2 & !x2;
                    let o_out = (o1 | o2) & m;
                    let f_out = (f1 & f2) & m;
                    (o_out, !(o_out | f_out) & m)
                }
            };
            self.ones[w] = o;
            new_unknowns.push(x);
        }
        self.replace_unknowns(new_unknowns);
        Ok(())
    }

    fn replace_unknowns(&mut self, planes: Vec<u64>) {
        if planes.iter().all(|&w| w == 0) {
            self.unknowns = None;
        } else {
            self.unknowns = Some(planes);
        }
    }

    fn check_shape(&self, other: &Self) -> Result<(), ShapeError> {
        if self.len == other.len {
            Ok(())
        } else {
            Err(ShapeError {
                left: self.len,
                right: other.len,
            })
        }
    }
}

fn last_mask(len: usize) -> u64 {
    match len % 64 {
        0 => u64::MAX,
        r => (1u64 << r) - 1,
    }
}

fn mask_of(word: usize, len: usize, last: u64) -> u64 {
    let words = len.div_ceil(64);
    if word + 1 == words { last } else { u64::MAX }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(len: usize, seed: u64) -> Vec<Ternary> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                match (x >> 32) % 3 {
                    0 => Ternary::False,
                    1 => Ternary::True,
                    _ => Ternary::Unknown,
                }
            })
            .collect()
    }

    #[test]
    fn binary_view_is_zero_copy_and_roundtrips() {
        for len in [0usize, 1, 63, 64, 65, 130] {
            let bits: Vec<bool> = (0..len).map(|i| i % 3 == 0).collect();
            let v = TriVec::from_bools(&bits);
            assert_eq!(v.unknowns(), None);
            assert!(v.is_binary());
            assert_eq!(v.to_bools().unwrap(), bits);
            let words = v.ones().to_vec();
            let moved = TriVec::from_binary(words, len);
            assert_eq!(moved.to_bools().unwrap(), bits);
        }
    }

    #[test]
    fn unknown_allocates_lazily() {
        let mut v = TriVec::from_bools(&[true, false]);
        assert_eq!(v.unknowns(), None);
        v.set(1, Ternary::Unknown);
        assert!(v.unknowns().is_some());
        assert_eq!(v.get(1), Ternary::Unknown);
        assert_eq!(v.to_bools(), None);
        assert_eq!(v.count_unknowns(), 1);
    }

    #[test]
    fn bit_ops_match_scalar() {
        for alg in Algebra::ALL {
            let a = sample(130, 7);
            let b = sample(130, 11);
            let mut va = TriVec::from_ternary(&a);
            let vb = TriVec::from_ternary(&b);
            va.and_assign(&vb, alg).unwrap();
            for i in 0..a.len() {
                assert_eq!(va.get(i), alg.and(a[i], b[i]), "{alg:?} and at {i}");
            }
            let mut va = TriVec::from_ternary(&a);
            va.or_assign(&vb, alg).unwrap();
            for i in 0..a.len() {
                assert_eq!(va.get(i), alg.or(a[i], b[i]), "{alg:?} or at {i}");
            }
        }
    }

    #[test]
    fn not_matches_scalar() {
        let a = sample(96, 3);
        let mut v = TriVec::from_ternary(&a);
        v.not();
        for (i, &value) in a.iter().enumerate() {
            assert_eq!(v.get(i), Algebra::Kleene.not(value), "not at {i}");
        }
    }

    #[test]
    fn shape_mismatch_is_an_error() {
        let mut a = TriVec::from_bools(&[true, false]);
        let b = TriVec::from_bools(&[true]);
        assert!(a.and_assign(&b, Algebra::Kleene).is_err());
    }
}
