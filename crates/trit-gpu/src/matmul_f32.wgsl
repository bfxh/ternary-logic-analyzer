// f32 基线矩阵乘：与 matmul.wgsl 完全相同的 naive 结构
// （一线程一输出、循环内积维度），唯一变量是数据表示（32 bit 浮点 vs 2 bit/trit）。
// fb 同样按转置行主序存放。

struct Params {
    m: u32,
    n: u32,
    k: u32,
};

@group(0) @binding(0) var<storage, read> fa: array<f32>;
@group(0) @binding(1) var<storage, read> fb: array<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if (index >= params.m * params.n) {
        return;
    }
    let row = index / params.n;
    let col = index % params.n;
    let a_base = row * params.k;
    let b_base = col * params.k;
    var sum = 0.0;
    for (var kk = 0u; kk < params.k; kk = kk + 1u) {
        sum = sum + fa[a_base + kk] * fb[b_base + kk];
    }
    out[index] = sum;
}

// 分块版（经典共享内存 tiling）：16×16 输出 tile，A/B 的 16×16 子块协作加载进
// workgroup storage，内积维按 tile 步进——全局读流量砍 TILE 倍。
const TILE: u32 = 16u;

var<workgroup> tile_a: array<f32, 256>;
var<workgroup> tile_b: array<f32, 256>;

@compute @workgroup_size(16, 16)
fn main_tiled(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let row = gid.y;
    let col = gid.x;
    let linear = lid.y * TILE + lid.x;
    var sum = 0.0;
    let k_tiles = (params.k + TILE - 1u) / TILE;
    for (var chunk = 0u; chunk < k_tiles; chunk = chunk + 1u) {
        let a_col = chunk * TILE + lid.x;
        tile_a[linear] = select(0.0, fa[row * params.k + a_col], row < params.m && a_col < params.k);
        // B^T 行主序：fb[col * k + kk]。
        let b_row = chunk * TILE + lid.y;
        tile_b[linear] = select(0.0, fb[col * params.k + b_row], col < params.n && b_row < params.k);
        workgroupBarrier();
        for (var kk = 0u; kk < TILE; kk = kk + 1u) {
            sum = sum + tile_a[lid.y * TILE + kk] * tile_b[kk * TILE + lid.x];
        }
        workgroupBarrier();
    }
    if (row < params.m && col < params.n) {
        out[row * params.n + col] = sum;
    }
}

// 寄存器阻塞版：workgroup 8×8，每线程 2×2 四个 FMA 累加器——
// tile_a/tile_b 每次读取喂 4 个乘加，算术强度再翻倍。tile 仍 16×16。
@compute @workgroup_size(8, 8)
fn main_tiled2(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    // 每 workgroup 覆盖一个 16×16 输出 tile：基址必须来自 workgroup 号，
    // 不能用 global_invocation_id（那是线程号，乘 16 会让相邻组互相覆盖输出）。
    let row_base = wg.y * 16u;
    let col_base = wg.x * 16u;
    let r0 = lid.y * 2u;
    let c0 = lid.x * 2u;
    let idx = lid.y * 8u + lid.x;
    var sum00 = 0.0;
    var sum01 = 0.0;
    var sum10 = 0.0;
    var sum11 = 0.0;
    let k_tiles = (params.k + TILE - 1u) / TILE;
    for (var chunk = 0u; chunk < k_tiles; chunk = chunk + 1u) {
        // 64 线程各装 4 槽：tile_a[r][kk] / tile_b[c][kk]，r、c 覆盖 0..16。
        for (var i = 0u; i < 4u; i = i + 1u) {
            let s = idx + i * 64u;
            let r = s / 16u;
            let kk = chunk * TILE + s % 16u;
            let a_ok = row_base + r < params.m && kk < params.k;
            let b_ok = col_base + r < params.n && kk < params.k;
            tile_a[s] = select(0.0, fa[(row_base + r) * params.k + kk], a_ok);
            // B^T 行主序：tile_b[c][kk] = fb[(col_base + c) * k + kk]。
            tile_b[s] = select(0.0, fb[(col_base + r) * params.k + kk], b_ok);
        }
        workgroupBarrier();
        for (var kk = 0u; kk < TILE; kk = kk + 1u) {
            let a0 = tile_a[r0 * TILE + kk];
            let a1 = tile_a[(r0 + 1u) * TILE + kk];
            let b0 = tile_b[c0 * TILE + kk];
            let b1 = tile_b[(c0 + 1u) * TILE + kk];
            sum00 = sum00 + a0 * b0;
            sum01 = sum01 + a0 * b1;
            sum10 = sum10 + a1 * b0;
            sum11 = sum11 + a1 * b1;
        }
        workgroupBarrier();
    }
    let row0 = row_base + r0;
    let col0 = col_base + c0;
    if (row0 < params.m && col0 < params.n) {
        out[row0 * params.n + col0] = sum00;
    }
    if (row0 < params.m && col0 + 1u < params.n) {
        out[row0 * params.n + col0 + 1u] = sum01;
    }
    if (row0 + 1u < params.m && col0 < params.n) {
        out[(row0 + 1u) * params.n + col0] = sum10;
    }
    if (row0 + 1u < params.m && col0 + 1u < params.n) {
        out[(row0 + 1u) * params.n + col0 + 1u] = sum11;
    }
}