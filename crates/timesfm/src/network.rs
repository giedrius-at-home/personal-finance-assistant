//! The network: residual-block embedding, 20 mixing-transformer layers
//! (sequence attention with RoPE, variate attention, feed-forward) and the
//! quantile output head (`timesfm3/torch`: `dense.py`, `transformer.py`).
//!
//! Activations are row matrices: one row per (series, variate, patch), in
//! that order, so a series-variate's patches are contiguous.

use ndarray::{Array1, Array2, ArrayView2, Axis};

use crate::Error;
use crate::prep::{HEAD, ROW};
use crate::safetensors::Checkpoint;

const MODEL: usize = 1280;
const HEADS: usize = 16;
const HEAD_DIM: usize = MODEL / HEADS;
const LAYERS: usize = 20;
/// `nn.RMSNorm(eps=None)`: float32 machine epsilon.
const RMS_EPS: f32 = f32::EPSILON;
/// `normalization._RECIPROCAL_OF_SOFTPLUS_0`: 1 / ln 2.
const RECIPROCAL_OF_SOFTPLUS_0: f32 = std::f32::consts::LOG2_E;
const ROPE_MAX_TIMESCALE: f32 = 10_000.0;

#[derive(Debug)]
struct Linear {
    /// (out, in), as stored.
    weight: Array2<f32>,
    bias: Option<Array1<f32>>,
}

impl Linear {
    fn load(
        checkpoint: &mut Checkpoint,
        name: &str,
        out: usize,
        input: usize,
        bias: bool,
    ) -> Result<Self, Error> {
        let weight = checkpoint.read(&format!("{name}.weight"), &[out, input])?;
        let weight = Array2::from_shape_vec((out, input), weight)
            .map_err(|e| Error::Checkpoint(e.to_string()))?;
        let bias = if bias {
            Some(Array1::from(
                checkpoint.read(&format!("{name}.bias"), &[out])?,
            ))
        } else {
            None
        };
        Ok(Self { weight, bias })
    }

    fn forward(&self, x: ArrayView2<f32>) -> Array2<f32> {
        let mut y = x.dot(&self.weight.t());
        if let Some(bias) = &self.bias {
            y += bias;
        }
        y
    }
}

fn vector(checkpoint: &mut Checkpoint, name: &str, len: usize) -> Result<Array1<f32>, Error> {
    Ok(Array1::from(checkpoint.read(name, &[len])?))
}

/// `nn.RMSNorm` over each row (or each `weight.len()`-wide chunk of a row).
fn rms_norm(x: &mut [f32], weight: &Array1<f32>) {
    let width = weight.len();
    for chunk in x.chunks_exact_mut(width) {
        let squares: f64 = chunk.iter().map(|v| f64::from(v * v)).sum();
        let inverse = 1.0 / ((squares / width as f64) as f32 + RMS_EPS).sqrt();
        for (v, w) in chunk.iter_mut().zip(weight) {
            *v = *v * inverse * w;
        }
    }
}

fn normed(x: &Array2<f32>, weight: &Array1<f32>) -> Array2<f32> {
    let mut y = x.as_standard_layout().into_owned();
    rms_norm(y.as_slice_mut().unwrap_or(&mut []), weight);
    y
}

/// `F.softplus` with PyTorch's default threshold.
fn softplus(x: f32) -> f32 {
    if x > 20.0 { x } else { x.exp().ln_1p() }
}

#[derive(Debug)]
struct Attention {
    query: Linear,
    key: Linear,
    value: Linear,
    out: Linear,
    query_ln: Array1<f32>,
    key_ln: Array1<f32>,
    /// `PerDimScale`: the query's learned per-dimension scale.
    query_scale: Vec<f32>,
    rope: bool,
    causal: bool,
}

impl Attention {
    fn load(
        checkpoint: &mut Checkpoint,
        name: &str,
        rope: bool,
        causal: bool,
    ) -> Result<Self, Error> {
        let mut linear =
            |part: &str| Linear::load(checkpoint, &format!("{name}.{part}"), MODEL, MODEL, false);
        let (query, key, value, out) = (
            linear("query_proj")?,
            linear("key_proj")?,
            linear("value_proj")?,
            linear("out_proj")?,
        );
        let per_dim =
            checkpoint.read(&format!("{name}.per_dim_scale.per_dim_scale"), &[HEAD_DIM])?;
        let root = (HEAD_DIM as f32).sqrt();
        Ok(Self {
            query,
            key,
            value,
            out,
            query_ln: vector(checkpoint, &format!("{name}.query_ln.weight"), HEAD_DIM)?,
            key_ln: vector(checkpoint, &format!("{name}.key_ln.weight"), HEAD_DIM)?,
            query_scale: per_dim
                .into_iter()
                .map(|s| RECIPROCAL_OF_SOFTPLUS_0 / root * softplus(s))
                .collect(),
            rope,
            causal,
        })
    }

