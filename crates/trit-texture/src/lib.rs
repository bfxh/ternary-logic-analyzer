//! 三进制块纹理压缩实验层（`publish = false`，不属于稳定 API）。
//!
//! 格式 **TT3**：**8×8 = 64 纹素**的块，块头是 RGB565 端点对（4 字节），
//! 每纹素 **1 个 trit** 索引 3 级调色板 `{e0, mid, e1}`；64 个 trit 按
//! 5 trit/字节打包成 13 字节 ⇒ 每块 **17 字节 = 2.125 bpp**。
//!
//! 对比基线（同一块结构、同一 3 级调色板，索引改 2 bit/纹素）：
//! 64×2 bit = 16 字节 ⇒ 每块 20 字节 = 2.5 bpp。**等调色板下 trit 载荷
//! 13 字节 < 二进制 16 字节（省 18.75% 载荷 / 15% 码率）**——可复算，
//! `cargo run -p trit-texture` 打表。
//!
//! 四类边界如实记账（见 docs/TERNARY-BOUNDARIES.md）：
//! - **A 原生红利**：3 级调色板索引 1 trit（log₂3 ≈ 1.585 bit）装得下，2 bit 装不满
//!   ——64 纹素取整后 13 字节 vs 16 字节（省 18.75% 载荷）。
//! - **B 模拟税**：64 纹素占 65 槽，打包粒度税 **1.6%**（v1 的 4×10 把这项清零，
//!   却换来 25% 的 mip 补边，得不偿失）；端点被迫走 RGB565 仍是量化税。
//! - **C 生态路径依赖**：RGB565 端点对抄自 BC1 的生态格式，不是三进制选择；
//!   无硬件采样器是自有格式的宿命。
//! - **D 基数无关**：最近调色板色搜索、端点优化（最小二乘精修）与进制无关。
//!
//! ## 块形状为什么从 4×10 改成 8×8
//!
//! 4×10 的唯一卖点是 40 恰被 5 整除、载荷零浪费。但 10 不是 2 的幂，而 mip 每级
//! 高度减半仍须 %10，于是正常 2 的幂纹理必须补到 `10·2^(n-1)` 的倍数——**恒定
//! 25% 填充**，名义 2.40 bpp 实际 3.00 bpp。8×8 付 1.6% 打包税，换来全程合法
//! 的 mip 链与 2.125 bpp。**「零槽位浪费」是局部最优，mip 合法性才是全局最优。**

#![forbid(unsafe_code)]

use trit_core::value::Ternary;

mod endpoints;
pub mod ttx;
pub mod ttx2;

use endpoints::{block_endpoints, nearest_index, palette_from_endpoints, refine_endpoints};

/// 块宽：**8×8 方块**（v2 起）。
///
/// > 教训（v1 的 4×10）：40 纹素恰被 5 整除 ⇒ 载荷 8 字节零浪费，看着很美。
/// > 但 10 不是 2 的幂，而每个 mip 级高度都要 %10，减半后很快不合法：正常 2 的幂
/// > 纹理（256/512/1024/2048）**没有一个**是 10 的倍数，必须补到 320/640/1280/2560
/// > ——**恒定 25% 高度填充**。名义 2.40 bpp 实际 3.00 bpp，相对 BC1 从「省 40%」
/// > 掉到「省 25%」。
/// >
/// > 取舍：2^n 永远不被 5 整除，**「零槽位浪费」与「mip 友好」不可兼得**。选后者。
pub const BLOCK_WIDTH: usize = 8;
/// 块高：8×8 方块，与宽相同 ⇒ 任意 2 的幂尺寸全程可 mip 到 1×1。
pub const BLOCK_HEIGHT: usize = 8;
/// 每块纹素数。
pub const BLOCK_PIXELS: usize = BLOCK_WIDTH * BLOCK_HEIGHT;
/// 每块 trit 索引数。
pub const TRITS_PER_BLOCK: usize = BLOCK_PIXELS;
/// trit 载荷字节数：ceil(64/5) = 13（65 槽用 64，打包税 **1.6%**；5 trit/字节 = 3^5 = 243 ≤ 256）。
///
/// 用 1.6% 的打包税换掉 v1 的 25% mip 补边——净赚 23.4 个百分点。
pub const TRIT_PAYLOAD_BYTES: usize = TRITS_PER_BLOCK.div_ceil(5);
/// 块总字节数：4 字节端点 + 13 字节 trit 载荷 = 17 字节 ⇒ **2.125 bpp**。
pub const TRIT_BLOCK_BYTES: usize = 4 + TRIT_PAYLOAD_BYTES;
/// 二进制基线载荷字节数：64 纹素 × 2 bit = 128 bit = 16 字节。
pub const BINARY_PAYLOAD_BYTES: usize = BLOCK_PIXELS * 2 / 8;
/// 二进制基线块总字节数：4 + 16 = 20 字节 ⇒ 2.5 bpp。
pub const BINARY_BLOCK_BYTES: usize = 4 + BINARY_PAYLOAD_BYTES;
/// 纹素数上限（解析/分配之前拒绝）。
pub const MAX_TEXTURE_PIXELS: usize = 1 << 22;

