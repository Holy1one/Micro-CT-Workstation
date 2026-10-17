//! Bounded 3-D cone-beam PWLS-Huber correction of an FDK initializer.
//! Siddon ray lengths define an exact discrete A/A^T pair in millimetres.
//! Only a smooth, coarse correction is prolonged; FDK detail is retained.
//! Disjoint validation angles gate the fixed-iteration candidate, once.
use serde::Serialize;

#[derive(Clone, Copy)]
pub(super) struct Geometry {
    pub sod: f64,
    pub sdd: f64,
    pub width: f64,
    pub offset_u: f64,
    pub offset_v: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Report {
    pub algorithm: &'static str,
    pub grid: usize,
    pub fit_views: usize,
    pub validation_views: usize,
    pub iterations: usize,
    pub fit_before: f64,
    pub fit_after: f64,
    pub validation_before: f64,
    pub validation_after: f64,
    pub accepted: bool,
    pub reason: String,
    pub max_correction: f32,
    pub huber_delta: f32,
    pub limitation: &'static str,
}

#[derive(Default)]
struct Matrix {
    rows: Vec<usize>,
    indices: Vec<u32>,
    lengths: Vec<f32>,
    observed: Vec<f32>,
    weight: Vec<f32>,
    validation: Vec<bool>,
}

/// Ray from source at (sod*sin,-sod*cos,0) to the flat detector. Coordinates
/// exactly match FDK's along=sod+y*cos-x*sin, u=x*cos+y*sin convention.
fn trace(
    n: usize,
    voxel: f64,
    g: Geometry,
    theta: f64,
    u: f64,
    v: f64,
    mut visit: impl FnMut(usize, f32),
) {
    let (s, c) = theta.sin_cos();
    let origin = [g.sod * s, -g.sod * c, 0.];
    let direction = [-g.sdd * s + u * c, g.sdd * c + u * s, v];
    let distance = direction.iter().map(|x| x * x).sum::<f64>().sqrt();
    let half = n as f64 * voxel / 2.;
    let mut entry = 0f64;
    let mut exit = 1f64;
    for a in 0..3 {
        if direction[a].abs() < 1e-12 {
            if origin[a] < -half || origin[a] > half {
                return;
            }
        } else {
            let t0 = (-half - origin[a]) / direction[a];
            let t1 = (half - origin[a]) / direction[a];
            entry = entry.max(t0.min(t1));
            exit = exit.min(t0.max(t1));
        }
    }
    if entry >= exit {
        return;
    }
    let mut cell = [0isize; 3];
    let mut next = [f64::INFINITY; 3];
    let mut step = [0isize; 3];
    let mut increment = [f64::INFINITY; 3];
    for a in 0..3 {
        cell[a] = ((origin[a] + (entry + 1e-10) * direction[a] + half) / voxel)
            .floor()
            .clamp(0., (n - 1) as f64) as isize;
        if direction[a].abs() > 1e-12 {
            step[a] = if direction[a] > 0. { 1 } else { -1 };
            let boundary = -half + (cell[a] + if step[a] > 0 { 1 } else { 0 }) as f64 * voxel;
            next[a] = (boundary - origin[a]) / direction[a];
            increment[a] = voxel / direction[a].abs();
        }
    }
    let mut t = entry;
    for _ in 0..3 * n + 3 {
        let end = next.iter().copied().fold(exit, f64::min);
        if end > t + 1e-12 {
            visit(
                (cell[2] as usize * n + cell[1] as usize) * n + cell[0] as usize,
                ((end - t) * distance) as f32,
            );
        }
        if end >= exit - 1e-12 {
            break;
        }
        for a in 0..3 {
            if next[a] <= end + 1e-12 {
                cell[a] += step[a];
                next[a] += increment[a];
            }
        }
        if cell.iter().any(|&i| i < 0 || i >= n as isize) {
            break;
        }
        t = end;
    }
}

impl Matrix {
    fn forward(&self, x: &[f32]) -> Vec<f32> {
        self.rows
            .windows(2)
            .map(|range| {
                (range[0]..range[1])
                    .map(|k| self.lengths[k] * x[self.indices[k] as usize])
                    .sum()
            })
            .collect()
    }
    fn adjoint(&self, y: &[f32], size: usize) -> Vec<f32> {
        let mut out = vec![0.; size];
        for (r, range) in self.rows.windows(2).enumerate() {
            for k in range[0]..range[1] {
                out[self.indices[k] as usize] += self.lengths[k] * y[r];
            }
        }
        out
    }
    fn errors(&self, x: &[f32]) -> (f64, f64) {
        let p = self.forward(x);
        let mut e = [0f64; 2];
        let mut w = [0f64; 2];
        for (i, value) in p.iter().enumerate() {
            let b = usize::from(self.validation[i]);
            e[b] += f64::from(self.weight[i]) * (f64::from(*value - self.observed[i])).powi(2);
            w[b] += f64::from(self.weight[i]);
        }
        (e[0] / w[0].max(1e-20), e[1] / w[1].max(1e-20))
    }
}

fn percentile(values: &[f32], q: f32) -> f32 {
    let mut v = values.to_vec();
    let i = ((v.len() - 1) as f32 * q) as usize;
    *v.select_nth_unstable_by(i, f32::total_cmp).1
}

fn optimize(a: &Matrix, initial: &[f32], n: usize, iterations: usize) -> (Vec<f32>, f32) {
    let mut x = initial.to_vec();
    let row_sum: Vec<_> = a
        .rows
        .windows(2)
        .map(|r| a.lengths[r[0]..r[1]].iter().sum::<f32>())
        .collect();
    let weighted_sum: Vec<_> = row_sum
        .iter()
        .enumerate()
        .map(|(r, s)| if a.validation[r] { 0. } else { s * a.weight[r] })
        .collect();
    let mut diagonal = a.adjoint(&weighted_sum, initial.len());
    let positive: Vec<_> = diagonal.iter().copied().filter(|d| *d > 0.).collect();
    if positive.is_empty() {
        return (x, 0.);
    }
    let beta = percentile(&positive, 0.5) * 0.035;
    let range = (percentile(initial, 0.99) - percentile(initial, 0.01)).max(1e-6);
    let mut differences = Vec::new();
    for i in 0..x.len() {
        if i % n + 1 < n {
            differences.push((x[i + 1] - x[i]).abs());
        }
    }
    let delta = (percentile(&differences, 0.5) * 3.).max(range * 0.01);
    for d in &mut diagonal {
        *d = (*d + 6. * beta).max(1e-8);
    }
    for _ in 0..iterations {
        let predicted = a.forward(&x);
        let residual: Vec<_> = predicted
            .iter()
            .enumerate()
            .map(|(r, p)| {
                if a.validation[r] {
                    0.
                } else {
                    (p - a.observed[r]) * a.weight[r]
                }
            })
            .collect();
        let mut gradient = a.adjoint(&residual, x.len());
        for z in 0..n {
            for y in 0..n {
                for xx in 0..n {
                    let i = (z * n + y) * n + xx;
                    for j in [
                        (xx + 1 < n).then_some(i + 1),
                        (y + 1 < n).then_some(i + n),
                        (z + 1 < n).then_some(i + n * n),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let d = (x[i] - x[j]).clamp(-delta, delta) * beta;
                        gradient[i] += d;
                        gradient[j] -= d;
                    }
                }
            }
        }
        for i in 0..x.len() {
            x[i] -= 0.9 * gradient[i] / diagonal[i];
        }
    }
    (x, delta)
}

fn coarse(volume: &[f32], source_n: usize, n: usize) -> Vec<f32> {
    let f = source_n / n;
    let mut out = vec![0.; n * n * n];
    for z in 0..source_n {
        for y in 0..source_n {
            for x in 0..source_n {
                out[((z / f) * n + y / f) * n + x / f] +=
                    volume[(z * source_n + y) * source_n + x] / f.pow(3) as f32;
            }
        }
    }
    out
}

fn prolong_add(volume: &mut [f32], source_n: usize, delta: &[f32], n: usize) {
    let scale = n as f32 / source_n as f32;
    let coordinate = |i: usize| {
        let v = ((i as f32 + 0.5) * scale - 0.5).clamp(0., (n - 1) as f32);
        (v.floor() as usize, v - v.floor())
    };
    for z in 0..source_n {
        let (zz, tz) = coordinate(z);
        for y in 0..source_n {
            let (yy, ty) = coordinate(y);
            for x in 0..source_n {
                let (xx, tx) = coordinate(x);
                let mut d = 0.;
                for dz in 0..2 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let weight = if dx == 0 { 1. - tx } else { tx }
                                * if dy == 0 { 1. - ty } else { ty }
                                * if dz == 0 { 1. - tz } else { tz };
                            d += weight
                                * delta[(((zz + dz).min(n - 1)) * n + (yy + dy).min(n - 1)) * n
                                    + (xx + dx).min(n - 1)];
                        }
                    }
                }
                volume[(z * source_n + y) * source_n + x] += d;
            }
        }
    }
}