    /// Multi-head attention within groups of `group` consecutive rows; a
    /// masked row is never attended to, and a query that may attend to
    /// nothing gets zeros (as `F.scaled_dot_product_attention` does).
    fn forward(&self, x: ArrayView2<f32>, group: usize, masked: &[bool]) -> Array2<f32> {
        let rows = x.nrows();
        let mut q = self.query.forward(x);
        let mut k = self.key.forward(x);
        let v = self.value.forward(x);
        let (Some(qs), Some(ks), Some(vs)) = (q.as_slice_mut(), k.as_slice_mut(), v.as_slice())
        else {
            return Array2::zeros((rows, MODEL));
        };
        if self.rope {
            let table = rope_table(group);
            for (r, (qr, kr)) in qs
                .as_chunks_mut::<MODEL>()
                .0
                .iter_mut()
                .zip(ks.as_chunks_mut::<MODEL>().0)
                .enumerate()
            {
                let angles = &table[(r % group) * HEAD_DIM / 2..][..HEAD_DIM / 2];
                rotate(qr, angles);
                rotate(kr, angles);
            }
        }
        rms_norm(qs, &self.query_ln);
        rms_norm(ks, &self.key_ln);
        for head in qs.as_chunks_mut::<HEAD_DIM>().0 {
            for (v, s) in head.iter_mut().zip(&self.query_scale) {
                *v *= s;
            }
        }
        // Flax-style memory-efficient attention: logits scaled by sqrt(d).
        let logit_scale = (HEAD_DIM as f32).sqrt();
        let mut context = vec![0f32; rows * MODEL];
        let mut weights = vec![0f32; group];
        for start in (0..rows).step_by(group) {
            for h in 0..HEADS {
                let offset = h * HEAD_DIM;
                for i in 0..group {
                    let qi = &qs[(start + i) * MODEL + offset..][..HEAD_DIM];
                    let mut max = f32::NEG_INFINITY;
                    let mut any = false;
                    for (j, w) in weights.iter_mut().enumerate() {
                        let allowed = (!self.causal || j <= i) && !masked[start + j];
                        *w = if allowed {
                            let kj = &ks[(start + j) * MODEL + offset..][..HEAD_DIM];
                            let dot: f32 = qi.iter().zip(kj).map(|(a, b)| a * b).sum();
                            let logit = dot * logit_scale;
                            max = max.max(logit);
                            any = true;
                            logit
                        } else {
                            f32::NEG_INFINITY
                        };
                    }
                    if !any {
                        continue;
                    }
                    let mut total = 0f64;
                    for w in weights.iter_mut() {
                        *w = if w.is_finite() { (*w - max).exp() } else { 0.0 };
                        total += f64::from(*w);
                    }
                    let total = total as f32;
                    let out = &mut context[(start + i) * MODEL + offset..][..HEAD_DIM];
                    for (j, w) in weights.iter().enumerate() {
                        if *w == 0.0 {
                            continue;
                        }
                        let p = w / total;
                        let vj = &vs[(start + j) * MODEL + offset..][..HEAD_DIM];
                        for (o, value) in out.iter_mut().zip(vj) {
                            *o += p * value;
                        }
                    }
                }
            }
        }
        let context = Array2::from_shape_vec((rows, MODEL), context).unwrap_or_default();
        self.out.forward(context.view())
    }
}

/// `RotaryPositionalEmbedding` angles `position / timescale` for each of
/// `positions` positions and half a head's dimensions.
fn rope_table(positions: usize) -> Vec<f32> {
    let half = HEAD_DIM / 2;
    let timescale: Vec<f32> = (0..half)
        .map(|j| {
            let fraction = 2.0 * j as f32 / HEAD_DIM as f32;
            ROPE_MAX_TIMESCALE.powf(fraction)
        })
        .collect();
    (0..positions)
        .flat_map(|p| timescale.iter().map(move |t| p as f32 / t))
        .collect()
}

/// Rotate every head of one row: first and second halves as pairs.
fn rotate(row: &mut [f32], angles: &[f32]) {
    let half = HEAD_DIM / 2;
    for head in row.as_chunks_mut::<HEAD_DIM>().0 {
        let (first, second) = head.split_at_mut(half);
        for ((a, b), angle) in first.iter_mut().zip(second.iter_mut()).zip(angles) {
            let (sin, cos) = angle.sin_cos();
            let (x, y) = (*a, *b);
            *a = x * cos - y * sin;
            *b = y * cos + x * sin;
        }
    }
}

#[derive(Debug)]
struct Layer {
    pre_seq: Array1<f32>,
    post_seq: Array1<f32>,
    seq: Attention,
    pre_var: Array1<f32>,
    post_var: Array1<f32>,
    var: Attention,
    pre_ff: Array1<f32>,
    post_ff: Array1<f32>,
    ff0: Linear,
    ff1: Linear,
}

