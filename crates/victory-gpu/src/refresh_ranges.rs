//! Bounded sparse refresh coverage. This changes scheduling, not guest ownership.
/// Keep nearby intervals together to avoid trading a small copy for another
/// kernel call. Fall back to the old envelope for badly fragmented inputs.
const GAP: usize = 64 * 1024;
const MAX_PARTS: usize = 64;

pub(super) struct Coverage {
    bounds: (usize, usize),
    parts: Vec<(usize, usize)>,
    sparse: bool,
}

impl Coverage {
    pub(super) fn new(from: usize, to: usize, sparse: bool) -> Self {
        Self { bounds: (from, to), parts: Vec::new(), sparse }
    }

    pub(super) fn bounds(&self) -> (usize, usize) { self.bounds }

    pub(super) fn ranges(&self) -> &[(usize, usize)] {
        if self.parts.is_empty() { std::slice::from_ref(&self.bounds) } else { &self.parts }
    }

    pub(super) fn add(&mut self, from: usize, to: usize) {
        let old = self.bounds;
        self.bounds = (old.0.min(from), old.1.max(to));
        if !self.sparse { return; }
        if self.parts.is_empty() {
            if from > old.1.saturating_add(GAP) {
                self.parts.extend([old, (from, to)]);
            } else if old.0 > to.saturating_add(GAP) {
                self.parts.extend([(from, to), old]);
            }
            return;
        }
        let first = self.parts.partition_point(|&(_, end)| end.saturating_add(GAP) < from);
        let last = first + self.parts[first..].partition_point(|&(start, _)| start <= to.saturating_add(GAP));
        if first == last {
            if self.parts.len() == MAX_PARTS {
                self.parts.clear();
                self.sparse = false;
            } else {
                self.parts.insert(first, (from, to));
            }
        } else {
            self.parts[first] = (self.parts[first].0.min(from), self.parts[last - 1].1.max(to));
            self.parts.drain(first + 1..last);
            if self.parts.len() == 1 { self.parts.clear(); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distant_ranges_leave_holes_and_bridge_when_requested() {
        let mut c = Coverage::new(0, 4096, true);
        c.add(1 << 20, (1 << 20) + 4096);
        c.add(8192, 16384);
        assert_eq!(c.ranges(), &[(0, 16384), (1 << 20, (1 << 20) + 4096)]);
        c.add(16384, 1 << 20);
        assert_eq!(c.ranges(), &[(0, (1 << 20) + 4096)]);
    }
    #[test]
    fn legacy_and_fragmentation_fallback_keep_the_full_envelope() {
        for sparse in [false, true] {
            let mut c = Coverage::new(0, 1, sparse);
            for i in 1..=MAX_PARTS+3 { c.add(i * GAP * 2, i * GAP * 2 + 1); }
            assert_eq!(c.ranges(), &[c.bounds()]);
        }
    }
    #[test]
    fn all_requested_bytes_remain_covered_in_unsorted_workloads() {
        let mut seed = 19u64;
        for _ in 0..100 {
            let mut c = Coverage::new(10, 20, true);
            let mut requested = vec![(10, 20)];
            for _ in 0..128 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let a = ((seed >> 32) as usize % 4096) * 4096;
                let b = a + (seed as usize % 16384) + 1;
                c.add(a, b);
                requested.push((a, b));
                for &(a, b) in &requested {
                    assert!(c.ranges().iter().any(|&(x, y)| x <= a && b <= y));
                }
                assert!(c.ranges().windows(2).all(|w| w[0].1.saturating_add(GAP) < w[1].0));
                assert_eq!(c.ranges().first().unwrap().0, c.bounds().0);
                assert_eq!(c.ranges().last().unwrap().1, c.bounds().1);
            }
        }
    }
}