/// 纹理压缩错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TextureError {
    /// 宽高必须分别是 4 / 8 的倍数。
    #[error("宽 {width} 必须是 {BLOCK_WIDTH} 的倍数，高 {height} 必须是 {BLOCK_HEIGHT} 的倍数")]
    Shape {
        /// 传入的宽。
        width: usize,
        /// 传入的高。
        height: usize,
    },
    /// 输入长度与宽高不符（RGBA，每纹素 4 字节）。
    #[error("输入 {actual} 字节，宽高要求 {expected} 字节（RGBA）")]
    SizeMismatch {
        /// 实际字节数。
        actual: usize,
        /// 期望字节数。
        expected: usize,
    },
    /// 纹理过大。
    #[error("纹理 {pixels} 纹素，超过上限 {MAX_TEXTURE_PIXELS}")]
    TooLarge {
        /// 纹素数。
        pixels: usize,
    },
    /// 解码数据的块数与宽高不符。
    #[error("块数 {actual} 与宽高要求的 {expected} 不符")]
    BlockCount {
        /// 实际块数。
        actual: usize,
        /// 期望块数。
        expected: usize,
    },
    /// TTX 魔数不是 "TTX1"。
    #[error("TTX 魔数不符：{0:?}")]
    TtxMagic([u8; 4]),
    /// TTX 容器截断（不足头部/载荷/校验尾）。
    #[error("TTX 容器只有 {actual} 字节，长度不符")]
    TtxTruncated {
        /// 实际字节数。
        actual: usize,
    },
    /// TTX 头部声明的块数与宽高推出的块数不符。
    #[error("TTX 声明 {declared} 块，宽高要求 {expected} 块")]
    TtxBlockCount {
        /// 声明的块数。
        declared: u32,
        /// 期望块数。
        expected: usize,
    },
    /// TTX 的 GF(3) 校验和不符。
    #[error("TTX 校验和不符：存 {stored:#04x}，算得 {expected:#04x}")]
    TtxChecksum {
        /// 存的校验字节。
        stored: u8,
        /// 重算的校验字节。
        expected: u8,
    },
    /// TTX1 是 v1（4×10 / 12 字节块）的冻结格式，当前块布局已不是 v1。
    ///
    /// 用新布局读写 "TTX1" 会让旧文件被静默误读——按「绝不猜」的纪律显式拒绝。
    /// v2 的纹理请走 TTX2（kind=TEX）。
    #[error(
        "TTX1 只支持 v1 的 4×10 / 12 字节块，当前块是 {width}×{height} / {bytes} 字节；v2 纹理请用 TTX2"
    )]
    TtxV1LayoutMismatch {
        /// 当前块宽。
        width: usize,
        /// 当前块高。
        height: usize,
        /// 当前块字节数。
        bytes: usize,
    },
    /// TTX2 的 kind 不是 0（TEX）/ 1（MOD）。
    #[error("TTX2 未知 kind：{0}")]
    TtxKind(u8),
    /// TTX2 的 flags/reserved 非零（前向兼容显式拒绝，不静默猜）。
    #[error("TTX2 flags/reserved 必须为零，实际 flags={flags} reserved={reserved}")]
    TtxFlags {
        /// flags 字节。
        flags: u8,
        /// reserved 双字节。
        reserved: u16,
    },
    /// TTX2 段没有任何 mip 级 / 层。
    #[error("TTX2 段没有任何 mip 级/层")]
    TtxEmpty,
    /// TTX2 的 mip 级未严格小于上一级。
    #[error("TTX2 第 {level} 级 mip 未严格小于上一级（宽高须不增且至少一维递减）")]
    TtxMip {
        /// 违规的 mip 级下标。
        level: usize,
    },
    /// TTX2 模型层形状不合法。
    #[error("TTX2 第 {layer} 层形状不合法：{message}")]
    TtxModelShape {
        /// 层下标。
        layer: usize,
        /// 说明。
        message: String,
    },
    /// TTX2 双轨不变式被破坏：pos 与 neg 同一字同一位同时置位。
    #[error("TTX2 第 {layer} 层第 {word} 字 pos/neg 轨重叠")]
    TtxRailOverlap {
        /// 层下标。
        layer: usize,
        /// 字下标。
        word: usize,
    },
}

