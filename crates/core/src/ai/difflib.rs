//! Python's `difflib.SequenceMatcher` over characters (no junk function,
//! `autojunk=True`): `ratio` and `quick_ratio`, used by lexical retrieval.

use std::collections::HashMap;

#[derive(Debug)]
pub struct SequenceMatcher {
    a: Vec<char>,
    b: Vec<char>,
    /// Positions of each non-popular character of `b`.
    b2j: HashMap<char, Vec<usize>>,
}

impl SequenceMatcher {
    pub fn new(a: &str, b: &str) -> Self {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
        for (j, c) in b.iter().enumerate() {
            b2j.entry(*c).or_default().push(j);
        }
        // Autojunk: characters in more than 1% of a long `b` are "popular".
        if b.len() >= 200 {
            let ntest = b.len() / 100 + 1;
            b2j.retain(|_, positions| positions.len() <= ntest);
        }
        Self { a, b, b2j }
    }

    /// `find_longest_match` (without junk, the extension loops only add
    /// equal neighbours, popular characters included).
    fn longest(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut size) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut next: HashMap<usize, usize> = HashMap::new();
            if let Some(positions) = self.b2j.get(&self.a[i]) {
                for &j in positions {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j
                        .checked_sub(1)
                        .and_then(|p| j2len.get(&p))
                        .copied()
                        .unwrap_or(0)
                        + 1;
                    next.insert(j, k);
                    if k > size {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        size = k;
                    }
                }
            }
            j2len = next;
        }
        while besti > alo && bestj > blo && self.a[besti - 1] == self.b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            size += 1;
        }
        while besti + size < ahi
            && bestj + size < bhi
            && self.a[besti + size] == self.b[bestj + size]
        {
            size += 1;
        }
        (besti, bestj, size)
    }

    /// The total size of `get_matching_blocks`.
    fn matches(&self) -> usize {
        let mut queue = vec![(0, self.a.len(), 0, self.b.len())];
        let mut total = 0;
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest(alo, ahi, blo, bhi);
            if k > 0 {
                total += k;
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        total
    }

    fn scaled(&self, matches: usize) -> f64 {
        let length = self.a.len() + self.b.len();
        if length == 0 {
            1.0
        } else {
            2.0 * matches as f64 / length as f64
        }
    }

    pub fn ratio(&self) -> f64 {
        self.scaled(self.matches())
    }

    /// An upper bound on `ratio`: shared characters regardless of order.
    pub fn quick_ratio(&self) -> f64 {
        let mut available: HashMap<char, i64> = HashMap::new();
        for c in &self.b {
            *available.entry(*c).or_default() += 1;
        }
        let mut matches = 0;
        for c in &self.a {
            let n = available.entry(*c).or_default();
            if *n > 0 {
                matches += 1;
            }
            *n -= 1;
        }
        self.scaled(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference values from CPython 3.14's difflib.
    #[test]
    fn matches_cpython() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;
        let m = SequenceMatcher::new("albert heijn", "albert heyn");
        assert!(close(m.ratio(), 0.869_565_217_391_304_3));
        assert!(close(m.quick_ratio(), 0.869_565_217_391_304_3));
        assert!(close(SequenceMatcher::new("abc", "xyz").ratio(), 0.0));
        assert!(close(
            SequenceMatcher::new("jumbo supermarkt", "jumbo").ratio(),
            0.476_190_476_190_476_16
        ));
        let a = format!("{}b", "a".repeat(250));
        let b = format!("{}bc", "a".repeat(240));
        assert!(close(
            SequenceMatcher::new(&a, &b).ratio(),
            0.977_687_626_774_847_9
        ));
    }
}
