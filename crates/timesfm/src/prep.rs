//! Everything around the network, in plain float32 arithmetic that follows
//! the PyTorch reference (`timesfm3/torch`: `model.py` `decode`/`forward`,
//! `util.py`, `cpm_revin_refine.py`) operation for operation: linear
//! detrending, patch-wise running statistics (RevIN), the residual block's
//! input rows, iterative CPM RevIN refinement and stitching. Sums accumulate
//! in f64, which is at least as exact as PyTorch's vectorised f32 sums.

/// Points per input patch.
pub const PATCH: usize = 32;
/// Points each patch predicts.
pub const OUTPUT_PATCH: usize = 64;
/// Output patches per input patch.
pub const ROLLS: usize = OUTPUT_PATCH / PATCH;
/// The deciles q0.1..q0.9.
pub const QUANTILES: usize = 9;
/// The median decile, the point estimate the RevIN refinement follows.
pub const MEDIAN: usize = QUANTILES / 2;
/// One residual-block input row: patch and next-patches values, then masks.
pub const ROW: usize = 2 * (PATCH + OUTPUT_PATCH);
/// One output-head row: every predicted point's nine deciles.
pub const HEAD: usize = OUTPUT_PATCH * QUANTILES;
/// The longest context the model reads (Python `_MAX_CONTEXT_LENGTH`).
pub const MAX_CONTEXT: usize = 15360;
/// Values are clipped to this magnitude.
pub const VALUE_CLIP: f32 = 1e20;
/// Stitched predictions take this many points of each patch's output.
const STITCH_EXTRACT: usize = if 2 * PATCH < OUTPUT_PATCH {
    2 * PATCH
} else {
    OUTPUT_PATCH
};
const STITCH_OVERLAP: usize = STITCH_EXTRACT - PATCH;
const TOLERANCE: f32 = 1e-6;
const DETREND_THRESHOLD: f32 = 0.5;

/// Running count, mean and standard deviation of the valid points so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub n: f32,
    pub mu: f32,
    pub sigma: f32,
}

/// `util.update_running_stats` for one variate: fold one patch into `prev`.
pub fn update_stats(prev: Stats, x: &[f32], masked: impl Fn(usize) -> bool) -> Stats {
    let (mut count, mut sum) = (0u32, 0f64);
    for (i, v) in x.iter().enumerate() {
        if !masked(i) {
            count += 1;
            sum += f64::from(*v);
        }
    }
    let inc_n = count as f32;
    let inc_sum = sum as f32;
    let inc_mu = if inc_n == 0.0 { 0.0 } else { inc_sum / inc_n };
    let mut squares = 0f64;
    for (i, v) in x.iter().enumerate() {
        if !masked(i) {
            let d = v - inc_mu;
            squares += f64::from(d * d);
        }
    }
    let inc_var = if inc_n == 0.0 {
        0.0
    } else {
        squares as f32 / inc_n
    };
    let inc_sigma = inc_var.sqrt();
    let Stats { n, mu, sigma } = prev;
    let new_n = n + inc_n;
    let new_mu = if new_n == 0.0 {
        0.0
    } else {
        (n * mu + inc_mu * inc_n) / new_n
    };
    let new_var = if new_n == 0.0 {
        0.0
    } else {
        (n * sigma * sigma
            + inc_n * inc_sigma * inc_sigma
            + n * (mu - new_mu) * (mu - new_mu)
            + inc_n * (inc_mu - new_mu) * (inc_mu - new_mu))
            / new_n
    };
    Stats {
        n: new_n,
        mu: new_mu,
        sigma: new_var.sqrt(),
    }
}

/// `util.get_running_stats` for one variate: the stats after each patch.
pub fn running_stats(values: &[f32], masks: &[bool]) -> Vec<Stats> {
    let mut stats = Stats::default();
    values
        .as_chunks::<PATCH>()
        .0
        .iter()
        .zip(masks.as_chunks::<PATCH>().0)
        .map(|(x, m)| {
            stats = update_stats(stats, x, |i| m[i]);
            stats
        })
        .collect()
}

