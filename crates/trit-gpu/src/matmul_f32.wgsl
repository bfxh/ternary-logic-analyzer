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
