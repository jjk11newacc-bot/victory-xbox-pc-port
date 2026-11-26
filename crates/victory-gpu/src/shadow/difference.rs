//! Exact changed-byte boundaries, skipping equal cache-line blocks.
//! Slice comparisons require no alignment or read-past-the-end assumptions.
const BLOCK: usize = 64;
const REGION: usize = 4096;

// Compare large equal spans in one slice comparison before locating the
// exact differing cache line. No probabilistic hash or skipped bytes.
fn first_region(a: &[u8], b: &[u8]) -> Option<usize> {
    a.chunks(REGION).zip(b.chunks(REGION)).position(|(x,y)| x != y)
}
fn last_region(a: &[u8], b: &[u8]) -> Option<usize> {
    a.chunks(REGION).zip(b.chunks(REGION)).rposition(|(x,y)| x != y)
}

pub(crate) fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let n = a.len().min(b.len());
    if n == 0 { return None; }
    if a[0] != b[0] { return Some(0); }
    let region = first_region(&a[..n], &b[..n])? * REGION;
    let end = (region + REGION).min(n);
    let (a, b) = (&a[region..end], &b[region..end]);
    let n = a.len();
    let whole = n / BLOCK * BLOCK;
    let from = a[..whole].chunks_exact(BLOCK)
        .zip(b[..whole].chunks_exact(BLOCK))
        .position(|(x, y)| x != y)
        .map_or(whole, |block| block * BLOCK);
    let end = (from + BLOCK).min(n);
    a[from..end].iter().zip(&b[from..end])
        .position(|(x, y)| x != y).map(|i| region + from + i)
}

pub(crate) fn last_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let n = a.len().min(b.len());
    if n == 0 { return None; }
    if a[n - 1] != b[n - 1] { return Some(n - 1); }
    let region = last_region(&a[..n], &b[..n])? * REGION;
    let end = (region + REGION).min(n);
    let (a, b) = (&a[region..end], &b[region..end]);
    let n = a.len();
    let whole = n / BLOCK * BLOCK;
    if let Some(i) = a[whole..n].iter().zip(&b[whole..n]).rposition(|(x, y)| x != y) {
        return Some(region + whole + i);
    }
    let from = a[..whole].chunks_exact(BLOCK)
        .zip(b[..whole].chunks_exact(BLOCK))
        .rposition(|(x, y)| x != y)? * BLOCK;
    a[from..from + BLOCK].iter().zip(&b[from..from + BLOCK])
        .rposition(|(x, y)| x != y).map(|i| region + from + i)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_tails_unaligned_and_multiple_changes() {
        for offset in 0..16 {
            for n in [0usize, 1, 7, 8, 15, 16, 63, 64, 65, 127, 128, 129, 257, 4095, 4096, 4097, 8193, 65536] {
                let base = vec![0x5a; n + offset];
                let a = &base[offset..];
                let mut b = a.to_vec();
                assert_eq!(first_difference(a, &b), None);
                assert_eq!(last_difference(a, &b), None);
                for at in [0, 1, 7, 8, 15, 16, 63, 64, 65, 4095, 4096, 4097, n / 2, n.saturating_sub(1)] {
                    if at >= n { continue; }
                    b[at] ^= 1;
                    assert_eq!(first_difference(a, &b), Some(at));
                    assert_eq!(last_difference(a, &b), Some(at));
                    b[n - 1] ^= 2;
                    assert_eq!(first_difference(a, &b), Some(at));
                    assert_eq!(last_difference(a, &b), Some(n - 1));
                    b[n - 1] ^= 2;
                    b[at] ^= 1;
                }
            }
        }
    }
    #[test]
    fn randomized_patterns_and_unequal_lengths_match_scalar_oracle() {
        let mut seed = 7u64;
        for n in 0..2048 {
            let mut a = vec![0; n];
            let mut b = vec![0; n + 3];
            for i in 0..n {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                a[i] = (seed >> 32) as u8;
                b[i] = a[i] ^ u8::from(seed % 97 == 0);
            }
            assert_eq!(first_difference(&a, &b), a.iter().zip(&b).position(|(x,y)| x != y));
            assert_eq!(last_difference(&a, &b), a.iter().zip(&b).rposition(|(x,y)| x != y));
        }
    }
}