/// RevIN's divisor: near-zero spreads divide by one.
fn safe(sigma: f32) -> f32 {
    if sigma < TOLERANCE { 1.0 } else { sigma }
}

/// `util.revin(reverse=False)`.
pub fn normalize(x: f32, stats: Stats) -> f32 {
    (x - stats.mu) / safe(stats.sigma)
}

/// A context's least-squares line over normalised time `t / context`, and
/// whether removing it shrank the spread enough to be applied.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Trend {
    pub slope: f32,
    pub intercept: f32,
    pub apply: bool,
}

impl Trend {
    /// The line at future step `step` (1-based) of a `context`-long context;
    /// zero when the trend is not applied.
    pub fn at(self, step: usize, context: usize) -> f32 {
        if self.apply {
            self.slope * (step as f32 / context as f32) + self.intercept
        } else {
            0.0
        }
    }
}

/// `decode`'s linear detrending of one context variate, in place.
pub fn detrend(values: &mut [f32], masks: &[bool]) -> Trend {
    let context = values.len();
    let time = |i: usize| (i as f32 - (context as f32 - 1.0)) / context as f32;
    let sum = |f: &dyn Fn(usize) -> f32| -> f32 {
        (0..context)
            .filter(|i| !masks[*i])
            .map(|i| f64::from(f(i)))
            .sum::<f64>() as f32
    };
    let n = masks.iter().filter(|m| !**m).count() as f32;
    let sum_t = sum(&|i| time(i));
    let sum_t2 = sum(&|i| time(i) * time(i));
    let sum_y = sum(&|i| values[i]);
    let sum_ty = sum(&|i| time(i) * values[i]);
    let det = n * sum_t2 - sum_t * sum_t;
    let n1 = n.max(1.0);
    let slope = if det == 0.0 {
        0.0
    } else {
        (n * sum_ty - sum_t * sum_y) / det
    };
    let intercept = if det == 0.0 {
        if n > 0.0 { sum_y / n1 } else { 0.0 }
    } else {
        (sum_y - slope * sum_t) / n1
    };
    let detrended: Vec<f32> = (0..context)
        .map(|i| values[i] - (slope * time(i) + intercept))
        .collect();
    let spread = |v: &[f32]| {
        let total = sum(&|i| v[i]);
        let squares = sum(&|i| v[i] * v[i]);
        let mean = total / n1;
        (squares / n1 - mean * mean).max(0.0).sqrt()
    };
    let std_orig = spread(values);
    let std_det = spread(&detrended);
    let apply = std_det < DETREND_THRESHOLD * std_orig;
    if apply {
        values.copy_from_slice(&detrended);
    }
    Trend {
        slope,
        intercept,
        apply,
    }
}

/// One variate of one series, patched: `values.len()` is a multiple of
/// [`PATCH`]. Masked points (padding, the target's horizon) are zero.
#[derive(Clone, Debug)]
pub struct Variate {
    pub values: Vec<f32>,
    pub masks: Vec<bool>,
    pub target: bool,
}

/// `forward._preprocess` for one variate: each patch's residual-block input
/// row (its normalised points, the next [`ROLLS`] patches' points normalised
/// with this patch's stats, then both masks as 0/1) and whether the patch is
/// fully masked.
pub fn resblock_rows(variate: &Variate, stats: &[Stats]) -> (Vec<f32>, Vec<bool>) {
    let patches = stats.len();
    let mut rows = Vec::with_capacity(patches * ROW);
    let mut fully_masked = Vec::with_capacity(patches);
    for (i, s) in stats.iter().enumerate() {
        let mut row = [0f32; ROW];
        let mut all = true;
        for p in 0..PATCH {
            let at = i * PATCH + p;
            let masked = variate.masks[at];
            row[p] = if masked {
                0.0
            } else {
                normalize(variate.values[at], *s)
            };
            row[PATCH + OUTPUT_PATCH + p] = f32::from(u8::from(masked));
            all &= masked;
        }
        for j in 0..OUTPUT_PATCH {
            let source = i + 1 + j / PATCH;
            let at = (source % patches) * PATCH + j % PATCH;
            let masked = variate.masks[at] || variate.target || source >= patches;
            row[PATCH + j] = if masked {
                0.0
            } else {
                normalize(variate.values[at], *s)
            };
            row[2 * PATCH + OUTPUT_PATCH + j] = f32::from(u8::from(masked));
            all &= masked;
        }
        rows.extend_from_slice(&row);
        fully_masked.push(all);
    }
    (rows, fully_masked)
}