/// Variance weights use the delta-method dark/flat noise propagation before
/// normalization. They are approximate after projection smoothing and are not
/// photon-count estimates. Interpolated rays receive reduced confidence.
pub(super) fn refine(
    volume: &mut [f32],
    source_n: usize,
    stack: &[f32],
    flags: &[u8],
    angles: &[f64],
    g: Geometry,
    gain: &[f32],
    dark_variance: &[f32],
    flat_variance: &[f32],
) -> Report {
    let n = 64;
    let detector = 64;
    let raw_n = super::DETECTOR_N;
    let block = raw_n / detector;
    let stride = angles.len().div_ceil(72).max(1);
    let selected: Vec<_> = (0..angles.len()).step_by(stride).collect();
    let mut report=Report {algorithm:"FDK + bounded coarse 3D cone-beam PWLS-Huber correction; matched Siddon operators",grid:n,
        fit_views:selected.len()-selected.len()/4,validation_views:selected.len()/4,iterations:8,
        fit_before:0.,fit_after:0.,validation_before:0.,validation_after:0.,accepted:false,
        reason:"insufficient supported measurements".into(),max_correction:0.,huber_delta:0.,
        limitation:"64^3 additive correction with 64x64 detector and <=72 sampled angles; signed FDK detail retained. Circular-orbit incompleteness, scatter, motion and calibration mismatch remain. Validation residual is not anatomical ground truth."};
    if angles.len() < 32
        || source_n % n != 0
        || volume.len() != source_n.pow(3)
        || stack.len() != angles.len() * raw_n * raw_n
        || flags.len() != stack.len()
        || gain.len() != raw_n * raw_n
        || dark_variance.len() != gain.len()
        || flat_variance.len() != gain.len()
        || ![g.sod, g.sdd, g.width, g.offset_u, g.offset_v]
            .iter()
            .all(|v| v.is_finite())
        || g.sod <= 0.
        || g.sdd <= g.sod
        || g.width <= 0.
    {
        return report;
    }
    let voxel = g.width * g.sod / g.sdd / n as f64;
    let pitch = g.width * super::SCREEN_CROP_SCALE / detector as f64;
    let half = (detector - 1) as f64 / 2.;
    let mut a = Matrix {
        rows: vec![0],
        ..Default::default()
    };
    for (rank, &view) in selected.iter().enumerate() {
        for y in 0..detector {
            for x in 0..detector {
                if ((x as f64 - half).hypot(y as f64 - half)) > detector as f64 * 0.44 {
                    continue;
                }
                let mut observed = 0.;
                let mut weight = 0.;
                let mut good = 0;
                for dy in 0..block {
                    for dx in 0..block {
                        let j = (y * block + dy) * raw_n + x * block + dx;
                        let i = view * raw_n * raw_n + j;
                        let confidence = match flags[i] {
                            1 => 1.,
                            5 => 0.25,
                            6 => 0.1,
                            7 => 0.03,
                            _ => 0.,
                        };
                        if confidence == 0.
                            || !stack[i].is_finite()
                            || gain[j] <= 1.
                            || !gain[j].is_finite()
                        {
                            continue;
                        }
                        let transmission = (-stack[i]).exp().clamp(1e-4, 100.);
                        let var = dark_variance[j].max(1.) * 1.1 / (gain[j] * transmission).powi(2)
                            + flat_variance[j].max(0.) / (10. * gain[j].powi(2));
                        let w = confidence / var.clamp(1e-6, 1.);
                        observed += stack[i] * w;
                        weight += w;
                        good += 1;
                    }
                }
                if good < block * block * 3 / 4 || weight <= 0. {
                    continue;
                }
                let start = a.indices.len();
                trace(
                    n,
                    voxel,
                    g,
                    angles[view],
                    (x as f64 - half) * pitch - g.offset_u,
                    (y as f64 - half) * pitch - g.offset_v,
                    |i, l| {
                        a.indices.push(i as u32);
                        a.lengths.push(l);
                    },
                );
                if a.indices.len() == start {
                    continue;
                }
                a.rows.push(a.indices.len());
                a.observed.push(observed / weight);
                a.weight.push(weight / (block * block) as f32);
                a.validation.push(rank % 4 == 3);
            }
        }
    }
    if a.observed.len() < 1000 || !a.validation.iter().any(|&v| v) {
        return report;
    }
    let scale = percentile(&a.weight, 0.5).max(1.);
    for w in &mut a.weight {
        *w = (*w / scale).clamp(0.001, 10.);
    }
    let initial = coarse(volume, source_n, n);
    let before = a.errors(&initial);
    let (candidate, huber) = optimize(&a, &initial, n, report.iterations);
    let delta: Vec<_> = candidate.iter().zip(&initial).map(|(a, b)| a - b).collect();
    // Validate the actual prolonged result, not the unconstrained coarse
    // iterate: restriction after interpolation is not the identity operator.
    let mut proposed = volume.to_vec();
    prolong_add(&mut proposed, source_n, &delta, n);
    let after = a.errors(&coarse(&proposed, source_n, n));
    report.fit_before = before.0;
    report.validation_before = before.1;
    report.fit_after = after.0;
    report.validation_after = after.1;
    report.huber_delta = huber;
    report.max_correction = delta.iter().fold(0f32, |m, d| m.max(d.abs()));
    let range = (percentile(&initial, 0.99) - percentile(&initial, 0.01)).max(1e-6);
    if !candidate.iter().all(|x| x.is_finite()) || !after.0.is_finite() || !after.1.is_finite() {
        report.reason = "non-finite candidate; FDK retained".into();
    } else if after.0 >= before.0 * 0.98 || after.1 >= before.1 * 0.98 {
        report.reason =
            "less than 2% fit and disjoint-angle validation improvement; FDK retained".into();
    } else if report.max_correction > range * 0.5 {
        report.reason = "correction exceeds half the robust FDK range; FDK retained".into();
    } else {
        volume.copy_from_slice(&proposed);
        report.accepted = true;
        report.reason="fixed-iteration candidate passed fit, disjoint-angle validation and correction-size gates".into();
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    fn geometry() -> Geometry {
        Geometry {
            sod: 50.,
            sdd: 85.,
            width: 40.,
            offset_u: 0.,
            offset_v: 0.,
        }
    }
    fn phantom_matrix(n: usize) -> Matrix {
        let mut a = Matrix {
            rows: vec![0],
            ..Default::default()
        };
        for view in 0..40 {
            for y in 0..n {
                for x in 0..n {
                    trace(
                        n,
                        1.,
                        geometry(),
                        view as f64 * std::f64::consts::TAU / 40.,
                        (x as f64 - (n - 1) as f64 / 2.) * 1.7,
                        (y as f64 - (n - 1) as f64 / 2.) * 1.7,
                        |i, l| {
                            a.indices.push(i as u32);
                            a.lengths.push(l);
                        },
                    );
                    a.rows.push(a.indices.len());
                    a.observed.push(0.);
                    a.weight.push(1.);
                    a.validation.push(view % 4 == 3);
                }
            }
        }
        a
    }
    #[test]
    fn central_ray_is_exact_in_mm_and_projector_is_an_adjoint_pair() {
        let mut length = 0.;
        trace(16, 0.5, geometry(), 0., 0., 0., |_, l| length += l);
        assert!((length - 8f32).abs() < 1e-5);
        let a = phantom_matrix(12);
        let x: Vec<_> = (0..12usize.pow(3))
            .map(|i| ((i * 17 % 101) as f32 - 50.) / 100.)
            .collect();
        let y: Vec<_> = (0..a.observed.len())
            .map(|i| ((i * 13 % 89) as f32 - 44.) / 100.)
            .collect();
        let left: f64 = a
            .forward(&x)
            .iter()
            .zip(&y)
            .map(|(a, b)| f64::from(*a) * f64::from(*b))
            .sum();
        let right: f64 = x
            .iter()
            .zip(a.adjoint(&y, x.len()))
            .map(|(a, b)| f64::from(*a) * f64::from(b))
            .sum();
        assert!(
            (left - right).abs() / left.abs().max(1.) < 1e-5,
            "{left}/{right}"
        );
    }
    #[test]
    fn off_axis_ray_uses_the_same_cone_geometry_as_fdk() {
        let n = 16;
        let g = geometry();
        for theta in [-0.7f64, 0., 1.2] {
            let (s, c) = theta.sin_cos();
            let (x, y, z) = (2.2, 3.2, 1.2);
            let m = g.sdd / (g.sod + y * c - x * s);
            let target = ((z + 8f64).floor() as usize * n + (y + 8f64).floor() as usize) * n
                + (x + 8f64).floor() as usize;
            let mut hit = false;
            trace(n, 1., g, theta, m * (x * c + y * s), m * z, |i, _| {
                hit |= i == target
            });
            assert!(hit, "theta={theta}");
        }
    }
    #[test]
    fn cone_beam_phantom_reduces_held_error_and_true_volume_error() {
        let n = 16;
        let mut a = phantom_matrix(n);
        let truth: Vec<f32> = (0..n * n * n)
            .map(|i| {
                let x = (i % n) as f32 - 7.5;
                let y = (i / n % n) as f32 - 7.5;
                let z = (i / n / n) as f32 - 7.5;
                if x * x / 25. + y * y / 16. + z * z / 36. < 1. {
                    0.08
                } else {
                    0.
                }
            })
            .collect();
        a.observed = a.forward(&truth);
        let initial: Vec<_> = truth
            .iter()
            .enumerate()
            .map(|(i, x)| x + ((i * 17 % 97) as f32 - 48.) * 0.0005 + 0.005)
            .collect();
        let before = a.errors(&initial);
        let (after, _) = optimize(&a, &initial, n, 16);
        let error = |v: &[f32]| {
            v.iter()
                .zip(&truth)
                .map(|(x, t)| (x - t).powi(2))
                .sum::<f32>()
        };
        println!(
            "phantom: validation MSE {} -> {}; true-volume SSE {} -> {}",
            before.1,
            a.errors(&after).1,
            error(&initial),
            error(&after)
        );
        assert!(a.errors(&after).1 < before.1 * 0.5);
        assert!(error(&after) < error(&initial) * 0.65);
        let inside: Vec<_> = after
            .iter()
            .zip(&truth)
            .filter(|(_, t)| **t > 0.)
            .map(|(x, _)| *x)
            .collect();
        assert!(
            inside.iter().sum::<f32>() / inside.len() as f32 > 0.07,
            "edge contrast lost"
        );
    }
    #[test]
    fn unsupported_input_keeps_original_signed_volume() {
        let mut v = vec![-0.02; 64usize.pow(3)];
        let old = v.clone();
        let report = refine(&mut v, 64, &[], &[], &[], geometry(), &[], &[], &[]);
        assert!(!report.accepted);
        assert_eq!(v, old);
    }
    #[test]
    fn trilinear_correction_preserves_constant_and_fdk_fine_detail() {
        let mut v: Vec<_> = (0..8usize.pow(3))
            .map(|i| if i % 2 == 0 { 0.1 } else { -0.1 })
            .collect();
        let old = v.clone();
        prolong_add(&mut v, 8, &vec![0.02; 4usize.pow(3)], 4);
        for (x, y) in v.iter().zip(old) {
            assert!((x - y - 0.02).abs() < 1e-6);
        }
    }

    #[test]
    fn analytic_3d_sphere_accepts_consistent_correction_and_rejects_conflicting_validation() {
        let g = geometry();
        let views = 32;
        let raw_n = super::super::DETECTOR_N;
        let angles: Vec<_> = (0..views)
            .map(|i| i as f64 * std::f64::consts::TAU / views as f64)
            .collect();
        let mut stack = vec![0f32; views * raw_n * raw_n];
        let flags = vec![1u8; stack.len()];
        let pitch = g.width * super::super::SCREEN_CROP_SCALE / raw_n as f64;
        let half = (raw_n - 1) as f64 / 2.;
        // Independent analytic chord integrals, not the reconstruction matrix.
        for (view, &theta) in angles.iter().enumerate() {
            let (s, c) = theta.sin_cos();
            let source = [g.sod * s, -g.sod * c, 0.];
            for y in 0..raw_n {
                for x in 0..raw_n {
                    let u = (x as f64 - half) * pitch;
                    let v = (y as f64 - half) * pitch;
                    let direction = [-g.sdd * s + u * c, g.sdd * c + u * s, v];
                    let length = direction.iter().map(|d| d * d).sum::<f64>().sqrt();
                    let dot = source
                        .iter()
                        .zip(direction)
                        .map(|(a, b)| a * b / length)
                        .sum::<f64>();
                    let distance2 = g.sod * g.sod - dot * dot;
                    stack[(view * raw_n + y) * raw_n + x] =
                        (0.16 * (25. - distance2).max(0.).sqrt()) as f32;
                }
            }
        }
        let n = 128;
        let voxel = g.width * g.sod / g.sdd / n as f64;
        let truth: Vec<f32> = (0..n * n * n)
            .map(|i| {
                let r2 = [i % n, i / n % n, i / n / n]
                    .iter()
                    .map(|&j| ((j as f64 + 0.5 - n as f64 / 2.) * voxel).powi(2))
                    .sum::<f64>();
                if r2 < 25. {
                    0.08
                } else {
                    0.
                }
            })
            .collect();
        let original: Vec<_> = truth.iter().map(|x| x + 0.005).collect();
        let mut output = original.clone();
        let gain = vec![1000.; raw_n * raw_n];
        let var = vec![1.; gain.len()];
        let report = refine(
            &mut output,
            n,
            &stack,
            &flags,
            &angles,
            g,
            &gain,
            &var,
            &var,
        );
        assert!(
            report.accepted,
            "{}; {} -> {}; correction {}",
            report.reason, report.validation_before, report.validation_after, report.max_correction
        );
        let error = |v: &[f32]| {
            v.iter()
                .zip(&truth)
                .map(|(x, t)| (x - t).powi(2))
                .sum::<f32>()
        };
        assert!(error(&output) < error(&original) * 0.5);
        for view in (3..views).step_by(4) {
            for value in &mut stack[view * raw_n * raw_n..(view + 1) * raw_n * raw_n] {
                *value += 1.;
            }
        }
        let mut rejected = original.clone();
        let report = refine(
            &mut rejected,
            n,
            &stack,
            &flags,
            &angles,
            g,
            &gain,
            &var,
            &var,
        );
        assert!(!report.accepted);
        assert_eq!(rejected, original);
        assert!(report.validation_after > report.validation_before);
    }
}
