// TT3 块纹理解码（与 trit-texture 的 CPU 解码逐字节对拍）。
//
// 块布局（每块 12 字节 = 3 个 u32，GPU 按 LE 读）：
//   word0: e0hi e0lo e1hi e1lo（e0/e1 为 RGB565 的 BE u16）
//   word1-2: 40 纹素 × 1 trit，5 trit/字节（低位在前）。
// 一线程一纹素；调色板 {e0, mid, e1} 的展开与 (a+b)/2 中点必须与 CPU 完全一致。

struct Params {
    texels: u32,
    width: u32,
    block_cols: u32,
};

@group(0) @binding(0) var<storage, read> blocks: array<u32>;
@group(0) @binding(1) var<storage, read_write> out: array<u32>;
@group(0) @binding(2) var<uniform> params: Params;

fn expand5(v: u32) -> u32 {
    return (v << 3u) | (v >> 2u);
}

fn expand6(v: u32) -> u32 {
    return (v << 2u) | (v >> 4u);
}

// 载荷第 k 字节（块内整体字节 4+k，落在 word1/word2；k<4 在 word1、k>=4 在 word2）。
fn payload_byte(base: u32, k: u32) -> u32 {
    let word = blocks[base + 1u + k / 4u];
    return (word >> ((k % 4u) * 8u)) & 0xffu;
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let texel = gid.x;
    if (texel >= params.texels) {
        return;
    }
    // 块是 4×10 的矩形、按块网格行主序排布——不是平铺扫描序。
    // 输出像素 (x, y) → 块 (x/4, y/10) → 块内序 index = (y%10)*4 + x%4
    //（与编码器 index%4 = x 偏移、index/4 = y 偏移一致）。
    let x = texel % params.width;
    let y = texel / params.width;
    let block_col = x / 4u;
    let block_row = y / 10u;
    let block_id = block_row * params.block_cols + block_col;
    let index = (y % 10u) * 4u + (x % 4u);
    let base = block_id * 3u;
    // 块头按字节拆（LE 语义），再按 BE 组回 16 位端点——位序错一点全图花。
    let b0 = blocks[base] & 0xffu;
    let b1 = (blocks[base] >> 8u) & 0xffu;
    let b2 = (blocks[base] >> 16u) & 0xffu;
    let b3 = (blocks[base] >> 24u) & 0xffu;
    let e0 = (b0 << 8u) | b1;
    let e1 = (b2 << 8u) | b3;
    let r0 = expand5((e0 >> 11u) & 0x1fu);
    let g0 = expand6((e0 >> 5u) & 0x3fu);
    let b0c = expand5(e0 & 0x1fu);
    let r1 = expand5((e1 >> 11u) & 0x1fu);
    let g1 = expand6((e1 >> 5u) & 0x3fu);
    let b1c = expand5(e1 & 0x1fu);
    let k = index / 5u;
    // 5 trit/字节是**基 3** 打包（3^5=243≤256），不是 3-bit 位段：
    // 第 i 个 trit = byte / 3^i % 3。
    var pow3 = 1u;
    for (var i = 0u; i < index % 5u; i = i + 1u) {
        pow3 = pow3 * 3u;
    }
    let trit = (payload_byte(base, k) / pow3) % 3u;
    var r = 0u;
    var g = 0u;
    var b = 0u;
    if (trit == 0u) {
        r = r0;
        g = g0;
        b = b0c;
    } else if (trit == 1u) {
        r = (r0 + r1) / 2u;
        g = (g0 + g1) / 2u;
        b = (b0c + b1c) / 2u;
    } else {
        r = r1;
        g = g1;
        b = b1c;
    }
    out[texel] = r | (g << 8u) | (b << 16u) | 0xff000000u;
}
