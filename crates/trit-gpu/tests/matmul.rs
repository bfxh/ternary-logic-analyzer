//! 三值矩阵乘：GPU 内核与 CPU 位并行镜像（`matmul_words`）逐元素对拍。
//! 没有可用 adapter 时跳过（与 `gpu::run` 同约定）。

#![cfg(feature = "gpu")]

use trit_gpu::{matmul, matmul_words, pack_columns, pack_rows};

#[test]
fn gpu_matmul_matches_cpu_mirror() {
    // 非整字边界（k=100 → 4 字 + 28 位零尾）+ 非方阵，边界与转置一起钉。
    let m = 33usize;
    let k = 100usize;
    let n = 17usize;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % 3) as i32 - 1
    };
    let a: Vec<i32> = (0..m * k).map(|_| next()).collect();
    let b: Vec<i32> = (0..k * n).map(|_| next()).collect();
    let (a_pos, a_neg) = pack_rows(&a, k);
    let (b_pos, b_neg) = pack_columns(&b, k, n);
    let expected = matmul_words(m, n, &a_pos, &a_neg, &b_pos, &b_neg);
    // 域检查：每个输出是 k 个 ±1 乘积之和，|值| ≤ k（runner 内部逐元素对拍
    // 依赖 matmul_words，这里先钉住它自己的取值域，防「两边一起错」）。
    assert_eq!(expected.len(), m * n);
    assert!(expected.iter().all(|value| value.abs() <= k as i64));

    let report = matmul::run(m, k, n);
    let Some(report) = report else {
        eprintln!("没有可用 GPU adapter，对拍跳过");
        return;
    };
    assert!(report.matches, "GPU 与 CPU 镜像不一致：{report:?}");
    assert_eq!(report.m, m);
    assert_eq!(report.n, n);
    assert_eq!(report.k, k);
}
