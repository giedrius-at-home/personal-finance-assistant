//! TimesFM 3.0 inference on the CPU: a Rust port of the PyTorch reference
//! (`timesfm3.torch`, timesfm 3.0.2) as the Python finance service calls
//! it — `TimesFM3Forecaster.predict_batch(contexts, horizon,
//! past_future_covariates, return_quantiles=True, make_positive=True)`.
//!
//! The pipeline per batch of four series: left-pad to a whole number of
//! 32-point patches, remove a linear trend where it explains most of the
//! spread, normalise each patch by the running mean and spread so far
//! (RevIN), embed each patch with the next two patches of any past-and-future
//! covariate, run 20 transformer layers (causal attention over patches with
//! rotary positions, then attention across the series' variates), read nine
//! deciles per future point, refine the horizon's RevIN statistics with the
//! model's own median estimates, stitch overlapping patch forecasts, add the
//! trend back, sort the deciles and clip at zero for non-negative histories.
//!
//! [`prep`] holds the numeric pieces around the network; [`network`] the
//! transformer. Weights load from the published `model.safetensors`
//! (`google/timesfm-3.0-pytorch`), read tensor by tensor without mmap.

mod network;
pub mod prep;
mod safetensors;

use std::path::{Path, PathBuf};

use finance_core::daily::SeriesForecaster;
use ndarray::Array2;

// Links Accelerate for ndarray's BLAS matrix products.
#[cfg(target_os = "macos")]
use blas_src as _;
use network::{Network, Shape};
use prep::{MAX_CONTEXT, OUTPUT_PATCH, PATCH, QUANTILES, ROW, Variate};

/// The Hugging Face repository of the checkpoint this port reproduces.
pub const REPOSITORY: &str = "google/timesfm-3.0-pytorch";
/// The published revision this port was checked against.
pub const REVISION: &str = "43046b85ec22d584a13f8098c2ed39c889e129c2";

/// One file of the checkpoint at [`REVISION`].
#[derive(Clone, Copy, Debug)]
pub struct CheckpointFile {
    pub name: &'static str,
    pub bytes: u64,
    /// Lowercase hex SHA-256 of the content.
    pub sha256: &'static str,
}

impl CheckpointFile {
    /// Where Hugging Face serves it.
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/{REPOSITORY}/resolve/{REVISION}/{}",
            self.name
        )
    }
}

/// The checkpoint's files, as published.
pub const FILES: [CheckpointFile; 2] = [
    CheckpointFile {
        name: "config.json",
        bytes: 1273,
        sha256: "ff17bbc07b792c5a904cca265b8468579d736a4fe84981da25eb871b0a125bc6",
    },
    CheckpointFile {
        name: "model.safetensors",
        bytes: 1_322_898_824,
        sha256: "a7592b0a8432baee54483254e5647856911ce69e09d09a9bb65904b2d98f17da",
    },
];

/// A checkpoint directory holds every file at its published size.
pub fn is_complete(dir: &Path) -> bool {
    FILES.iter().all(|f| {
        std::fs::metadata(dir.join(f.name)).is_ok_and(|m| m.is_file() && m.len() == f.bytes)
    })
}
/// The name daily forecasts report (Python strips the `google/` prefix).
pub const NAME: &str = "timesfm-3.0-pytorch";
/// Series per forward pass (Python `per_core_batch_size`). Batches pad to
/// their longest member, so this also fixes the results.
const BATCH: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("TimesFM checkpoint: {0}")]
    Checkpoint(String),
    #[error("TimesFM checkpoint: {0}")]
    Io(#[from] std::io::Error),
    #[error("TimesFM input: {0}")]
    Input(String),
}

/// A loaded TimesFM 3.0 model (about 1.3 GB in memory).
#[derive(Debug)]
pub struct TimesFm {
    network: Network,
}

/// One series ready for `decode`: a context of the batch's length, its
/// padding mask and, when the batch has them, the covariate through the
/// horizon.
struct Item {
    target: Vec<f32>,
    mask: Vec<bool>,
    covariate: Option<Vec<f32>>,
}

