//! 二进制/三值边界的吞吐基准。
//!
//! `cargo run --release -p trit-core --example bulk_bench`

use std::time::{Duration, Instant};

use trit_core::{Algebra, TriVec};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    let n = 1usize << 20;
    let a: Vec<bool> = (0..n).map(|i| i % 3 != 0).collect();
    let b: Vec<bool> = (0..n).map(|i| i % 5 != 0).collect();

    let t = Instant::now();
    let mut va = TriVec::from_bools(&a);
    let pack = t.elapsed();

    let t = Instant::now();
    let vb = TriVec::from_bools(&b);
    let pack_b = t.elapsed();

    let t = Instant::now();
    va.and_assign(&vb, Algebra::Kleene).unwrap();
    let and = t.elapsed();

    let t = Instant::now();
    let out = va.to_bools().unwrap();
    let unpack = t.elapsed();

    // 分配放在计时外：这里量的是 move-only 视图本身。
    let zeros = vec![0u64; n.div_ceil(64)];
    let t = Instant::now();
    let _binary = TriVec::from_binary(zeros, n);
    let move_view = t.elapsed();

    let checksum = out.iter().filter(|&&x| x).count();
    println!("n={n} (checksum={checksum})");
    println!(
        "pack       {:8.3} ms  {:6.1} M elem/s",
        ms(pack),
        mps(n, pack)
    );
    println!(
        "pack b     {:8.3} ms  {:6.1} M elem/s",
        ms(pack_b),
        mps(n, pack_b)
    );
    println!(
        "K3 and     {:8.3} ms  {:6.1} M elem/s",
        ms(and),
        mps(n, and)
    );
    println!(
        "unpack     {:8.3} ms  {:6.1} M elem/s",
        ms(unpack),
        mps(n, unpack)
    );
    println!("binary view {:7.3} ms  (move only, no copy)", ms(move_view));
}

fn mps(n: usize, d: Duration) -> f64 {
    n as f64 / d.as_secs_f64() / 1e6
}
