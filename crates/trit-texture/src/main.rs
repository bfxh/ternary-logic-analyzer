//! `trit-texture` 同机对比表：合成纹理上 TT3（trit 索引）vs 二进制基线
//! （同端点、同 3 级调色板、2 bit 索引），码率与 PSNR 同尺输出。
//!
//! 可复算：`cargo run -p trit-texture`。对比口径见 docs/TERNARY-BOUNDARIES.md §6/§7：
//! 两边同端点、同调色板、同最近色搜索，唯一变量是索引的进制。

use std::time::Instant;

use trit_texture::{
    BINARY_BLOCK_BYTES, BLOCK_PIXELS, TRIT_BLOCK_BYTES, decode_rgba, decode_rgba_baseline,
    encode_rgba, encode_rgba_baseline, psnr_rgb,
};

/// 128×64 的合成纹理：水平渐变 + 竖条纹 + 少量椒盐噪点，RGBA。
fn synthetic_texture(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = vec![0u8; width * height * 4];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for y in 0..height {
        for x in 0..width {
            let offset = (y * width + x) * 4;
            let stripe = if (x / 4) % 2 == 0 { 40 } else { 0 };
            let mut r = (x * 255 / width) as u8;
            let mut g = (y * 255 / height) as u8;
            let mut b = 100u8.saturating_add(stripe);
            // ~2% 纹素打椒盐，暴露最近色索引的行为。
            if next() % 100 < 2 {
                let salt = (next() % 256) as u8;
                r = salt;
                g = salt;
                b = salt;
            }
            pixels[offset] = r;
            pixels[offset + 1] = g;
            pixels[offset + 2] = b;
            pixels[offset + 3] = 255;
        }
    }
    pixels
}

fn main() {
    let width = 128;
    let height = 64;
    let pixels = synthetic_texture(width, height);
    let texels = width * height;
    let raw_bytes = texels * 4;

    let start = Instant::now();
    let trit = encode_rgba(&pixels, width, height).unwrap();
    let trit_encode_us = start.elapsed().as_micros();
    let trit_decoded = decode_rgba(&trit).unwrap();

    let start = Instant::now();
    let binary = encode_rgba_baseline(&pixels, width, height).unwrap();
    let binary_encode_us = start.elapsed().as_micros();
    let binary_decoded = decode_rgba_baseline(&binary).unwrap();

    let trit_bytes = trit.blocks.len() * TRIT_BLOCK_BYTES;
    let binary_bytes = binary.blocks.len() * BINARY_BLOCK_BYTES;

    println!(
        "合成纹理 {width}×{height}（{texels} 纹素，块 = 4×8 = {BLOCK_PIXELS} 纹素，RGB565 端点对 + 3 级调色板）"
    );
    println!();
    println!(
        "{:<28} {:>8} {:>8} {:>8} {:>9} {:>9}",
        "format", "bytes", "bpp", "PSNR", "enc µs", "块字节"
    );
    println!(
        "{:<28} {:>8} {:>8} {:>8} {:>9} {:>8}",
        "raw RGBA", raw_bytes, "-", "-", "-", "-"
    );
    println!(
        "{:<28} {:>8} {:>8.2} {:>8.2} {:>9} {:>8}",
        "TT3 (1 trit/texel)",
        trit_bytes,
        trit_bytes as f64 * 8.0 / texels as f64,
        psnr_rgb(&pixels, &trit_decoded).unwrap_or(f64::INFINITY),
        trit_encode_us,
        TRIT_BLOCK_BYTES
    );
    println!(
        "{:<28} {:>8} {:>8.2} {:>8.2} {:>9} {:>8}",
        "binary 2-bit (3-level palette)",
        binary_bytes,
        binary_bytes as f64 * 8.0 / texels as f64,
        psnr_rgb(&pixels, &binary_decoded).unwrap_or(f64::INFINITY),
        binary_encode_us,
        BINARY_BLOCK_BYTES
    );
    println!();
    println!(
        "等调色板（3 级）码率差：每块 {} vs {} 字节（{}%）",
        TRIT_BLOCK_BYTES,
        BINARY_BLOCK_BYTES,
        100 * (BINARY_BLOCK_BYTES - TRIT_BLOCK_BYTES) / BINARY_BLOCK_BYTES
    );
    println!(
        "PSNR 差（同端点同调色板，索引进制不同）：{:.3} dB",
        psnr_rgb(&pixels, &binary_decoded).unwrap() - psnr_rgb(&pixels, &trit_decoded).unwrap()
    );
}