/// Configuration values this port hard-codes, checked against the
/// checkpoint's `config.json` when it is present.
const EXPECTED_CONFIG: &[(&str, &str)] = &[
    ("/input_patch_len", "32"),
    ("/output_patch_len", "64"),
    ("/quantiles", "[0.1,0.2,0.3,0.4,0.5,0.6,0.7,0.8,0.9]"),
    ("/input_transform", "\"identity\""),
    ("/linear_detrending_threshold", "0.5"),
    ("/use_linear_detrending", "true"),
    ("/use_stitching", "true"),
    ("/use_iterative_cpm_revin", "true"),
    ("/use_frozen_running_stats", "false"),
    ("/use_variate_attention", "true"),
    ("/residual_block_config/hidden_dims", "1280"),
    ("/residual_block_config/output_dims", "1280"),
    ("/residual_block_config/activation", "\"relu\""),
    ("/residual_block_config/use_bias", "false"),
    ("/residual_block_config/identity_skip", "false"),
    ("/residual_block_config/prenorm", "\"none\""),
    ("/transformer_config/num_layers", "20"),
    ("/transformer_config/transformer/model_dims", "1280"),
    ("/transformer_config/transformer/hidden_dims", "1280"),
    ("/transformer_config/transformer/num_heads", "16"),
    ("/transformer_config/transformer/qk_norm", "\"rms\""),
    ("/transformer_config/transformer/v_norm", "\"none\""),
    ("/transformer_config/transformer/ff_activation", "\"relu\""),
    ("/transformer_config/transformer/use_bias", "false"),
    ("/transformer_config/transformer/use_rope_seq", "true"),
    ("/transformer_config/transformer/use_rope_var", "false"),
    ("/transformer_config/transformer/causal_attention", "true"),
    (
        "/transformer_config/transformer/use_memory_efficient_attention",
        "true",
    ),
];

fn check_config(path: &Path) -> Result<(), Error> {
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|e| Error::Checkpoint(format!("config.json: {e}")))?;
    for (pointer, expected) in EXPECTED_CONFIG {
        let expected: serde_json::Value = serde_json::from_str(expected)
            .map_err(|e| Error::Checkpoint(format!("expected {pointer}: {e}")))?;
        if config.pointer(pointer) != Some(&expected) {
            return Err(Error::Checkpoint(format!(
                "config.json {pointer} is not {expected}"
            )));
        }
    }
    Ok(())
}

/// The checkpoint file in `dir` (or `dir` itself when it is the file).
pub fn weights_path(dir: &Path) -> PathBuf {
    if dir.extension().is_some_and(|e| e == "safetensors") {
        dir.to_owned()
    } else {
        dir.join("model.safetensors")
    }
}

/// The current snapshot of [`REPOSITORY`] in a Hugging Face hub cache
/// (`~/.cache/huggingface/hub`), when all its files are there.
pub fn hub_snapshot(hub: &Path) -> Option<PathBuf> {
    let repo = hub.join(format!("models--{}", REPOSITORY.replace('/', "--")));
    let revision = std::fs::read_to_string(repo.join("refs/main")).ok()?;
    let snapshot = repo.join("snapshots").join(revision.trim());
    is_complete(&snapshot).then_some(snapshot)
}

/// The Hugging Face hub cache: `$HF_HUB_CACHE`, `$HF_HOME/hub` or
/// `~/.cache/huggingface/hub`.
pub fn hub_cache() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = var("HF_HUB_CACHE") {
        return Some(dir.into());
    }
    if let Some(home) = var("HF_HOME") {
        return Some(PathBuf::from(home).join("hub"));
    }
    var("HOME").map(|home| PathBuf::from(home).join(".cache/huggingface/hub"))
}