/// RGB565 打包：R 高 5 位、G 中 6 位、B 低 5 位。
fn pack_rgb565(r: u8, g: u8, b: u8) -> u16 {
    (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3)
}

/// RGB565 展开（左对齐补位，`(x << 3) | (x >> 2)` 风格，让 0/31 满量程）。
fn unpack_rgb565(value: u16) -> (u8, u8, u8) {
    let r5 = (value >> 11) as u8 & 0x1f;
    let g6 = (value >> 5) as u8 & 0x3f;
    let b5 = value as u8 & 0x1f;
    (
        (r5 << 3) | (r5 >> 2),
        (g6 << 2) | (g6 >> 4),
        (b5 << 3) | (b5 >> 2),
    )
}

/// 5 trit/字节打包（低位在前）：`Σ digit_i · 3^i`，3^5 = 243 ≤ 256。
fn trit_digit(value: Ternary) -> u32 {
    match value {
        Ternary::False => 0,
        Ternary::Unknown => 1,
        Ternary::True => 2,
    }
}

fn pack_trits(values: &[Ternary]) -> [u8; TRIT_PAYLOAD_BYTES] {
    let mut out = [0u8; TRIT_PAYLOAD_BYTES];
    for (index, value) in values.iter().enumerate() {
        let byte = index / 5;
        let slot = index % 5;
        out[byte] += (trit_digit(*value) * 3u32.pow(slot as u32)) as u8;
    }
    out
}

fn unpack_trits(bytes: &[u8; TRIT_PAYLOAD_BYTES]) -> [Ternary; TRITS_PER_BLOCK] {
    let mut out = [Ternary::False; TRITS_PER_BLOCK];
    for (index, slot) in out.iter_mut().enumerate() {
        let digit = (bytes[index / 5] as u32 / 3u32.pow((index % 5) as u32)) % 3;
        *slot = match digit {
            0 => Ternary::False,
            1 => Ternary::Unknown,
            _ => Ternary::True,
        };
    }
    out
}

/// 一个 TT3 块：4 字节 RGB565 端点 + 7 字节 trit 载荷。
pub type TritBlock = [u8; TRIT_BLOCK_BYTES];

/// TT3 编码后的纹理：`blocks[y / 8][x / 4]` 行主序。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TritTexture {
    /// 宽（4 的倍数）。
    pub width: usize,
    /// 高（8 的倍数）。
    pub height: usize,
    /// 块数据。
    pub blocks: Vec<TritBlock>,
}

