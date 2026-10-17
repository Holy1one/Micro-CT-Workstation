//! Reference-driven, colour-independent detector defects. All thresholds operate
//! on registered linear measurements. Never classify specimen projections by
//! their brightness or colour. Estimates remain separate from measured rays.
use super::preprocessing::ReferenceStats;
use serde::Serialize;
use std::collections::VecDeque;

pub(super) const FLAT: u8 = 1;
pub(super) const SENSOR: u8 = 2;
pub(super) const PROTECTED: u8 = 4;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Report {
    pub algorithm: &'static str,
    pub reference_pixels: usize,
    pub sensor_pixels: usize,
    pub protected_pixels: usize,
    pub rejected_by_budget: bool,
    pub persistent_gain_pixels_per_band: [usize; 3],
    pub limitation: &'static str,
}

pub(super) struct Plan {
    pub mask: Vec<u8>,
    pub flat: [ReferenceStats; 3],
    pub report: Report,
    pub persistent_gain: [Vec<bool>; 3],
}

fn median(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.;
    }
    let mid = values.len() / 2;
    *values.select_nth_unstable_by(mid, f32::total_cmp).1
}

fn baseline(values: &[f32]) -> (f32, f32) {
    let m = median(&mut values.to_vec());
    let mad = median(&mut values.iter().map(|v| (v - m).abs()).collect::<Vec<_>>());
    (m, 1.4826 * mad)
}

fn inside(i: usize, n: usize, radius: f32) -> bool {
    let x = (i % n) as f32 + 0.5 - n as f32 / 2.;
    let y = (i / n) as f32 + 0.5 - n as f32 / 2.;
    x * x + y * y < (n as f32 * 0.5 * radius).powi(2)
}

fn neighbors(i: usize, n: usize) -> impl Iterator<Item = usize> {
    [
        i.checked_sub(n),
        (i + n < n * n).then_some(i + n),
        (i % n > 0).then(|| i - 1),
        (i % n + 1 < n).then_some(i + 1),
    ]
    .into_iter()
    .flatten()
}

fn components(mask: &[bool], n: usize) -> Vec<Vec<usize>> {
    let mut seen = vec![false; mask.len()];
    let mut result = Vec::new();
    for i in 0..mask.len() {
        if !mask[i] || seen[i] {
            continue;
        }
        let mut queue = VecDeque::from([i]);
        let mut part = Vec::new();
        seen[i] = true;
        while let Some(j) = queue.pop_front() {
            part.push(j);
            for k in neighbors(j, n) {
                if mask[k] && !seen[k] {
                    seen[k] = true;
                    queue.push_back(k);
                }
            }
        }
        result.push(part);
    }
    result
}

/// Fill only enclosed holes of an accepted component, not its bounding box.
fn fill_component(part: &[usize], n: usize, mask: &mut [bool]) {
    let x0 = part.iter().map(|i| i % n).min().unwrap();
    let x1 = part.iter().map(|i| i % n).max().unwrap();
    let y0 = part.iter().map(|i| i / n).min().unwrap();
    let y1 = part.iter().map(|i| i / n).max().unwrap();
    let mut wall = vec![false; n * n];
    for &i in part {
        wall[i] = true;
        mask[i] = true;
    }
    let mut exterior = vec![false; n * n];
    let mut queue = VecDeque::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = y * n + x;
            if (x == x0 || x == x1 || y == y0 || y == y1) && !wall[i] {
                exterior[i] = true;
                queue.push_back(i);
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        for j in neighbors(i, n) {
            if j % n >= x0 && j % n <= x1 && j / n >= y0 && j / n <= y1 && !wall[j] && !exterior[j]
            {
                exterior[j] = true;
                queue.push_back(j);
            }
        }
    }
    for y in y0..=y1 {
        for x in x0..=x1 {
            let i = y * n + x;
            if !exterior[i] {
                mask[i] = true;
            }
        }
    }
}

/// Discrete harmonic interpolation from the valid boundary, per connected
/// component. Unmasked samples are bitwise unchanged. No remote/global colour.
pub(super) fn interpolate(values: &mut [f32], mask: &[bool], valid: &[bool], n: usize) -> usize {
    interpolate_marked(values, mask, valid, n)
        .iter()
        .filter(|&&v| v)
        .count()
}