impl TimesFm {
    /// Load the weights from a checkpoint directory (with `config.json`
    /// and `model.safetensors`) or the `.safetensors` file itself.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let weights = weights_path(path);
        let config = weights.with_file_name("config.json");
        if config.is_file() {
            check_config(&config)?;
        }
        let mut checkpoint = safetensors::Checkpoint::open(&weights)?;
        Ok(Self {
            network: Network::load(&mut checkpoint)?,
        })
    }

    /// Nine deciles (q0.1..q0.9) per future step for each context, as
    /// Python's `predict_batch(..., return_quantiles=True,
    /// make_positive=True)` returns them. A covariate covers its context
    /// and the horizon (`context.len() + horizon` values).
    pub fn predict(
        &self,
        contexts: &[Vec<f32>],
        horizon: usize,
        covariates: Option<&[Option<Vec<f32>>]>,
    ) -> Result<Vec<Vec<[f32; QUANTILES]>>, Error> {
        if horizon == 0 {
            return Err(Error::Input("horizon must be positive".into()));
        }
        if covariates.is_some_and(|c| c.len() != contexts.len()) {
            return Err(Error::Input("one covariate slot per context".into()));
        }
        for (i, context) in contexts.iter().enumerate() {
            let covariate = covariates.and_then(|c| c[i].as_deref());
            if context.is_empty() {
                return Err(Error::Input("empty context".into()));
            }
            if !context
                .iter()
                .chain(covariate.unwrap_or(&[]))
                .all(|v| v.is_finite())
            {
                return Err(Error::Input("values must be finite".into()));
            }
            if covariate.is_some_and(|c| c.len() <= context.len()) {
                return Err(Error::Input(
                    "a covariate must reach past its context".into(),
                ));
            }
        }
        let global_horizon = horizon.div_ceil(OUTPUT_PATCH) * OUTPUT_PATCH;
        let mut outputs = Vec::with_capacity(contexts.len());
        for (index, batch) in contexts.chunks(BATCH).enumerate() {
            let covariates = covariates.map(|c| &c[index * BATCH..][..batch.len()]);
            let longest = batch.iter().map(Vec::len).max().unwrap_or(0);
            let context = (longest.div_ceil(PATCH) * PATCH).clamp(PATCH, MAX_CONTEXT);
            let mut items: Vec<Item> = batch
                .iter()
                .enumerate()
                .map(|(i, target)| {
                    format(target, covariates.and_then(|c| c[i].as_deref()), context)
                })
                .collect();
            // A series without covariates in a batch with them gets zeros.
            let width = items
                .iter()
                .find_map(|i| i.covariate.as_ref().map(Vec::len));
            if let Some(width) = width {
                for item in &mut items {
                    item.covariate.get_or_insert_with(|| vec![0.0; width]);
                }
            }
            let decode_horizon = width.map_or(global_horizon, |w| w - context);
            if decode_horizon == 0 {
                return Err(Error::Input(
                    "covariates must reach past the context".into(),
                ));
            }
            outputs.extend(self.decode(&items, context, decode_horizon));
        }
        Ok(outputs
            .into_iter()
            .zip(contexts)
            .map(|(rows, context)| {
                let positive = context.iter().all(|v| *v >= 0.0);
                rows.into_iter()
                    .take(horizon)
                    .map(|mut row| {
                        row.sort_by(f32::total_cmp);
                        if positive {
                            row = row.map(|v| v.max(0.0));
                        }
                        row
                    })
                    .collect()
            })
            .collect())
    }

    /// `TimesFM3Torch.decode` for one batch of formatted series.
    fn decode(&self, items: &[Item], context: usize, horizon: usize) -> Vec<Vec<[f32; QUANTILES]>> {
        let (forecast_patches, horizon_patches) = prep::horizon_patches(horizon);
        let padded = horizon_patches * PATCH;
        let context_patches = context / PATCH;
        let patches = context_patches + horizon_patches;
        let variates = if items.iter().any(|i| i.covariate.is_some()) {
            2
        } else {
            1
        };
        let shape = Shape {
            series: items.len(),
            variates,
            patches,
        };
        let mut input = Array2::<f32>::zeros((shape.rows(), ROW));
        let mut masked = vec![false; shape.rows()];
        let mut targets = Vec::with_capacity(items.len());
        for (b, item) in items.iter().enumerate() {
            let mut series = Vec::with_capacity(variates);
            let mut values = item.target.clone();
            let mut masks = item.mask.clone();
            let trend = prep::detrend(&mut values, &masks);
            zero_masked(&mut values, &masks);
            values.resize(context + padded, 0.0);
            masks.resize(context + padded, true);
            series.push((
                Variate {
                    values,
                    masks,
                    target: true,
                },
                trend,
            ));
            if let Some(covariate) = &item.covariate {
                let mut values = covariate[..context].to_vec();
                let mut masks = item.mask.clone();
                let trend = prep::detrend(&mut values, &masks);
                zero_masked(&mut values, &masks);
                for step in 0..horizon {
                    values.push(covariate[context + step] - trend.at(step + 1, context));
                    masks.push(false);
                }
                values.resize(context + padded, 0.0);
                masks.resize(context + padded, true);
                series.push((
                    Variate {
                        values,
                        masks,
                        target: false,
                    },
                    trend,
                ));
            }
            for (v, (variate, trend)) in series.into_iter().enumerate() {
                let stats = prep::running_stats(&variate.values, &variate.masks);
                let (rows, fully_masked) = prep::resblock_rows(&variate, &stats);
                for (p, lead) in prep::leading(&fully_masked).into_iter().enumerate() {
                    let row = shape.row(b, v, p);
                    masked[row] = lead;
                    input
                        .row_mut(row)
                        .iter_mut()
                        .zip(&rows[p * ROW..(p + 1) * ROW])
                        .for_each(|(to, from)| *to = *from);
                }
                if v == 0 {
                    targets.push((stats, trend));
                }
            }
        }
        let wanted: Vec<usize> = (0..items.len())
            .flat_map(|b| (0..patches).map(move |p| shape.row(b, 0, p)))
            .collect();
        let raw = self.network.forward(&input, shape, &masked, &wanted);
        let raw = raw.as_standard_layout();
        let raw = raw.as_slice().unwrap_or(&[]);
        targets
            .into_iter()
            .enumerate()
            .map(|(b, (stats, trend))| {
                let raw = &raw[b * patches * prep::HEAD..(b + 1) * patches * prep::HEAD];
                let refined = prep::refine(raw, &stats, context_patches);
                let forecasts: Vec<_> = (0..forecast_patches)
                    .map(|k| {
                        let patch = context_patches - 1 + k;
                        prep::denormalize(raw, patch, refined[patch])
                    })
                    .collect();
                let mut rows = prep::stitch(&forecasts);
                rows.truncate(horizon);
                for (step, row) in rows.iter_mut().enumerate() {
                    let line = trend.at(step + 1, context);
                    row.iter_mut().for_each(|v| *v += line);
                }
                rows
            })
            .collect()
    }
}