/// 把 RGBA 像素编码成 TT3。
///
/// 宽须是 4 的倍数、高须是 8 的倍数；输入长度必须恰为 `width × height × 4`。
pub fn encode_rgba(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<TritTexture, TextureError> {
    validate(width, height, pixels.len())?;
    let block_cols = width / BLOCK_WIDTH;
    let block_rows = height / BLOCK_HEIGHT;
    let mut blocks = Vec::with_capacity(block_cols * block_rows);
    for block_row in 0..block_rows {
        for block_col in 0..block_cols {
            let origin = (block_col * BLOCK_WIDTH, block_row * BLOCK_HEIGHT);
            let mut texels = [[0u8; 4]; BLOCK_PIXELS];
            for (index, texel) in texels.iter_mut().enumerate() {
                let x = origin.0 + index % BLOCK_WIDTH;
                let y = origin.1 + index / BLOCK_WIDTH;
                let offset = (y * width + x) * 4;
                *texel = [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ];
            }
            blocks.push(encode_block(&texels));
        }
    }
    Ok(TritTexture {
        width,
        height,
        blocks,
    })
}

/// 单块编码：min/max 端点 + 最小二乘精修，每纹素最近调色板色的 trit 索引。
pub fn encode_block(texels: &[[u8; 4]; BLOCK_PIXELS]) -> TritBlock {
    let (e0, e1) = block_endpoints(texels);
    encode_block_with_endpoints(texels, e0, e1)
}

/// 同 [`encode_block`]，但端点由调用方给（已做 RGB565 量化）——精修/标定入口。
pub fn encode_block_with_endpoints(
    texels: &[[u8; 4]; BLOCK_PIXELS],
    e0: u16,
    e1: u16,
) -> TritBlock {
    let (e0, e1) = refine_endpoints(texels, e0, e1);
    let palette = palette_from_endpoints(e0, e1);
    let mut indices = [Ternary::False; TRITS_PER_BLOCK];
    for (index, pixel) in texels.iter().enumerate() {
        indices[index] = match nearest_index(&palette, pixel[0], pixel[1], pixel[2]) {
            0 => Ternary::False,
            1 => Ternary::Unknown,
            _ => Ternary::True,
        };
    }
    let mut block = [0u8; TRIT_BLOCK_BYTES];
    block[0..2].copy_from_slice(&e0.to_be_bytes());
    block[2..4].copy_from_slice(&e1.to_be_bytes());
    block[4..].copy_from_slice(&pack_trits(&indices));
    block
}

/// 解码 TT3 回 RGBA（alpha 恒 255）。
pub fn decode_rgba(texture: &TritTexture) -> Result<Vec<u8>, TextureError> {
    let mut out = vec![0u8; texture.width * texture.height * 4];
    decode_rgba_into(texture, &mut out)?;
    Ok(out)
}

/// 解码到**调用方缓冲**：热路径复用同一份输出，免掉整图分配（显存/内存账）。
///
/// `out` 必须恰为 `width × height × 4` 字节；解码器不做隐式清零，逐字节覆写。
pub fn decode_rgba_into(texture: &TritTexture, out: &mut [u8]) -> Result<(), TextureError> {
    let block_cols = texture.width / BLOCK_WIDTH;
    let block_rows = texture.height / BLOCK_HEIGHT;
    let expected = block_cols * block_rows;
    if texture.blocks.len() != expected {
        return Err(TextureError::BlockCount {
            actual: texture.blocks.len(),
            expected,
        });
    }
    if out.len() != texture.width * texture.height * 4 {
        return Err(TextureError::SizeMismatch {
            actual: out.len(),
            expected: texture.width * texture.height * 4,
        });
    }
    for (block_index, block) in texture.blocks.iter().enumerate() {
        let block_col = block_index % block_cols;
        let block_row = block_index / block_cols;
        let e0 = u16::from_be_bytes([block[0], block[1]]);
        let e1 = u16::from_be_bytes([block[2], block[3]]);
        let mut payload = [0u8; TRIT_PAYLOAD_BYTES];
        payload.copy_from_slice(&block[4..]);
        let indices = unpack_trits(&payload);
        let palette = palette_from_endpoints(e0, e1);
        for index in 0..BLOCK_PIXELS {
            let x = block_col * BLOCK_WIDTH + index % BLOCK_WIDTH;
            let y = block_row * BLOCK_HEIGHT + index / BLOCK_WIDTH;
            let offset = (y * texture.width + x) * 4;
            let (r, g, b) = palette[match indices[index] {
                Ternary::False => 0,
                Ternary::Unknown => 1,
                Ternary::True => 2,
            }];
            out[offset] = r;
            out[offset + 1] = g;
            out[offset + 2] = b;
            out[offset + 3] = 255;
        }
    }
    Ok(())
}

/// 二进制基线纹理：同一块结构、同一 3 级调色板，索引 2 bit/纹素。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryTexture {
    /// 宽。
    pub width: usize,
    /// 高。
    pub height: usize,
    /// 块数据（4 字节端点 + 8 字节位载荷）。
    pub blocks: Vec<[u8; BINARY_BLOCK_BYTES]>,
}

