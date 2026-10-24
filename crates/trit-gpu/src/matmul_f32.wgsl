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