fn zero_masked(values: &mut [f32], masks: &[bool]) {
    for (v, m) in values.iter_mut().zip(masks) {
        if *m {
            *v = 0.0;
        }
    }
}

/// `_Query.format`: left-pad (masked) or keep the last `context` points; a
/// covariate keeps its future part.
fn format(target: &[f32], covariate: Option<&[f32]>, context: usize) -> Item {
    let len = target.len();
    if len >= context {
        let future = covariate.map_or(0, |c| c.len() - len);
        return Item {
            target: target[len - context..].to_vec(),
            mask: vec![false; context],
            covariate: covariate.map(|c| c[c.len() - (context + future)..].to_vec()),
        };
    }
    let pad = context - len;
    let padded = |values: &[f32]| {
        let mut out = vec![0.0; pad];
        out.extend_from_slice(values);
        out
    };
    let mut mask = vec![true; pad];
    mask.resize(context, false);
    Item {
        target: padded(target),
        mask,
        covariate: covariate.map(padded),
    }
}

/// NumPy's float32 mean of nine values: pairwise summation (eight partial
/// sums, then the ninth), divided in float32.
fn numpy_mean(row: &[f32; QUANTILES]) -> f32 {
    let sum = ((row[0] + row[1]) + (row[2] + row[3])) + ((row[4] + row[5]) + (row[6] + row[7]));
    (sum + row[8]) / QUANTILES as f32
}