/// 把 RGBA 像素编码成二进制基线（与 TT3 同端点、同调色板，仅索引编码不同）。
pub fn encode_rgba_baseline(
    pixels: &[u8],
    width: usize,
    height: usize,
) -> Result<BinaryTexture, TextureError> {
    validate(width, height, pixels.len())?;
    let block_cols = width / BLOCK_WIDTH;
    let block_rows = height / BLOCK_HEIGHT;
    let mut blocks = Vec::with_capacity(block_cols * block_rows);
    for block_row in 0..block_rows {
        for block_col in 0..block_cols {
            let mut texels = [[0u8; 4]; BLOCK_PIXELS];
            for (index, texel) in texels.iter_mut().enumerate() {
                let x = block_col * BLOCK_WIDTH + index % BLOCK_WIDTH;
                let y = block_row * BLOCK_HEIGHT + index / BLOCK_WIDTH;
                let offset = (y * width + x) * 4;
                *texel = [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ];
            }
            blocks.push(encode_block_baseline(&texels));
        }
    }
    Ok(BinaryTexture {
        width,
        height,
        blocks,
    })
}

/// 单块基线编码：索引 2 bit/纹素，LSB 先行。
///
/// 端点走**未精修的 min/max**（与 TT3 精修前的行为一致），这样基线与 TT3 的
/// 对比仍然只差「索引进制」这一个变量；精修带来的增益在 `encode_block` 侧。
pub fn encode_block_baseline(texels: &[[u8; 4]; BLOCK_PIXELS]) -> [u8; BINARY_BLOCK_BYTES] {
    let (e0, e1) = block_endpoints(texels);
    let palette = palette_from_endpoints(e0, e1);
    let mut payload = [0u8; BINARY_PAYLOAD_BYTES];
    for (index, pixel) in texels.iter().enumerate() {
        let palette_index = nearest_index(&palette, pixel[0], pixel[1], pixel[2]) as u64;
        let bit = index * 2;
        payload[bit / 8] |= ((palette_index << (bit % 8)) & 0xff) as u8;
        if bit % 8 > 6 {
            payload[bit / 8 + 1] |= (palette_index >> (8 - bit % 8)) as u8;
        }
    }
    let mut block = [0u8; BINARY_BLOCK_BYTES];
    block[0..2].copy_from_slice(&e0.to_be_bytes());
    block[2..4].copy_from_slice(&e1.to_be_bytes());
    block[4..].copy_from_slice(&payload);
    block
}

/// 解码二进制基线回 RGBA。
pub fn decode_rgba_baseline(texture: &BinaryTexture) -> Result<Vec<u8>, TextureError> {
    let block_cols = texture.width / BLOCK_WIDTH;
    let block_rows = texture.height / BLOCK_HEIGHT;
    let expected = block_cols * block_rows;
    if texture.blocks.len() != expected {
        return Err(TextureError::BlockCount {
            actual: texture.blocks.len(),
            expected,
        });
    }
    let mut out = vec![0u8; texture.width * texture.height * 4];
    for (block_index, block) in texture.blocks.iter().enumerate() {
        let block_col = block_index % block_cols;
        let block_row = block_index / block_cols;
        let e0 = u16::from_be_bytes([block[0], block[1]]);
        let e1 = u16::from_be_bytes([block[2], block[3]]);
        let palette = palette_from_endpoints(e0, e1);
        for index in 0..BLOCK_PIXELS {
            let bit = index * 2;
            let mut palette_index = (block[4 + bit / 8] as u64 >> (bit % 8)) & 0x3;
            if bit % 8 > 6 {
                palette_index |= (block[4 + bit / 8 + 1] as u64) << (8 - bit % 8) & 0x3;
            }
            let x = block_col * BLOCK_WIDTH + index % BLOCK_WIDTH;
            let y = block_row * BLOCK_HEIGHT + index / BLOCK_WIDTH;
            let offset = (y * texture.width + x) * 4;
            let (r, g, b) = palette[palette_index as usize];
            out[offset] = r;
            out[offset + 1] = g;
            out[offset + 2] = b;
            out[offset + 3] = 255;
        }
    }
    Ok(out)
}

