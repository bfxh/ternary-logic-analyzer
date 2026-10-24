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

// 分块版（共享内存）：16×16 的输出 tile，一次协作加载 8-word（256 trit）的
// A 行段与 B 列段进 workgroup storage——A 段被 tile 内 16 个列线程复用、
// B 段被 16 个行线程复用，把重复读砍 16×。同 bind group、同数据，只有映射不同。
const TILED: u32 = 16u;
const TILED_WORDS: u32 = 8u;

var<workgroup> tile_ap: array<u32, 512>;
var<workgroup> tile_an: array<u32, 512>;
var<workgroup> tile_bp: array<u32, 512>;
var<workgroup> tile_bn: array<u32, 512>;

@compute @workgroup_size(16, 16)
fn main_tiled(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    let row = gid.y;
    let col = gid.x;
    let linear = lid.y * TILED + lid.x;
    var sum = 0;
    let word_tiles = (params.words + TILED_WORDS - 1u) / TILED_WORDS;
    for (var chunk = 0u; chunk < word_tiles; chunk = chunk + 1u) {
        // 每段 8 字、tile 有 16×16 线程：A 由 lid.x<8 的线程装入 lid.y 行的 8 个槽，
        // B 由 lid.y<8 的线程装入 lid.x 列的 8 个槽——槽位布局必须与下方累加读取
        // 一致（否则就是「加载 16 宽、累加 8 宽」的错位，读进相邻 chunk 的字）。
        if (lid.x < TILED_WORDS) {
            let a_word = chunk * TILED_WORDS + lid.x;
            let a_ok = row < params.m && a_word < params.words;
            let slot = lid.y * TILED_WORDS + lid.x;
            tile_ap[slot] = select(0u, a_pos[row * params.words + a_word], a_ok);
            tile_an[slot] = select(0u, a_neg[row * params.words + a_word], a_ok);
        }
        if (lid.y < TILED_WORDS) {
            let b_word = chunk * TILED_WORDS + lid.y;
            let b_ok = col < params.n && b_word < params.words;
            let slot = lid.x * TILED_WORDS + lid.y;
            tile_bp[slot] = select(0u, b_pos[col * params.words + b_word], b_ok);
            tile_bn[slot] = select(0u, b_neg[col * params.words + b_word], b_ok);
        }
        workgroupBarrier();
        for (var w = 0u; w < TILED_WORDS; w = w + 1u) {
            let ap = tile_ap[lid.y * TILED_WORDS + w];
            let an = tile_an[lid.y * TILED_WORDS + w];
            let bp = tile_bp[lid.x * TILED_WORDS + w];
            let bn = tile_bn[lid.x * TILED_WORDS + w];
            sum = sum
                + i32(countOneBits(ap & bp))
                + i32(countOneBits(an & bn))
                - i32(countOneBits(ap & bn))
                - i32(countOneBits(an & bp));
        }
        workgroupBarrier();
    }
    if (row < params.m && col < params.n) {
        out[row * params.n + col] = sum;
    }
}