/// Inference masks only the leading run of fully masked patches (padding).
pub fn leading(fully_masked: &[bool]) -> Vec<bool> {
    let mut still = true;
    fully_masked
        .iter()
        .map(|m| {
            still &= *m;
            still
        })
        .collect()
}

/// `cpm_iterative_revin_refine` for one variate: from patch `horizon_from`
/// on, the stats also count the model's own median estimates of the masked
/// horizon patches before it. `raw` holds each patch's [`HEAD`] logits.
pub fn refine(raw: &[f32], stats: &[Stats], horizon_from: usize) -> Vec<Stats> {
    let mut carry = Stats::default();
    let mut anchor = [[0f32; PATCH]; ROLLS];
    let mut offset = 0;
    stats
        .iter()
        .enumerate()
        .map(|(i, actual)| {
            let horizon = i >= horizon_from;
            let estimate = update_stats(carry, &anchor[offset], |_| false);
            let out = if horizon { estimate } else { *actual };
            offset = if horizon { (offset + 1) % ROLLS } else { 0 };
            if offset == 0 {
                for (r, points) in anchor.iter_mut().enumerate() {
                    for (p, point) in points.iter_mut().enumerate() {
                        let logit = raw[i * HEAD + (r * PATCH + p) * QUANTILES + MEDIAN];
                        *point = (logit * out.sigma + out.mu).clamp(-VALUE_CLIP, VALUE_CLIP);
                    }
                }
            }
            carry = out;
            out
        })
        .collect()
}

/// `torch.linspace(1, 0, steps)` in float32, as PyTorch computes it.
fn linspace_down(steps: usize) -> Vec<f32> {
    if steps == 1 {
        return vec![1.0];
    }
    let step = -1.0 / (steps as f32 - 1.0);
    let half = steps / 2;
    (0..steps)
        .map(|i| {
            if i < half {
                1.0 + step * i as f32
            } else {
                0.0 - step * (steps - 1 - i) as f32
            }
        })
        .collect()
}

/// `util.stitch_patches`: consecutive patch forecasts, each
/// [`STITCH_EXTRACT`] points long, blended linearly where they overlap.
pub fn stitch(patches: &[Vec<[f32; QUANTILES]>]) -> Vec<[f32; QUANTILES]> {
    let Some((first, rest)) = patches.split_first() else {
        return Vec::new();
    };
    if rest.is_empty() {
        return first.clone();
    }
    let weights = linspace_down(STITCH_OVERLAP);
    let mut out: Vec<[f32; QUANTILES]> = first[..PATCH].to_vec();
    for pair in patches.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);
        for (t, w) in weights.iter().enumerate() {
            let mut point = [0f32; QUANTILES];
            for (q, v) in point.iter_mut().enumerate() {
                *v = w * prev[PATCH + t][q] + (1.0 - w) * next[t][q];
            }
            out.push(point);
        }
        out.extend_from_slice(&next[STITCH_OVERLAP..PATCH]);
    }
    if let Some(last) = patches.last() {
        out.extend_from_slice(&last[PATCH..STITCH_EXTRACT]);
    }
    out
}

/// How a horizon is laid out with stitching: forecast patches read, and
/// masked horizon patches appended after the context.
pub fn horizon_patches(horizon: usize) -> (usize, usize) {
    let needed = horizon.saturating_sub(STITCH_OVERLAP).div_ceil(PATCH);
    let forecast = needed.max(1);
    (forecast, forecast + ROLLS - 1)
}

