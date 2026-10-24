//! TTX2 的模型域：三值层/模型、双轨打包、偏置基 3 打包、MOD 规范校验流。
//!
//! 「GPU-ready」是实义：pos/neg u32 字就是 `trit-gpu` matmul 内核绑定的
//! storage buffer 布局（行整字，bit = `1 << (i % 32)`），上传零重排。

use trit_core::value::Ternary;

use crate::TextureError;

/// 每字 trit 数（与 trit-gpu 的 `TRITS_PER_WORD` 一致，就地声明互不依赖）。
pub const TRITS_PER_WORD: usize = 32;
/// 偏置基 3 打包密度（5 trit/字节，243 ≤ 256）。
pub(crate) const TRITS_PER_BIAS_BYTE: usize = 5;

/// 一层三值全连接：`out = sign(W · in + b)`，权重/偏置 ∈ {-1, 0, +1}。
///
/// 权重以双轨位平面存储（`pos` 装 +1 的位、`neg` 装 -1 的位）：第 `row` 行第
/// `col` 列的权重位于 `pos[row · words_per_row + col / 32]` 的 `1 << (col % 32)`
/// 位——与 GPU 内核消费的布局逐位一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TritModelLayer {
    /// 输入维数。
    pub inputs: usize,
    /// 输出维数。
    pub outputs: usize,
    /// 正轨字（行主序，每行 `ceil(inputs / 32)` 个整字）。
    pub pos: Vec<u32>,
    /// 负轨字（与 `pos` 同形）。
    pub neg: Vec<u32>,
    /// 偏置，取值 `-1/0/1`。
    pub bias: Vec<i8>,
}

impl TritModelLayer {
    /// 从行主序权重构造（校验取值域与形状），打包成双轨布局。
    pub fn new(weights: &[Vec<i8>], bias: &[i8]) -> Result<Self, TextureError> {
        let outputs = weights.len();
        let inputs = weights.first().map_or(0, Vec::len);
        if outputs == 0 || inputs == 0 {
            return Err(TextureError::TtxModelShape {
                layer: 0,
                message: "inputs/outputs 必须非零".to_string(),
            });
        }
        let words = inputs.div_ceil(TRITS_PER_WORD);
        let mut pos = vec![0u32; outputs * words];
        let mut neg = vec![0u32; outputs * words];
        for (row, row_weights) in weights.iter().enumerate() {
            if row_weights.len() != inputs {
                return Err(TextureError::TtxModelShape {
                    layer: 0,
                    message: format!("权重列数应为 {inputs}，实际 {}", row_weights.len()),
                });
            }
            for (col, value) in row_weights.iter().enumerate() {
                let bit = 1u32 << (col % TRITS_PER_WORD);
                match value {
                    1 => pos[row * words + col / TRITS_PER_WORD] |= bit,
                    -1 => neg[row * words + col / TRITS_PER_WORD] |= bit,
                    0 => {}
                    other => {
                        return Err(TextureError::TtxModelShape {
                            layer: 0,
                            message: format!("权重只能是 -1/0/1，实际 {other}"),
                        });
                    }
                }
            }
        }
        if bias.len() != outputs {
            return Err(TextureError::TtxModelShape {
                layer: 0,
                message: format!("偏置长度应为 {outputs}，实际 {}", bias.len()),
            });
        }
        for value in bias {
            if !(-1..=1).contains(value) {
                return Err(TextureError::TtxModelShape {
                    layer: 0,
                    message: format!("偏置只能是 -1/0/1，实际 {value}"),
                });
            }
        }
        Ok(Self {
            inputs,
            outputs,
            pos,
            neg,
            bias: bias.to_vec(),
        })
    }

    /// 每行字数。
    pub fn words_per_row(&self) -> usize {
        self.inputs.div_ceil(TRITS_PER_WORD)
    }

    /// 从双轨解出第 `row` 行第 `col` 列的权重。
    pub fn weight(&self, row: usize, col: usize) -> i8 {
        let words = self.words_per_row();
        let mask = 1u32 << (col % TRITS_PER_WORD);
        let word = row * words + col / TRITS_PER_WORD;
        if self.pos[word] & mask != 0 {
            1
        } else if self.neg[word] & mask != 0 {
            -1
        } else {
            0
        }
    }

    pub(crate) fn validate(&self, layer: usize) -> Result<(), TextureError> {
        if self.inputs == 0 || self.outputs == 0 {
            return Err(TextureError::TtxModelShape {
                layer,
                message: "inputs/outputs 必须非零".to_string(),
            });
        }
        let words = self.words_per_row();
        let expected = self
            .outputs
            .checked_mul(words)
            .ok_or(TextureError::TtxModelShape {
                layer,
                message: "outputs × words 溢出".to_string(),
            })?;
        if self.pos.len() != expected || self.neg.len() != expected {
            return Err(TextureError::TtxModelShape {
                layer,
                message: format!(
                    "双轨字数应为 {expected}，实际 {}/{}",
                    self.pos.len(),
                    self.neg.len()
                ),
            });
        }
        for (word, (pos, neg)) in self.pos.iter().zip(&self.neg).enumerate() {
            if pos & neg != 0 {
                return Err(TextureError::TtxRailOverlap { layer, word });
            }
        }
        if self.bias.len() != self.outputs {
            return Err(TextureError::TtxModelShape {
                layer,
                message: format!("偏置长度应为 {}，实际 {}", self.outputs, self.bias.len()),
            });
        }
        for value in &self.bias {
            if !(-1..=1).contains(value) {
                return Err(TextureError::TtxModelShape {
                    layer,
                    message: format!("偏置只能是 -1/0/1，实际 {value}"),
                });
            }
        }
        Ok(())
    }
}