pub(super) fn interpolate_marked(
    values: &mut [f32],
    mask: &[bool],
    valid: &[bool],
    n: usize,
) -> Vec<bool> {
    let mut repaired = vec![false; mask.len()];
    for part in components(mask, n) {
        let mut sum = 0f64;
        let mut count = 0usize;
        for &i in &part {
            for j in neighbors(i, n) {
                if !mask[j] && valid[j] && values[j].is_finite() {
                    sum += f64::from(values[j]);
                    count += 1;
                }
            }
        }
        if count == 0 {
            continue;
        }
        let seed = (sum / count as f64) as f32;
        for &i in &part {
            values[i] = seed;
        }
        for _ in 0..600 {
            let mut error = 0f32;
            for &i in &part {
                let mut s = 0.;
                let mut w = 0.;
                for j in neighbors(i, n) {
                    if (mask[j] || valid[j]) && values[j].is_finite() {
                        s += values[j];
                        w += 1.;
                    }
                }
                if w > 0. {
                    let next = s / w;
                    error = error.max((next - values[i]).abs());
                    values[i] = next;
                }
            }
            if error < seed.abs().max(1.) * 1e-6 {
                break;
            }
        }
        for i in part {
            repaired[i] = true;
        }
    }
    repaired
}

pub(super) fn detect(dark: &[ReferenceStats; 3], flat: &[ReferenceStats; 3], n: usize) -> Plan {
    assert!(
        n >= 32
            && dark
                .iter()
                .chain(flat)
                .all(|s| s.mean.len() == n * n && s.variance.len() == n * n)
    );
    let gain: [Vec<f32>; 3] = std::array::from_fn(|b| {
        (0..n * n)
            .map(|i| (flat[b].mean[i] - dark[b].mean[i]).max(0.))
            .collect()
    });
    let finite: Vec<_> = (0..n * n)
        .map(|i| (0..3).all(|b| flat[b].mean[i].is_finite() && dark[b].mean[i].is_finite()))
        .collect();
    let intensity: Vec<f32> = (0..n * n)
        .map(|i| (0..3).map(|b| gain[b][i]).sum())
        .collect();
    let mut candidate = vec![false; n * n];
    let mut sensor = vec![false; n * n];
    let radii = [3usize, 10, 22].map(|r| (r * n / 256).max(1));
    for radius in radii {
        let r = radius as isize;
        let coords = [-r, -r / 2, 0, r / 2, r];
        let offsets: Vec<_> = coords
            .iter()
            .flat_map(|&y| coords.iter().map(move |&x| (x, y)))
            .filter(|(x, y)| x.abs() == r || y.abs() == r)
            .collect();
        for i in 0..n * n {
            if !inside(i, n, 0.92) {
                continue;
            }
            let local: Vec<_> = offsets
                .iter()
                .filter_map(|(dx, dy)| {
                    let x = (i % n) as isize + dx;
                    let y = (i / n) as isize + dy;
                    if x < 0 || y < 0 || x >= n as isize || y >= n as isize {
                        return None;
                    }
                    let j = y as usize * n + x as usize;
                    (inside(j, n, 1. / 1.01) && finite[j]).then_some(j)
                })
                .collect();
            if local.len() < 12 {
                continue;
            }
            let (m, mad) = baseline(&local.iter().map(|&j| intensity[j]).collect::<Vec<_>>());
            if m < 200. {
                continue;
            }
            if !finite[i] {
                candidate[i] = true;
                continue;
            }
            let brightness = (intensity[i] - m).abs() > (0.35 * m).max(8. * mad);
            let mut colour = false;
            for b in 0..3 {
                let (c, cmad) = baseline(
                    &local
                        .iter()
                        .map(|&j| gain[b][j] / intensity[j].max(1.))
                        .collect::<Vec<_>>(),
                );
                colour |= (gain[b][i] / intensity[i].max(1.) - c).abs() > 0.055f32.max(8. * cmad);
                let (d, dmad) =
                    baseline(&local.iter().map(|&j| dark[b].mean[j]).collect::<Vec<_>>());
                let noise = dark[b].variance[i].max(0.).sqrt();
                let hot = (dark[b].mean[i] - d).abs() > 20f32.max(8. * dmad).max(6. * noise);
                let (cv, cvmad) = baseline(
                    &local
                        .iter()
                        .map(|&j| flat[b].variance[j].max(0.) / gain[b][j].max(20.).powi(2))
                        .collect::<Vec<_>>(),
                );
                let unstable = flat[b].variance[i].max(0.) / gain[b][i].max(20.).powi(2)
                    > 0.0625f32.max(cv + 8. * cvmad);
                sensor[i] |= hot || unstable;
            }
            candidate[i] |= brightness || colour || sensor[i];
        }
    }
    let mut repair = vec![false; n * n];
    let mut protected = vec![false; n * n];
    for part in components(&candidate, n) {
        let width = part.iter().map(|i| i % n).max().unwrap()
            - part.iter().map(|i| i % n).min().unwrap()
            + 1;
        let height = part.iter().map(|i| i / n).max().unwrap()
            - part.iter().map(|i| i / n).min().unwrap()
            + 1;
        let boundary = part
            .iter()
            .any(|&i| neighbors(i, n).any(|j| !inside(j, n, 0.92)));
        if part.len() > n * n * 4 / 100 || width.max(height) > 6 * width.min(height) || boundary {
            for i in part {
                protected[i] = true;
            }
        } else {
            fill_component(&part, n, &mut repair);
        }
    }
    // Optical blemishes have a low-contrast halo beyond the high-confidence
    // core. A component-scaled collar moves the interpolation boundary away
    // from that halo; one-pixel dilation left a sharp artificial ring.
    for part in components(&repair, n) {
        let width = part.iter().map(|i| i % n).max().unwrap()
            - part.iter().map(|i| i % n).min().unwrap()
            + 1;
        let height = part.iter().map(|i| i / n).max().unwrap()
            - part.iter().map(|i| i / n).min().unwrap()
            + 1;
        let radius = width.max(height).div_ceil(5).clamp(1, (n * 3 / 100).max(1)) as isize;
        for i in part {
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    let x = (i % n) as isize + dx;
                    let y = (i / n) as isize + dy;
                    if x < 0 || y < 0 || x >= n as isize || y >= n as isize {
                        continue;
                    }
                    let j = y as usize * n + x as usize;
                    if inside(j, n, 0.92) && !protected[j] {
                        repair[j] = true;
                    }
                }
            }
        }
    }
    let rejected = repair.iter().filter(|&&x| x).count() > n * n / 10;
    let mask: Vec<u8> = (0..n * n)
        .map(|i| {
            if protected[i] {
                PROTECTED
            } else if repair[i] && !rejected {
                FLAT | if sensor[i] { SENSOR } else { 0 }
            } else {
                0
            }
        })
        .collect();
    let accepted: Vec<_> = mask.iter().map(|m| m & FLAT != 0).collect();
    let valid: Vec<_> = (0..n * n)
        .map(|i| inside(i, n, 1. / 1.01) && finite[i] && !accepted[i] && !protected[i])
        .collect();
    let corrected = std::array::from_fn(|b| {
        let mut g = gain[b].clone();
        interpolate(&mut g, &accepted, &valid, n);
        ReferenceStats {
            mean: (0..n * n)
                .map(|i| {
                    if accepted[i] {
                        dark[b].mean[i] + g[i]
                    } else {
                        flat[b].mean[i]
                    }
                })
                .collect(),
            variance: flat[b].variance.clone(),
        }
    });
    Plan { report: Report {
        algorithm: "local multiscale intensity/chromaticity MAD; dark outliers and relative temporal variance; connected-component protection; same-channel harmonic gain repair",
        reference_pixels: mask.iter().filter(|m| **m & FLAT != 0).count(),
        sensor_pixels: mask.iter().filter(|m| **m & SENSOR != 0).count(),
        protected_pixels: mask.iter().filter(|m| **m & PROTECTED != 0).count(),
        rejected_by_budget: rejected,
        persistent_gain_pixels_per_band: [0;3],
        limitation: "256 detector grid; conservative inner 92% radius; elongated/large/boundary regions protected. Reference-only evidence cannot uniquely identify compact fixtures or smoothly varying defects; no specimen-based defect detection.",
    }, mask, flat: corrected, persistent_gain: std::array::from_fn(|_|vec![false;n*n]) }
}