impl Layer {
    fn load(checkpoint: &mut Checkpoint, index: usize) -> Result<Self, Error> {
        let name = format!("transformer_stack.layers.{index}");
        let mut norm = |part: &str| vector(checkpoint, &format!("{name}.{part}.weight"), MODEL);
        let (pre_seq, post_seq, pre_var, post_var, pre_ff, post_ff) = (
            norm("pre_seq_attn_ln")?,
            norm("post_seq_attn_ln")?,
            norm("pre_var_attn_ln")?,
            norm("post_var_attn_ln")?,
            norm("pre_ff_ln")?,
            norm("post_ff_ln")?,
        );
        Ok(Self {
            pre_seq,
            post_seq,
            seq: Attention::load(checkpoint, &format!("{name}.seq_attn"), true, true)?,
            pre_var,
            post_var,
            var: Attention::load(checkpoint, &format!("{name}.var_attn"), false, false)?,
            pre_ff,
            post_ff,
            ff0: Linear::load(checkpoint, &format!("{name}.ff0"), MODEL, MODEL, false)?,
            ff1: Linear::load(checkpoint, &format!("{name}.ff1"), MODEL, MODEL, false)?,
        })
    }

    /// `MixingTransformer.forward` on rows ordered (series, variate, patch).
    fn forward(&self, x: Array2<f32>, shape: Shape, masked: &[bool]) -> Array2<f32> {
        let seq = self
            .seq
            .forward(normed(&x, &self.pre_seq).view(), shape.patches, masked);
        let h1 = normed(&seq, &self.post_seq) + &x;

        let h2 = if shape.variates == 1 {
            let var = self
                .var
                .forward(normed(&h1, &self.pre_var).view(), 1, masked);
            normed(&var, &self.post_var) + &h1
        } else {
            // Regroup rows to (series, patch, variate) for attention across variates.
            let order = shape.variate_major();
            let mut back = vec![0; order.len()];
            for (to, from) in order.iter().enumerate() {
                back[*from] = to;
            }
            let regrouped = normed(&h1, &self.pre_var).select(Axis(0), &order);
            let regrouped_mask: Vec<bool> = order.iter().map(|r| masked[*r]).collect();
            let var = self
                .var
                .forward(regrouped.view(), shape.variates, &regrouped_mask)
                .select(Axis(0), &back);
            normed(&var, &self.post_var) + &h1
        };

        let mut hidden = self.ff0.forward(normed(&h2, &self.pre_ff).view());
        hidden.mapv_inplace(|v| v.max(0.0));
        let ff = self.ff1.forward(hidden.view());
        normed(&ff, &self.post_ff) + &h2
    }
}

/// The batch's dimensions.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    pub series: usize,
    pub variates: usize,
    pub patches: usize,
}

impl Shape {
    pub(crate) fn rows(self) -> usize {
        self.series * self.variates * self.patches
    }

    pub(crate) fn row(self, series: usize, variate: usize, patch: usize) -> usize {
        (series * self.variates + variate) * self.patches + patch
    }

    /// Row indices in (series, patch, variate) order.
    fn variate_major(self) -> Vec<usize> {
        let mut order = Vec::with_capacity(self.rows());
        for b in 0..self.series {
            for p in 0..self.patches {
                for v in 0..self.variates {
                    order.push(self.row(b, v, p));
                }
            }
        }
        order
    }
}

/// The loaded network weights (about 1.3 GB of float32).
#[derive(Debug)]
pub(crate) struct Network {
    hidden: Linear,
    output: Linear,
    residual: Linear,
    layers: Vec<Layer>,
    head: Linear,
}

impl Network {
    pub(crate) fn load(checkpoint: &mut Checkpoint) -> Result<Self, Error> {
        let block = "pre_transformer_resblock";
        Ok(Self {
            hidden: Linear::load(
                checkpoint,
                &format!("{block}.hidden_layer"),
                MODEL,
                ROW,
                false,
            )?,
            output: Linear::load(
                checkpoint,
                &format!("{block}.output_layer"),
                MODEL,
                MODEL,
                false,
            )?,
            residual: Linear::load(
                checkpoint,
                &format!("{block}.residual_layer"),
                MODEL,
                ROW,
                false,
            )?,
            layers: (0..LAYERS)
                .map(|i| Layer::load(checkpoint, i))
                .collect::<Result<_, _>>()?,
            head: Linear::load(checkpoint, "output_head", HEAD, MODEL, true)?,
        })
    }

    /// Embed the residual-block input rows, run the transformer and return
    /// the output head's raw (normalised) logits for the `wanted` rows.
    /// `masked` marks the leading padding patches attention skips.
    pub(crate) fn forward(
        &self,
        input: &Array2<f32>,
        shape: Shape,
        masked: &[bool],
        wanted: &[usize],
    ) -> Array2<f32> {
        let mut hidden = self.hidden.forward(input.view());
        hidden.mapv_inplace(|v| v.max(0.0));
        let mut x = self.output.forward(hidden.view()) + self.residual.forward(input.view());
        for layer in &self.layers {
            x = layer.forward(x, shape, masked);
        }
        let selected = x.select(Axis(0), wanted);
        self.head.forward(selected.view())
    }
}