/// 三值模型：层按顺序串联（层间衔接由写者保证，容器只校验层内形状）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TritModel {
    /// 各层。
    pub layers: Vec<TritModelLayer>,
}

impl TritModel {
    /// 构造并校验（≥1 层，逐层形状/取值/双轨不变式）。
    pub fn new(layers: Vec<TritModelLayer>) -> Result<Self, TextureError> {
        if layers.is_empty() {
            return Err(TextureError::TtxEmpty);
        }
        for (index, layer) in layers.iter().enumerate() {
            layer.validate(index)?;
        }
        Ok(Self { layers })
    }
}

fn bias_digit(value: i8) -> u32 {
    // digit = value + 1 ∈ {0,1,2}（-1→0，0→1，+1→2）。
    (value + 1) as u32
}

pub(crate) fn pack_bias(bias: &[i8]) -> Vec<u8> {
    let mut out = vec![0u8; bias.len().div_ceil(TRITS_PER_BIAS_BYTE)];
    for (index, value) in bias.iter().enumerate() {
        out[index / TRITS_PER_BIAS_BYTE] +=
            (bias_digit(*value) * 3u32.pow((index % TRITS_PER_BIAS_BYTE) as u32)) as u8;
    }
    out
}

pub(crate) fn unpack_bias(bytes: &[u8], outputs: usize) -> Vec<i8> {
    let mut out = Vec::with_capacity(outputs);
    for index in 0..outputs {
        let digit = (bytes[index / TRITS_PER_BIAS_BYTE] as u32 / 3u32.pow((index % 5) as u32)) % 3;
        out.push(digit as i8 - 1);
    }
    out
}

/// MOD 的规范 trit 流：每层每行的 W·32 个权重 trit（含零填充尾）+ 偏置 trit。
pub(crate) fn model_trits(model: &TritModel) -> Vec<Ternary> {
    let digit_of = |pos_bit: bool, neg_bit: bool| {
        // +1→2（True），-1→0（False），0→1（Unknown）——与 bias_digit 同映射。
        match (pos_bit, neg_bit) {
            (true, _) => Ternary::True,
            (false, true) => Ternary::False,
            (false, false) => Ternary::Unknown,
        }
    };
    let mut trits = Vec::new();
    for layer in &model.layers {
        let words = layer.words_per_row();
        for word in 0..layer.outputs * words {
            for bit in 0..TRITS_PER_WORD {
                let mask = 1u32 << bit;
                trits.push(digit_of(
                    layer.pos[word] & mask != 0,
                    layer.neg[word] & mask != 0,
                ));
            }
        }
        for value in &layer.bias {
            trits.push(match bias_digit(*value) {
                0 => Ternary::False,
                1 => Ternary::Unknown,
                _ => Ternary::True,
            });
        }
    }
    trits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ttx2::{KIND_MOD, MAGIC, TtxDocument, from_ttx2, to_ttx2};

    #[test]
    fn model_roundtrip_pins_the_dual_rail_bit_layout() {
        // 手算常量钉死位布局（GPU 内核消费的就是这份布局）：
        // 行 [ +1, -1, 0, +1, …第 32 列再一个 +1 ]（inputs=33 → W=2）：
        // pos 字 0 = bit0|bit3 = 0b1001 = 9，字 1 = bit0 = 1；neg 字 0 = bit1 = 2。
        let mut row0 = vec![0i8; 33];
        row0[0] = 1;
        row0[1] = -1;
        row0[3] = 1;
        row0[32] = 1;
        let mut row1 = vec![0i8; 33];
        row1[2] = -1;
        let layer = TritModelLayer::new(&[row0, row1], &[0, 1]).unwrap();
        assert_eq!(layer.words_per_row(), 2);
        assert_eq!(layer.pos, vec![0b1001, 0b0000_0001, 0, 0]);
        assert_eq!(layer.neg, vec![0b0010, 0, 0b0100, 0]);
        for (row, col, expected) in [(0, 0, 1), (0, 1, -1), (0, 2, 0), (0, 32, 1), (1, 2, -1)] {
            assert_eq!(layer.weight(row, col), expected, "({row},{col})");
        }
        let model = TritModel::new(vec![layer]).unwrap();
        let bytes = to_ttx2(&TtxDocument::Model(model.clone())).unwrap();
        assert_eq!(&bytes[..4], &MAGIC);
        assert_eq!(bytes[4], KIND_MOD);
        let TtxDocument::Model(back) = from_ttx2(&bytes).unwrap() else {
            panic!("kind 漂移");
        };
        assert_eq!(back, model);
    }

    #[test]
    fn model_layer_rejects_bad_rows_values_and_bias() {
        assert!(matches!(
            TritModelLayer::new(&[], &[0]),
            Err(TextureError::TtxModelShape { .. })
        ));
        assert!(matches!(
            TritModelLayer::new(&[vec![0, 1], vec![1]], &[0, 0]),
            Err(TextureError::TtxModelShape { .. })
        ));
        assert!(matches!(
            TritModelLayer::new(&[vec![2, 0]], &[0]),
            Err(TextureError::TtxModelShape { .. })
        ));
        assert!(matches!(
            TritModelLayer::new(&[vec![0, 1]], &[5]),
            Err(TextureError::TtxModelShape { .. })
        ));
        // 轨重叠（直接构造绕过 new）：必须结构化拒绝。
        let overlapped = TritModelLayer {
            inputs: 32,
            outputs: 1,
            pos: vec![1],
            neg: vec![1],
            bias: vec![0],
        };
        assert!(matches!(
            TritModel::new(vec![overlapped]),
            Err(TextureError::TtxRailOverlap { layer: 0, word: 0 })
        ));
    }
}
