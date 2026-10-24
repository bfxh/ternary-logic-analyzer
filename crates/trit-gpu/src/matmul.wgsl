// 三值矩阵乘内核（双导轨 + 四组 POPC）与同结构 f32 基线。
//
// C[m][n] = Σ_k A[m][k]·B[k][n]。B 按**转置**行主序存放（b_pos[n][w] 是 B 的第 n 列），
// 于是每个输出就是两次行向量的三值点积——与 dot 内核同一公式：
//   Σ popcount(ap&bp) + popcount(an&bn) − popcount(ap&bn) − popcount(an&bp)
// 两侧都是 naive「一线程一输出、循环内积维度」，唯一变量是数据表示（2 bit/trit vs 32 bit）。

struct Params {
    m: u32,
    n: u32,
    words: u32,
};

@group(0) @binding(0) var<storage, read> a_pos: array<u32>;
@group(0) @binding(1) var<storage, read> a_neg: array<u32>;
@group(0) @binding(2) var<storage, read> b_pos: array<u32>;
@group(0) @binding(3) var<storage, read> b_neg: array<u32>;
@group(0) @binding(4) var<storage, read_write> out: array<i32>;
@group(0) @binding(5) var<uniform> params: Params;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x;
    if (index >= params.m * params.n) {
        return;
    }
    let row = index / params.n;
    let col = index % params.n;
    let a_base = row * params.words;
    let b_base = col * params.words;
    var sum = 0;
    for (var w = 0u; w < params.words; w = w + 1u) {
        let ap = a_pos[a_base + w];
        let an = a_neg[a_base + w];
        let bp = b_pos[b_base + w];
        let bn = b_neg[b_base + w];
        sum = sum
            + i32(countOneBits(ap & bp))
            + i32(countOneBits(an & bn))
            - i32(countOneBits(ap & bn))
            - i32(countOneBits(an & bp));
    }
    out[index] = sum;
}