/// A stable multiplicative gain anomaly present in both references and views
/// is already corrected by measured dark/flat normalization. Replacing only
/// its flat would introduce an artifact. Probe only reference-selected sites;
/// agreement keeps the measured gain and never removes specimen structure.
pub(super) fn confirm_persistent_gain(
    plan: &mut Plan,
    dark: &[ReferenceStats; 3],
    original_flat: &[ReferenceStats; 3],
    probes: &[[Vec<f32>; 3]],
    n: usize,
) {
    let mask: Vec<_> = plan
        .mask
        .iter()
        .map(|m| m & FLAT != 0 && m & SENSOR == 0)
        .collect();
    let mut votes: [Vec<usize>; 3] = std::array::from_fn(|_| vec![0; n * n]);
    let mut counts = votes.clone();
    for raw in probes {
        for b in 0..3 {
            let signal: Vec<_> = raw[b]
                .iter()
                .zip(&dark[b].mean)
                .map(|(p, d)| p - d)
                .collect();
            let valid: Vec<_> = (0..n * n)
                .map(|i| {
                    !mask[i]
                        && plan.mask[i] & (FLAT|PROTECTED) == 0
                        && inside(i, n, 1. / 1.01)
                        && signal[i].is_finite()
                        && signal[i] > 1.
                })
                .collect();
            let mut background = signal.clone();
            let filled = interpolate_marked(&mut background, &mask, &valid, n);
            for i in 0..n * n {
                if !mask[i]
                    || !filled[i]
                    || !signal[i].is_finite()
                    || signal[i] <= 5.
                    || background[i] <= 5.
                {
                    continue;
                }
                let old = original_flat[b].mean[i] - dark[b].mean[i];
                let corrected = plan.flat[b].mean[i] - dark[b].mean[i];
                if !old.is_finite() || old <= 1. || corrected <= 1. {
                    continue;
                }
                let expected = (old / corrected).ln();
                if expected.abs() < 0.2 {
                    continue;
                }
                let observed = (signal[i] / background[i]).ln();
                counts[b][i] += 1;
                if expected * observed > 0.
                    && (observed - expected).abs() < 0.15f32.max(expected.abs() * 0.35)
                {
                    votes[b][i] += 1;
                }
            }
        }
    }
    for b in 0..3 {
        for i in 0..n * n {
            if counts[b][i] >= 8 && votes[b][i] * 5 >= counts[b][i] * 4 {
                plan.persistent_gain[b][i] = true;
                plan.flat[b].mean[i] = original_flat[b].mean[i];
            }
        }
        plan.report.persistent_gain_pixels_per_band[b] =
            plan.persistent_gain[b].iter().filter(|&&v| v).count();
    }
}

