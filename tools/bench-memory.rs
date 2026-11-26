// rustc --edition 2024 -O tools/bench-memory.rs -o bench-memory
#[path = "../crates/victory-gpu/src/shadow/difference.rs"]
mod improved;
mod previous {
/// Where `a` and `b` (of one length) first differ, compared eight bytes at
/// a time.
pub fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let whole = a.len().min(b.len()) / 8 * 8;
    let from = a[..whole]
        .as_chunks::<8>().0.iter()
        .zip(b[..whole].chunks_exact(8))
        .position(|(x, y)| x != y)
        .map_or(whole, |word| word * 8);
    a[from..].iter().zip(&b[from..]).position(|(x, y)| x != y).map(|i| from + i)
}

/// Where `a` and `b` (of one length) last differ.
pub fn last_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let whole = a.len().min(b.len()) / 8 * 8;
    if let Some(i) = a[whole..].iter().zip(&b[whole..]).rposition(|(x, y)| x != y) {
        return Some(whole + i);
    }
    let word = a[..whole]
        .as_chunks::<8>().0.iter()
        .zip(b[..whole].chunks_exact(8))
        .rposition(|(x, y)| x != y)?
        * 8;
    a[word..word + 8]
        .iter()
        .zip(&b[word..word + 8])
        .rposition(|(x, y)| x != y)
        .map(|i| word + i)
}

}
use std::hint::black_box;
use std::time::Instant;
fn measure(f: fn(&[u8], &[u8]) -> Option<usize>, a: &[u8], b: &[u8]) -> u128 {
    let start = Instant::now();
    for _ in 0..100_000 { black_box(f(black_box(a), black_box(b))); }
    start.elapsed().as_micros()
}
fn main() {
    let a = vec![3u8; 65536];
    for case in ["equal", "middle", "last", "first"] {
        let mut b = a.clone();
        match case { "middle" => b[32768] ^= 1, "last" => b[65535] ^= 1, "first" => b[0] ^= 1, _ => {} }
        for (label, old, new) in [
            ("first", previous::first_difference as fn(&[u8], &[u8]) -> Option<usize>, improved::first_difference as fn(&[u8], &[u8]) -> Option<usize>),
            ("last", previous::last_difference as fn(&[u8], &[u8]) -> Option<usize>, improved::last_difference as fn(&[u8], &[u8]) -> Option<usize>),
        ] {
            assert_eq!(old(&a,&b),new(&a,&b));
            let before = measure(old,&a,&b);
            let after = measure(new,&a,&b);
            println!("{case} {label}: before={before}us after={after}us ratio={:.2}x",before as f64/after as f64);
        }
    }
}