/// The service's daily rows: q0.1, the deciles' mean (most days have no
/// spending, so the median would understate totals) and q0.9.
pub fn service_rows(deciles: &[[f32; QUANTILES]]) -> Vec<[f64; 3]> {
    deciles
        .iter()
        .map(|q| {
            [
                f64::from(q[0]),
                f64::from(numpy_mean(q)),
                f64::from(q[QUANTILES - 1]),
            ]
        })
        .collect()
}

impl SeriesForecaster for TimesFm {
    fn name(&self) -> &str {
        NAME
    }

    fn supports_covariates(&self) -> bool {
        true
    }

    fn quantiles(
        &self,
        contexts: &[Vec<f64>],
        horizon: usize,
        covariates: Option<&[Vec<f64>]>,
    ) -> Result<Vec<Vec<[f64; 3]>>, String> {
        let narrow = |v: &Vec<f64>| v.iter().map(|x| *x as f32).collect::<Vec<f32>>();
        let contexts: Vec<Vec<f32>> = contexts.iter().map(narrow).collect();
        let covariates: Option<Vec<Option<Vec<f32>>>> =
            covariates.map(|c| c.iter().map(|v| Some(narrow(v))).collect());
        self.predict(&contexts, horizon, covariates.as_deref())
            .map(|outputs| outputs.iter().map(|d| service_rows(d)).collect())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_pads_on_the_left_and_keeps_the_covariate_future() {
        let item = format(&[1.0, 2.0], Some(&[5.0, 6.0, 7.0]), 4);
        assert_eq!(item.target, [0.0, 0.0, 1.0, 2.0]);
        assert_eq!(item.mask, [true, true, false, false]);
        assert_eq!(item.covariate.unwrap(), [0.0, 0.0, 5.0, 6.0, 7.0]);
        let item = format(&[1.0, 2.0, 3.0], Some(&[4.0, 5.0, 6.0, 7.0]), 2);
        assert_eq!(item.target, [2.0, 3.0]);
        assert_eq!(item.covariate.unwrap(), [5.0, 6.0, 7.0]);
    }

    #[test]
    fn service_rows_take_the_outer_deciles_and_their_mean() {
        let rows = service_rows(&[[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]]);
        assert_eq!(rows, [[1.0, 5.0, 9.0]]);
    }

    #[test]
    fn checkpoints_are_found_in_a_hub_cache() {
        let hub = std::env::temp_dir().join(format!("timesfm-hub-{}", std::process::id()));
        let repo = hub.join("models--google--timesfm-3.0-pytorch");
        assert_eq!(hub_snapshot(&hub), None);
        std::fs::create_dir_all(repo.join("refs")).unwrap();
        std::fs::create_dir_all(repo.join("snapshots/abc")).unwrap();
        std::fs::write(repo.join("refs/main"), "abc\n").unwrap();
        let snapshot = repo.join("snapshots/abc");
        std::fs::write(snapshot.join("config.json"), vec![b' '; 1273]).unwrap();
        assert_eq!(hub_snapshot(&hub), None);
        // Sparse: the published size without writing 1.3 GB.
        let weights = std::fs::File::create(snapshot.join("model.safetensors")).unwrap();
        weights.set_len(FILES[1].bytes - 1).unwrap();
        assert_eq!(hub_snapshot(&hub), None);
        weights.set_len(FILES[1].bytes).unwrap();
        assert_eq!(hub_snapshot(&hub), Some(snapshot));
        assert_eq!(
            weights_path(&repo.join("snapshots/abc")),
            repo.join("snapshots/abc/model.safetensors")
        );
        std::fs::remove_dir_all(&hub).unwrap();
    }

    #[test]
    fn bad_inputs_are_refused_before_any_work() {
        let config = std::env::temp_dir().join(format!("timesfm-cfg-{}.json", std::process::id()));
        std::fs::write(&config, r#"{"input_patch_len": 16}"#).unwrap();
        assert!(check_config(&config).is_err());
        std::fs::remove_file(&config).unwrap();
    }
}