/// 两块同尺寸 RGBA 缓冲的 PSNR（按 RGB 通道，忽略 alpha）。
/// 完全相同时返回 `None`（MSE = 0，PSNR 无穷）。
pub fn psnr_rgb(a: &[u8], b: &[u8]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut total = 0u64;
    let mut count = 0u64;
    for (left, right) in a.iter().zip(b.iter()) {
        let diff = i64::from(*left) - i64::from(*right);
        total += (diff * diff) as u64;
        count += 1;
    }
    // 每像素取 RGB 三通道（跳过每 4 字节里的 alpha）。
    let pixels = count / 4;
    if pixels == 0 {
        return None;
    }
    let mse = (total as f64) / (pixels as f64 * 3.0);
    if mse == 0.0 {
        return None;
    }
    Some(10.0 * (255.0 * 255.0 / mse).log10())
}

fn validate(width: usize, height: usize, len: usize) -> Result<(), TextureError> {
    if width == 0
        || height == 0
        || !width.is_multiple_of(BLOCK_WIDTH)
        || !height.is_multiple_of(BLOCK_HEIGHT)
    {
        return Err(TextureError::Shape { width, height });
    }
    let pixels = width
        .checked_mul(height)
        .ok_or(TextureError::TooLarge { pixels: usize::MAX })?;
    if pixels > MAX_TEXTURE_PIXELS {
        return Err(TextureError::TooLarge { pixels });
    }
    let expected = pixels * 4;
    if len != expected {
        return Err(TextureError::SizeMismatch {
            actual: len,
            expected,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(color: [u8; 3]) -> [[u8; 4]; BLOCK_PIXELS] {
        [[color[0], color[1], color[2], 255]; BLOCK_PIXELS]
    }

    #[test]
    fn pack_roundtrips_all_five_trit_values() {
        // 3^5 = 243 ≤ 256：全部 243 种 5-trit 组合都必须逐位还原。
        let mut digits = [Ternary::False; 5];
        for value in 0..243u32 {
            let mut rest = value;
            for digit in digits.iter_mut() {
                *digit = match rest % 3 {
                    0 => Ternary::False,
                    1 => Ternary::Unknown,
                    _ => Ternary::True,
                };
                rest /= 3;
            }
            let packed = pack_trits(&digits);
            assert_eq!(packed[0] as u32, value, "value {value}");
            let unpacked = unpack_trits(&packed);
            assert_eq!(&unpacked[..5], &digits, "value {value}");
        }
    }

    #[test]
    fn block_roundtrips_endpoint_colors_exactly() {
        // RGB565 可精确表示的颜色（R/B 8 的倍数、G 4 的倍数）当端点：
        // 端点色必须零误差往返；中间色是端点平均，只要求落到 mid 槽位。
        // 565 可精确表示的色不止「8 的倍数」（高段的展开带低位回填），所以直接用
        // crate 自己的 pack→unpack 构造可表示色，端点必须零误差往返。
        // 端点是**逐通道** min/max（与 BC1 一致），所以两色必须全通道单调，
        // min/max 才恰好是这两个色本身。
        for raw in [
            ([40u8, 8, 200], [236u8, 251, 250]),
            ([0, 0, 0], [255, 255, 255]),
        ] {
            let expand = |c: [u8; 3]| {
                let (r, g, b) = unpack_rgb565(pack_rgb565(c[0], c[1], c[2]));
                [r, g, b]
            };
            let low = expand(raw.0);
            let high = expand(raw.1);
            let mut texels = solid(low);
            texels[1] = [high[0], high[1], high[2], 255];
            // mid 槽位：端点平均色必须精确落到 index 1（mid 舍入写错会跑偏）。
            let avg = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
            texels[2] = [
                avg(low[0], high[0]),
                avg(low[1], high[1]),
                avg(low[2], high[2]),
                255,
            ];
            let block = encode_block(&texels);
            let e0 = u16::from_be_bytes([block[0], block[1]]);
            let e1 = u16::from_be_bytes([block[2], block[3]]);
            let palette = palette_from_endpoints(e0, e1);
            let mids = [texels[2][0], texels[2][1], texels[2][2]];
            for (texel, color) in [(0, low), (1, high), (2, mids)] {
                let pixel = &texels[texel];
                let decoded = palette[nearest_index(&palette, pixel[0], pixel[1], pixel[2])];
                assert_eq!(
                    [decoded.0, decoded.1, decoded.2],
                    color[..3],
                    "case {raw:?} texel {texel}"
                );
            }
        }
    }

    #[test]
    fn texture_roundtrip_and_block_budgets_are_exact() {
        // 8×8 块：TT3 每块 17 字节（4 端点 + 13 载荷）、基线每块 20 字节（4 + 16）
        // ⇒ 2.125 vs 2.5 bpp。载荷 13 字节 = ceil(64/5)，打包税 1.6%。
        let width = 8;
        let height = 8;
        let mut pixels = vec![0u8; width * height * 4];
        for (index, byte) in pixels.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        let trit = encode_rgba(&pixels, width, height).unwrap();
        assert_eq!(trit.blocks.len(), 1);
        assert_eq!(trit.blocks[0].len(), TRIT_BLOCK_BYTES);
        assert_eq!(TRIT_BLOCK_BYTES, 17);
        let decoded = decode_rgba(&trit).unwrap();
        assert_eq!(decoded.len(), pixels.len());

        let baseline = encode_rgba_baseline(&pixels, width, height).unwrap();
        assert_eq!(baseline.blocks[0].len(), BINARY_BLOCK_BYTES);
        assert_eq!(BINARY_BLOCK_BYTES, 20);
        let decoded_baseline = decode_rgba_baseline(&baseline).unwrap();
        assert_eq!(decoded_baseline.len(), pixels.len());
        // 两种编码的块数相同，每块 17 vs 20 字节（省 15% 码率）。
        assert_eq!(trit.blocks.len(), baseline.blocks.len());
        assert_eq!(
            20 * trit.blocks.len() * TRIT_BLOCK_BYTES,
            17 * baseline.blocks.len() * BINARY_BLOCK_BYTES
        );
        // 打包税：64 trit 占 13×5 = 65 槽 ⇒ 浪费 1/65 ≈ 1.6%（不是 v1 的 20% 载荷差）。
        assert_eq!(TRIT_PAYLOAD_BYTES * 5 - TRITS_PER_BLOCK, 1);
    }

    #[test]
    fn block_shape_is_mip_friendly() {
        // 回归门：块形状必须都是 2 的幂，否则 mip 链每级减半后很快不合法，
        // 正常 2 的幂纹理就得补边（v1 的 4×10 高须 %10 ⇒ 恒定 25% 填充）。
        //
        // 这正是「零槽位浪费」的陷阱：40 恰被 5 整除看着很美，但 10 不是 2 的幂，
        // 于是每级减半都失去合法性。要把这条钉死，防止有人为了省 1.6% 打包税
        // 把块改回 4×10。
        assert!(
            BLOCK_WIDTH.is_power_of_two(),
            "块宽 {BLOCK_WIDTH} 必须是 2 的幂，否则 mip 链断裂"
        );
        assert!(
            BLOCK_HEIGHT.is_power_of_two(),
            "块高 {BLOCK_HEIGHT} 必须是 2 的幂，否则 mip 链断裂"
        );
        // 方形块：宽高相等时任意 2 的幂纹理都能一路 mip 到 1×1。
        assert_eq!(BLOCK_WIDTH, BLOCK_HEIGHT, "非方块会让 mip 衰减不对称");
        // 逐级减半：从 4096 一路到**块尺寸**（8），每一级都必须是块的整数倍。
        // 低于一个块的层级（4/2/1）在语义上就是空的——mip 链到底了，不是非法。
        let mut side = 4096usize;
        while side >= BLOCK_WIDTH {
            assert!(
                side.is_multiple_of(BLOCK_WIDTH),
                "{side} 不是块宽 {BLOCK_WIDTH} 的倍数，该级 mip 非法"
            );
            side /= 2;
        }
        // 停下时必须恰好是「减半后不再够一个块」，而不是提前卡在某个中间层。
        assert!(
            side < BLOCK_WIDTH,
            "mip 链应在块尺寸处到底，实际停在 {side}"
        );
    }

    #[test]
    fn baseline_roundtrips_endpoint_colors_exactly() {
        // 二进制基线与 TT3 同端点同调色板，仅索引编码不同——位打包写错会丢色。
        for raw in [
            ([40u8, 8, 200], [236u8, 251, 250]),
            ([0, 0, 0], [255, 255, 255]),
        ] {
            let expand = |c: [u8; 3]| {
                let (r, g, b) = unpack_rgb565(pack_rgb565(c[0], c[1], c[2]));
                [r, g, b]
            };
            let low = expand(raw.0);
            let high = expand(raw.1);
            // low 填满整块，texel 1 放 high：端点才恰好是这两个色。
            let mut pixels = vec![0u8; BLOCK_WIDTH * BLOCK_HEIGHT * 4];
            for texel in 0..BLOCK_PIXELS {
                pixels[texel * 4] = low[0];
                pixels[texel * 4 + 1] = low[1];
                pixels[texel * 4 + 2] = low[2];
                pixels[texel * 4 + 3] = 255;
            }
            pixels[4] = high[0];
            pixels[5] = high[1];
            pixels[6] = high[2];
            pixels[7] = 255;
            let texture = encode_rgba_baseline(&pixels, BLOCK_WIDTH, BLOCK_HEIGHT).unwrap();
            assert_eq!(texture.blocks.len(), 1);
            let decoded = decode_rgba_baseline(&texture).unwrap();
            for (texel, color) in [(0, low), (1, high)] {
                assert_eq!(
                    [
                        decoded[texel * 4],
                        decoded[texel * 4 + 1],
                        decoded[texel * 4 + 2]
                    ],
                    color,
                    "case {raw:?} texel {texel}"
                );
            }
        }
    }

    #[test]
    fn rejects_bad_shape_size_and_oversize() {
        assert!(matches!(
            encode_rgba(&[], 3, 8),
            Err(TextureError::Shape {
                width: 3,
                height: 8
            })
        ));
        assert!(matches!(
            encode_rgba(&[0; 64], 4, 4),
            Err(TextureError::Shape {
                width: 4,
                height: 4
            })
        ));
        assert!(matches!(
            encode_rgba(&[0; 16], 8, 8),
            Err(TextureError::SizeMismatch {
                actual: 16,
                expected: 256
            })
        ));
        let big = (MAX_TEXTURE_PIXELS + 1).next_multiple_of(BLOCK_PIXELS);
        assert!(matches!(
            encode_rgba(&[], BLOCK_WIDTH, big / BLOCK_WIDTH),
            Err(TextureError::TooLarge { .. })
        ));
        // 解码侧块数不符。
        let texture = TritTexture {
            width: BLOCK_WIDTH,
            height: BLOCK_HEIGHT,
            blocks: vec![[0u8; TRIT_BLOCK_BYTES]; 3],
        };
        assert!(matches!(
            decode_rgba(&texture),
            Err(TextureError::BlockCount {
                actual: 3,
                expected: 1
            })
        ));
    }

    #[test]
    fn decode_into_matches_allocating_and_rejects_short_buffers() {
        let width = BLOCK_WIDTH;
        let height = BLOCK_HEIGHT;
        let pixels: Vec<u8> = (0..width * height * 4).map(|i| (i % 251) as u8).collect();
        let texture = encode_rgba(&pixels, width, height).unwrap();
        let allocating = decode_rgba(&texture).unwrap();
        let mut reused = vec![7u8; width * height * 4];
        decode_rgba_into(&texture, &mut reused).unwrap();
        assert_eq!(reused, allocating, "into 路径必须与分配路径逐字节一致");
        // alpha 恒 255：两条路径共用实现，必须显式钉住通道值。
        for texel in 0..width * height {
            assert_eq!(reused[texel * 4 + 3], 255, "texel {texel}");
        }
        // 输出缓冲长度不符：显式拒绝而不是 panic。
        let mut short = vec![0u8; width * height * 4 - 4];
        assert!(matches!(
            decode_rgba_into(&texture, &mut short),
            Err(TextureError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn psnr_reports_none_for_identical_and_ranks_clean_over_noise() {
        assert_eq!(psnr_rgb(&[], &[]), None);
        let a = vec![128u8; 64];
        assert_eq!(psnr_rgb(&a, &a), None);
        let clean = vec![100u8; 4];
        let noisy = vec![110u8; 4];
        let worse = vec![140u8; 4];
        let psnr_clean = psnr_rgb(&clean, &noisy).unwrap();
        let psnr_worse = psnr_rgb(&clean, &worse).unwrap();
        assert!(psnr_clean > psnr_worse, "{psnr_clean} vs {psnr_worse}");
    }
}