/// Interior invalid islands only. Preserve the original flag (3/4) separately;
/// these estimates must never become quantitative colour measurements.
pub(super) fn projection_mask(flags: &[u8], plan: &Plan, n: usize) -> Vec<bool> {
    let invalid: Vec<_> = flags
        .iter()
        .enumerate()
        .map(|(i, f)| inside(i, n, 0.90) && matches!(*f, 3 | 4))
        .collect();
    let mut mask: Vec<_> = plan.mask.iter().map(|m| m & SENSOR != 0).collect();
    for part in components(&invalid, n) {
        if part.len() <= 16
            && part.iter().all(|&i| {
                inside(i, n, 0.89)
                    && neighbors(i, n).all(|j| {
                        invalid[j]
                            || (matches!(flags[j], 1 | 5 | 6) && plan.mask[j] & PROTECTED == 0)
                    })
            })
        {
            for i in part {
                mask[i] = true;
            }
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;
    const N: usize = 128;
    fn fields(rgb: [f32; 3]) -> ([ReferenceStats; 3], [ReferenceStats; 3]) {
        (
            std::array::from_fn(|_| ReferenceStats {
                mean: vec![10.; N * N],
                variance: vec![1.; N * N],
            }),
            std::array::from_fn(|b| ReferenceStats {
                mean: vec![rgb[b] + 10.; N * N],
                variance: vec![4.; N * N],
            }),
        )
    }
    #[test]
    fn every_background_colour_and_smooth_vignetting_is_preserved() {
        for rgb in [
            [1200., 100., 60.],
            [100., 1200., 60.],
            [60., 100., 1200.],
            [800., 800., 50.],
        ] {
            let (d, mut f) = fields(rgb);
            for b in 0..3 {
                for i in 0..N * N {
                    let x = (i % N) as f32 / N as f32 - 0.5;
                    let y = (i / N) as f32 / N as f32 - 0.5;
                    f[b].mean[i] = 10. + rgb[b] * (1. - 0.6 * (x * x + y * y));
                }
            }
            let p = detect(&d, &f, N);
            assert_eq!(p.report.reference_pixels, 0, "{rgb:?}");
        }
    }
    #[test]
    fn white_coloured_bright_and_dark_blobs_are_repaired_on_any_background() {
        for rgb in [[1200., 100., 60.], [100., 1200., 60.], [60., 100., 1200.]] {
            for bad in [[2000., 2000., 2000.], [100., 100., 2400.], [0., 0., 0.]] {
                let (d, mut f) = fields(rgb);
                for y in 60..67 {
                    for x in 60..67 {
                        for b in 0..3 {
                            f[b].mean[y * N + x] = 10. + bad[b];
                        }
                    }
                }
                let p = detect(&d, &f, N);
                assert!(p.mask[63 * N + 63] & FLAT != 0, "{rgb:?}/{bad:?}");
                for b in 0..3 {
                    assert!((p.flat[b].mean[63 * N + 63] - (10. + rgb[b])).abs() < 0.1);
                }
                assert_eq!(p.flat[0].mean[50 * N + 50], f[0].mean[50 * N + 50]);
            }
        }
    }
    #[test]
    fn genuine_colour_boundary_and_long_support_are_protected() {
        let (d, mut f) = fields([100., 1200., 60.]);
        for y in 0..N {
            for x in N / 2..N {
                f[0].mean[y * N + x] = 1210.;
                f[1].mean[y * N + x] = 110.;
            }
        }
        assert_eq!(detect(&d, &f, N).report.reference_pixels, 0);
        let (d, mut f) = fields([100., 1200., 60.]);
        for y in 45..48 {
            for x in 25..103 {
                for b in 0..3 {
                    f[b].mean[y * N + x] = 20.;
                }
            }
        }
        let p = detect(&d, &f, N);
        assert_eq!(p.report.reference_pixels, 0);
        assert!(p.report.protected_pixels > 0);
    }
    #[test]
    fn temporal_instability_and_dark_hot_pixels_are_sensor_defects() {
        let (mut d, mut f) = fields([100., 1200., 60.]);
        d[1].mean[50 * N + 50] = 180.;
        f[1].variance[75 * N + 75] = 1e6;
        let p = detect(&d, &f, N);
        assert!(p.mask[50 * N + 50] & SENSOR != 0);
        assert!(p.mask[75 * N + 75] & SENSOR != 0);
    }
    #[test]
    fn interpolation_without_valid_boundary_does_not_invent_measurements() {
        let mut a = vec![f32::NAN; 32 * 32];
        assert_eq!(
            interpolate(&mut a, &vec![true; 32 * 32], &vec![false; 32 * 32], 32),
            0
        );
        assert!(a.iter().all(|x| x.is_nan()));
    }

    #[test]
    fn smooth_coloured_defect_halo_is_included_before_interpolation() {
        let (d, mut f) = fields([100., 1200., 60.]);
        for i in 0..N * N {
            let r2 = ((i % N) as f32 - 64.).powi(2) + ((i / N) as f32 - 64.).powi(2);
            for (b, a) in [50., 1400., 600.].iter().enumerate() {
                f[b].mean[i] += a * (-r2 / 18.).exp();
            }
        }
        let p = detect(&d, &f, N);
        assert!(
            (p.flat[1].mean[64 * N + 64] - 1210.).abs() < 36.,
            "halo boundary is not clean: {}",
            p.flat[1].mean[64 * N + 64]
        );
    }
    #[test]
    fn persistent_gain_is_normalized_while_flat_only_contamination_is_repaired() {
        for factor in [0.1, 3.] {
            let (d, mut f) = fields([200., 1200., 200.]);
            let bad = 64 * N + 64;
            for b in 0..3 {
                f[b].mean[bad] = 10. + (f[b].mean[bad] - 10.) * factor;
            }
            for persistent in [true, false] {
                let mut plan = detect(&d, &f, N);
                let probes: Vec<[Vec<f32>; 3]> = (0..12)
                    .map(|_| {
                        std::array::from_fn(|b| {
                            let mut v = vec![10. + [200., 1200., 200.][b] * 0.5; N * N];
                            if persistent {
                                v[bad] = 10. + (v[bad] - 10.) * factor;
                            }
                            v
                        })
                    })
                    .collect();
                confirm_persistent_gain(&mut plan, &d, &f, &probes, N);
                assert_eq!(plan.persistent_gain[1][bad], persistent);
                let expected = if persistent { f[1].mean[bad] } else { 1210. };
                assert!((plan.flat[1].mean[bad] - expected).abs() < 0.1);
            }
        }
    }
}
