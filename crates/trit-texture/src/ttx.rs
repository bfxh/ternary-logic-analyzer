//! **TTX**（TritText eXchange）——本仓自有的三进制纹理容器格式。
//!
//! 布局（全部小端）：
//!
//! ```text
//! 偏移  大小  内容
//! 0     4    magic "TTX1"
//! 4     4    width  (u32，BLOCK_WIDTH 的倍数)
//! 8     4    height (u32，BLOCK_HEIGHT 的倍数)
//! 12    4    blocks (u32，块数 = (width/4)·(height/10))
//! 16    N    块载荷：每块 TRIT_BLOCK_BYTES(12) 字节
//! 16+N  1    GF(3) CRC-3：对全部块的 40·blocks 个**索引 trit** 计算
//! ```
//!
//! 与二进制容器（PNG 等）的差异：校验和不是比特 CRC，而是 **GF(3) 上的
//! CRC-3**——对三进制内容按三进制运算，`x³+2x+1` 本原，单符号错全检。
//! 头部之后的每个字节都属于格式；加载时逐段校验，不一致即结构化报错。

use trit_core::value::Ternary;

use crate::{
    BLOCK_HEIGHT, BLOCK_WIDTH, TRIT_BLOCK_BYTES, TRITS_PER_BLOCK, TextureError, TritTexture,
};

/// 魔数：`TTX1`（版本随首字节递进）。
pub const MAGIC: [u8; 4] = *b"TTX1";
/// 头部字节数（magic + width + height + blocks）。
pub const HEADER_BYTES: usize = 16;
/// 校验尾字节数（3 trit 打进 1 字节：3³ = 27 ≤ 256）。
pub const CHECKSUM_BYTES: usize = 1;

/// GF(3) 生成多项式 `x³ + 2x + 1`（本原），系数高次在前——与 trit-codec 的
/// checksum 同多项式；此处就地实现（实验层互不依赖）。
const GEN: [i32; 4] = [1, 0, 2, 1];

pub(crate) fn gf3_remainder(trits: &[Ternary]) -> [u8; CHECKSUM_BYTES] {
    let digit = |t: Ternary| match t {
        Ternary::False => 0i32,
        Ternary::Unknown => 1,
        Ternary::True => 2,
    };
    let mut buf: Vec<i32> = trits.iter().map(|t| digit(*t)).collect();
    buf.extend_from_slice(&[0; 3]);
    for i in 0..=buf.len() - GEN.len() {
        let coeff = buf[i];
        if coeff != 0 {
            for (j, &g) in GEN.iter().enumerate() {
                buf[i + j] = (buf[i + j] - coeff * g).rem_euclid(3);
            }
        }
    }
    let r = &buf[buf.len() - 3..];
    [(r[0] * 9 + r[1] * 3 + r[2]) as u8]
}

pub(crate) fn block_trits(block: &[u8; TRIT_BLOCK_BYTES]) -> [Ternary; TRITS_PER_BLOCK] {
    let mut out = [Ternary::False; TRITS_PER_BLOCK];
    for (index, slot) in out.iter_mut().enumerate() {
        // 只取每块的索引 trit 载荷（后 8 字节 = 40 trit，基 3 打包）。
        let byte = block[4 + index / 5];
        let digit = (byte as u32 / 3u32.pow((index % 5) as u32)) % 3;
        *slot = match digit {
            0 => Ternary::False,
            1 => Ternary::Unknown,
            _ => Ternary::True,
        };
    }
    out
}

fn checksum(texture: &TritTexture) -> u8 {
    let mut trits = Vec::with_capacity(texture.blocks.len() * TRITS_PER_BLOCK);
    for block in &texture.blocks {
        trits.extend_from_slice(&block_trits(block));
    }
    gf3_remainder(&trits)[0]
}

/// 把纹理序列化成 TTX 容器。
pub fn to_ttx(texture: &TritTexture) -> Vec<u8> {
    let mut out =
        Vec::with_capacity(HEADER_BYTES + texture.blocks.len() * TRIT_BLOCK_BYTES + CHECKSUM_BYTES);
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&(texture.width as u32).to_le_bytes());
    out.extend_from_slice(&(texture.height as u32).to_le_bytes());
    out.extend_from_slice(&(texture.blocks.len() as u32).to_le_bytes());
    for block in &texture.blocks {
        out.extend_from_slice(block);
    }
    out.push(checksum(texture));
    out
}