/// The output patch of `raw` row `patch` in data units: reverse RevIN with
/// `stats`, clipped, first [`STITCH_EXTRACT`] points.
pub fn denormalize(raw: &[f32], patch: usize, stats: Stats) -> Vec<[f32; QUANTILES]> {
    let row = &raw[patch * HEAD..(patch + 1) * HEAD];
    (0..STITCH_EXTRACT)
        .map(|t| {
            let mut point = [0f32; QUANTILES];
            for (q, v) in point.iter_mut().enumerate() {
                *v = (row[t * QUANTILES + q] * stats.sigma + stats.mu)
                    .clamp(-VALUE_CLIP, VALUE_CLIP);
            }
            point
        })
        .collect()
}

#[cfg(test)]
mod tests {
    //! Expected values were printed by the PyTorch reference
    //! (`timesfm3.torch.util`, `cpm_revin_refine`, `model.decode`'s
    //! detrending) on the same inputs.

    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-5 * (1.0 + b.abs())
    }

    fn assert_all_close(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len());
        for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
            assert!(close(*a, *b), "index {i}: {a} != {b}");
        }
    }

    include!("prep_cases.rs");

    #[test]
    fn running_stats_match_pytorch() {
        let stats = running_stats(&STATS_VALUES, &STATS_MASKS);
        let flat: Vec<f32> = stats.iter().flat_map(|s| [s.n, s.mu, s.sigma]).collect();
        assert_all_close(&flat, &STATS_EXPECTED);
    }

    #[test]
    fn detrending_matches_pytorch() {
        for (values, masks, expected, slope, intercept, apply) in DETREND_CASES {
            let mut values = values.to_vec();
            let trend = detrend(&mut values, masks);
            assert_eq!(trend.apply, *apply);
            assert!(close(trend.slope, *slope), "{} != {slope}", trend.slope);
            assert!(close(trend.intercept, *intercept));
            assert_all_close(&values, expected);
        }
    }

    #[test]
    fn resblock_rows_match_pytorch() {
        let variate = Variate {
            values: ROWS_VALUES.to_vec(),
            masks: ROWS_MASKS.to_vec(),
            target: false,
        };
        let stats = running_stats(&variate.values, &variate.masks);
        let (rows, masked) = resblock_rows(&variate, &stats);
        assert_all_close(&rows, &ROWS_EXPECTED);
        assert_eq!(masked, ROWS_FULLY_MASKED);
        let target = Variate {
            target: true,
            ..variate
        };
        let (rows, _) = resblock_rows(&target, &stats);
        // A target's next-patch points are always masked.
        assert!(
            rows.chunks(ROW)
                .all(|r| r[PATCH..PATCH + OUTPUT_PATCH].iter().all(|v| *v == 0.0))
        );
        assert_eq!(
            leading(&[true, true, false, true]),
            [true, true, false, false]
        );
    }

    #[test]
    fn refinement_matches_pytorch() {
        let stats: Vec<Stats> = REFINE_STATS
            .chunks(3)
            .map(|s| Stats {
                n: s[0],
                mu: s[1],
                sigma: s[2],
            })
            .collect();
        let refined = refine(&REFINE_RAW, &stats, REFINE_FROM);
        let flat: Vec<f32> = refined.iter().flat_map(|s| [s.mu, s.sigma]).collect();
        assert_all_close(&flat, &REFINE_EXPECTED);
    }

    #[test]
    fn stitching_matches_pytorch() {
        let patches: Vec<Vec<[f32; QUANTILES]>> = STITCH_INPUT
            .chunks(STITCH_EXTRACT * QUANTILES)
            .map(|p| p.chunks(QUANTILES).map(|q| q.try_into().unwrap()).collect())
            .collect();
        let stitched: Vec<f32> = stitch(&patches).into_iter().flatten().collect();
        assert_all_close(&stitched, &STITCH_EXPECTED);
        assert_eq!(stitch(&patches[..1]).len(), STITCH_EXTRACT);
    }

    #[test]
    fn horizons_past_one_patch_add_forecast_patches() {
        assert_eq!(horizon_patches(7), (1, 2));
        assert_eq!(horizon_patches(64), (1, 2));
        assert_eq!(horizon_patches(65), (2, 3));
        assert_eq!(horizon_patches(130), (4, 5));
    }
}