/// 从 TTX 容器加载纹理：魔数、尺寸对齐、块数与载荷长度、GF(3) 校验和
/// 逐段验证——任何不一致都是结构化错误，绝不猜。
pub fn from_ttx(bytes: &[u8]) -> Result<TritTexture, TextureError> {
    if bytes.len() < HEADER_BYTES + CHECKSUM_BYTES {
        return Err(TextureError::TtxTruncated {
            actual: bytes.len(),
        });
    }
    if bytes[..4] != MAGIC {
        return Err(TextureError::TtxMagic(bytes[..4].try_into().unwrap()));
    }
    let width = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let height = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let blocks = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    if width == 0 || !width.is_multiple_of(BLOCK_WIDTH) {
        return Err(TextureError::Shape { width, height });
    }
    if height == 0 || !height.is_multiple_of(BLOCK_HEIGHT) {
        return Err(TextureError::Shape { width, height });
    }
    let expected_blocks = (width / BLOCK_WIDTH) * (height / BLOCK_HEIGHT);
    if blocks != expected_blocks {
        return Err(TextureError::TtxBlockCount {
            declared: blocks as u32,
            expected: expected_blocks,
        });
    }
    let payload_len = blocks * TRIT_BLOCK_BYTES;
    if bytes.len() != HEADER_BYTES + payload_len + CHECKSUM_BYTES {
        return Err(TextureError::TtxTruncated {
            actual: bytes.len(),
        });
    }
    let mut block_list = Vec::with_capacity(blocks);
    for index in 0..blocks {
        let start = HEADER_BYTES + index * TRIT_BLOCK_BYTES;
        let mut block = [0u8; TRIT_BLOCK_BYTES];
        block.copy_from_slice(&bytes[start..start + TRIT_BLOCK_BYTES]);
        block_list.push(block);
    }
    let texture = TritTexture {
        width,
        height,
        blocks: block_list,
    };
    let stored = bytes[HEADER_BYTES + payload_len];
    if stored != checksum(&texture) {
        return Err(TextureError::TtxChecksum {
            stored,
            expected: checksum(&texture),
        });
    }
    Ok(texture)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_rgba, encode_rgba};

    fn sample(width: usize, height: usize) -> Vec<u8> {
        (0..width * height * 4).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn round_trips_through_container() {
        let pixels = sample(8, 10);
        let texture = encode_rgba(&pixels, 8, 10).unwrap();
        let container = to_ttx(&texture);
        // 布局：16 头 + 2 块×12 + 1 校验（8 宽 → 2 列块）。
        assert_eq!(container.len(), HEADER_BYTES + 24 + CHECKSUM_BYTES);
        assert_eq!(&container[..4], &MAGIC);
        let back = from_ttx(&container).unwrap();
        assert_eq!(back, texture);
        assert_eq!(decode_rgba(&back).unwrap(), decode_rgba(&texture).unwrap());
    }

    #[test]
    fn rejects_bad_magic_truncation_and_block_count() {
        let texture = encode_rgba(&sample(4, 10), 4, 10).unwrap();
        let mut container = to_ttx(&texture);
        // 魔数错。
        container[0] = b'X';
        assert!(matches!(
            from_ttx(&container),
            Err(TextureError::TtxMagic(_))
        ));
        // 截断。
        let mut truncated = to_ttx(&texture);
        truncated.pop();
        assert!(matches!(
            from_ttx(&truncated),
            Err(TextureError::TtxTruncated { .. })
        ));
        // 块数声明与尺寸不符。
        let mut bad_count = to_ttx(&texture);
        bad_count[12] = 9;
        assert!(matches!(
            from_ttx(&bad_count),
            Err(TextureError::TtxBlockCount {
                declared: 9,
                expected: 1
            })
        ));
    }

    #[test]
    fn payload_corruption_is_caught_by_gf3_checksum() {
        // 翻转载荷里的一个 trit（字节 16 = 首块索引 trit 打包首字节）：
        // GF(3) CRC-3 对单符号错全检。
        let texture = encode_rgba(&sample(4, 10), 4, 10).unwrap();
        let mut container = to_ttx(&texture);
        container[HEADER_BYTES + 4] ^= 0x1;
        assert!(matches!(
            from_ttx(&container),
            Err(TextureError::TtxChecksum { .. })
        ));
    }

    #[test]
    fn multi_block_checksum_covers_every_block() {
        // 8×20 = 4 块：把最后一块载荷打翻，校验和必须仍然抓到——
        // 防止"只校验了第一块"的假实现。
        let texture = encode_rgba(&sample(8, 20), 8, 20).unwrap();
        assert_eq!(texture.blocks.len(), 4);
        let mut container = to_ttx(&texture);
        let last_block_payload = HEADER_BYTES + 3 * TRIT_BLOCK_BYTES + 4;
        container[last_block_payload] ^= 0x1;
        assert!(matches!(
            from_ttx(&container),
            Err(TextureError::TtxChecksum { .. })
        ));
    }
}
