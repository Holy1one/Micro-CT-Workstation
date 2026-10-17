//! Measured two-basis lookup inversion. Optical mixing is included in the
//! measured camera-band response; no invented scintillator composition.
//! Basis materials are calibration coordinates. Specimen identity and thickness
//! are inferred outputs, never required calibration inputs.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ValidationPoint {
    pub thickness_mm: [f64; 2],
    pub log_attenuation: [f64; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Calibration {
    pub schema_version: u32,
    pub measurement: String,
    pub capture_profile_hash: String,
    pub voltage_kv: f64,
    pub layers: Vec<super::spectral::StackLayer>,
    pub basis_names: [String; 2],
    pub thickness_grid_mm: [Vec<f64>; 2],
    // First basis index varies fastest. Response includes assembled optics,
    // camera CFA cross-talk and layer filtering, using linear green/blue data.
    pub log_attenuation: Vec<[f64; 2]>,
    pub validation_points: Vec<ValidationPoint>,
    pub source_description: String,
    /// Maps common green detector coordinates to measured blue coordinates.
    pub blue_to_green_affine: [[f64; 3]; 2],
    pub registration_rms_pixels: f64,
    pub geometry_mm: [f64; 3],
    pub mirror_x: bool,
    pub rotation_direction: String,
    pub center_offset_mm: [f64; 2],
    pub detector_shape: [usize; 2],
    /// Optional broad candidate library in these calibrated basis coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_library: Option<BasisLibrary>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisCandidate {
    pub name: String,
    /// Equivalent basis volume fractions at the library's reference density.
    pub basis_signature: [f64; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BasisLibrary {
    pub basis_names: [String; 2],
    pub candidates: Vec<BasisCandidate>,
    /// Covariance from independent basis reconstruction validation, not RGB noise.
    pub covariance: [[f64; 2]; 2],
    pub maximum_squared_error: f64,
    pub ambiguity_squared_error_gap: f64,
    pub source_description: String,
}

pub struct PreparedLibrary<'a> {
    library: &'a BasisLibrary,
    precision: [[f64; 2]; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct CandidateMatch {
    /// 0 unknown, 65535 ambiguous, otherwise 1-based candidate index.
    pub label: u16,
    pub best_candidate: u16,
    pub density_scale: f64,
    pub squared_error: f64,
}

impl BasisLibrary {
    pub fn prepare(&self, basis_names: &[String; 2]) -> Result<PreparedLibrary<'_>, String> {
        let c = self.covariance;
        let det = c[0][0] * c[1][1] - c[0][1] * c[1][0];
        if &self.basis_names != basis_names
            || self.candidates.len() < 2
            || self.candidates.len() > 200
            || self.source_description.trim().is_empty()
            || c.iter().flatten().any(|v| !v.is_finite())
            || (c[0][1] - c[1][0]).abs() > 1e-12
            || c[0][0] <= 0.
            || c[1][1] <= 0.
            || det <= 0.
            || !self.maximum_squared_error.is_finite()
            || self.maximum_squared_error <= 0.
            || !self.ambiguity_squared_error_gap.is_finite()
            || self.ambiguity_squared_error_gap <= 0.
        {
            return Err(
                "invalid material library basis, covariance or acceptance thresholds".into(),
            );
        }
        let mut names = std::collections::HashSet::new();
        for candidate in &self.candidates {
            if candidate.name.trim().is_empty()
                || !names.insert(&candidate.name)
                || candidate
                    .basis_signature
                    .iter()
                    .any(|v| !v.is_finite() || *v < 0.)
                || candidate.basis_signature.iter().all(|v| *v == 0.)
            {
                return Err("invalid or duplicate material library signature".into());
            }
        }
        Ok(PreparedLibrary {
            library: self,
            precision: [
                [c[1][1] / det, -c[0][1] / det],
                [-c[1][0] / det, c[0][0] / det],
            ],
        })
    }
}

impl PreparedLibrary<'_> {
    pub fn classify(&self, observed: [f64; 2]) -> CandidateMatch {
        let unknown = CandidateMatch {
            label: 0,
            best_candidate: 0,
            density_scale: 0.,
            squared_error: f64::NAN,
        };
        if observed.iter().any(|v| !v.is_finite() || *v < 0.) || observed == [0., 0.] {
            return unknown;
        }
        let product = |a: [f64; 2], b: [f64; 2]| -> f64 {
            a[0] * (self.precision[0][0] * b[0] + self.precision[0][1] * b[1])
                + a[1] * (self.precision[1][0] * b[0] + self.precision[1][1] * b[1])
        };
        let mut best = unknown;
        best.squared_error = f64::INFINITY;
        let mut second = f64::INFINITY;
        for (index, candidate) in self.library.candidates.iter().enumerate() {
            let signature = candidate.basis_signature;
            // Unknown amplitude (path length for rays; density scale for voxels)
            // is profiled out independently for every candidate.
            let denominator = product(signature, signature);
            let numerator = product(signature, observed);
            if !denominator.is_finite() || denominator <= 0. || !numerator.is_finite() {
                continue;
            }
            let scale = (numerator / denominator).max(0.);
            let difference = [
                observed[0] - scale * signature[0],
                observed[1] - scale * signature[1],
            ];
            let error = product(difference, difference);
            if !scale.is_finite() || !error.is_finite() {
                continue;
            }
            let error = error.max(0.);
            if error < best.squared_error {
                second = best.squared_error;
                best = CandidateMatch {
                    label: (index + 1) as u16,
                    best_candidate: (index + 1) as u16,
                    density_scale: scale,
                    squared_error: error,
                };
            } else {
                second = second.min(error);
            }
        }
        if best.squared_error > self.library.maximum_squared_error {
            best.label = 0;
        } else if second - best.squared_error < self.library.ambiguity_squared_error_gap {
            best.label = u16::MAX;
        }
        best
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Estimate {
    pub thickness_mm: [f64; 2],
    pub residual: f64,
    pub condition: f64,
}

impl Calibration {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(library) = &self.material_library {
            library.prepare(&self.basis_names)?;
        }
        if self.schema_version != 1
            || !matches!(self.measurement.as_str(), "measured" | "synthetic")
            || self.basis_names.iter().any(|s| s.trim().is_empty())
            || self.basis_names[0] == self.basis_names[1]
            || !self.voltage_kv.is_finite()
            || self.voltage_kv <= 0.
            || self.source_description.trim().is_empty()
        {
            return Err("invalid material calibration identity".into());
        }
        if self
            .blue_to_green_affine
            .iter()
            .flatten()
            .any(|v| !v.is_finite())
            || !self.registration_rms_pixels.is_finite()
            || self.registration_rms_pixels < 0.
            || self.registration_rms_pixels > 0.5
            || self.geometry_mm.iter().any(|v| !v.is_finite() || *v <= 0.)
            || self.center_offset_mm.iter().any(|v| !v.is_finite())
            || !matches!(
                self.rotation_direction.as_str(),
                "clockwise" | "counterclockwise"
            )
            || self.detector_shape != [DETECTOR_N, DETECTOR_N]
        {
            return Err(
                "material calibration needs measured subpixel registration and matching geometry"
                    .into(),
            );
        }
        let a = self.blue_to_green_affine;
        if (a[0][0] * a[1][1] - a[0][1] * a[1][0]).abs() < 0.5 {
            return Err("material channel registration is singular or excessively scaled".into());
        }
        super::spectral::validate_layers(&self.layers)?;
        for grid in &self.thickness_grid_mm {
            if grid.len() < 2
                || grid.len() > 32
                || grid[0] != 0.
                || grid.iter().any(|v| !v.is_finite() || *v < 0.)
                || grid.windows(2).any(|v| v[1] <= v[0])
            {
                return Err(
                    "calibration requires increasing nonnegative thickness grids starting at zero"
                        .into(),
                );
            }
        }
        let nx = self.thickness_grid_mm[0].len();
        let ny = self.thickness_grid_mm[1].len();
        if self.log_attenuation.len() != nx * ny
            || self
                .log_attenuation
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || *v < 0. || *v > 30.)
            || self.log_attenuation[0].iter().any(|v| v.abs() > 1e-6)
        {
            return Err("invalid measured calibration response grid".into());
        }
        let mut determinant_sign = 0.;
        for y in 0..ny - 1 {
            for x in 0..nx - 1 {
                for fraction in [[0., 0.], [0., 1.], [1., 0.], [1., 1.], [0.5, 0.5]] {
                    let position = [
                        self.thickness_grid_mm[0][x]
                            + fraction[0]
                                * (self.thickness_grid_mm[0][x + 1] - self.thickness_grid_mm[0][x]),
                        self.thickness_grid_mm[1][y]
                            + fraction[1]
                                * (self.thickness_grid_mm[1][y + 1] - self.thickness_grid_mm[1][y]),
                    ];
                    let (_, j) = self.forward(position);
                    let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
                    if j.iter().flatten().any(|v| !v.is_finite() || *v < 0.)
                        || condition(j) > 100.
                        || det.abs() < 1e-10
                    {
                        return Err(
                            "material response is not sufficiently independent or monotonic".into(),
                        );
                    }
                    if determinant_sign == 0. {
                        determinant_sign = det.signum();
                    }
                    if det.signum() != determinant_sign {
                        return Err("material response grid folds".into());
                    }
                }
            }
        }
        if self.validation_points.len() < 3 {
            return Err(
                "at least three independent measured validation points are required".into(),
            );
        }
        for (index, point) in self.validation_points.iter().enumerate() {
            if (self.thickness_grid_mm[0].contains(&point.thickness_mm[0])
                && self.thickness_grid_mm[1].contains(&point.thickness_mm[1]))
                || self.validation_points[..index]
                    .iter()
                    .any(|previous| previous.thickness_mm == point.thickness_mm)
            {
                return Err(
                    "validation points must be distinct mixtures outside the training grid nodes"
                        .into(),
                );
            }
            if point.thickness_mm.iter().enumerate().any(|(i, v)| {
                !v.is_finite() || *v < 0. || *v > *self.thickness_grid_mm[i].last().unwrap()
            }) || point
                .log_attenuation
                .iter()
                .any(|v| !v.is_finite() || *v < 0.)
            {
                return Err("invalid held-out calibration measurement".into());
            }
            let estimate = self
                .invert(point.log_attenuation)
                .ok_or("held-out calibration response is outside the invertible domain")?;
            for (i, grid) in self.thickness_grid_mm.iter().enumerate() {
                if (estimate.thickness_mm[i] - point.thickness_mm[i]).abs()
                    > grid.last().unwrap() * 0.05
                {
                    return Err(
                        "held-out basis thickness error exceeds 5% of the calibrated range".into(),
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn validate_scan(
        &self,
        manifest: &Manifest,
        layers: &[super::spectral::StackLayer],
    ) -> Result<(), String> {
        self.validate()?;
        if self.measurement != "measured" {
            return Err("synthetic calibration cannot label real scan materials".into());
        }
        let profile = manifest
            .capture_profile
            .as_ref()
            .ok_or("material calibration requires a frozen acquisition profile")?;
        if self.capture_profile_hash != profile.parameters_hash
            || (self.voltage_kv - manifest.voltage_kv).abs() > 0.1
            || self.layers != layers
            || manifest.geometry.measurement != "measured"
            || self.mirror_x != manifest.geometry.mirror_x
            || self.rotation_direction != manifest.geometry.rotation_direction
            || self
                .center_offset_mm
                .iter()
                .zip([
                    manifest.geometry.center_offset_x_mm,
                    manifest.geometry.center_offset_y_mm,
                ])
                .any(|(a, b)| (a - b).abs() > 0.01)
            || self
                .geometry_mm
                .iter()
                .zip([
                    manifest.geometry.sod_mm,
                    manifest.geometry.object_to_detector_mm,
                    manifest.geometry.detector_width_mm,
                ])
                .any(|(a, b)| (a - b).abs() > 0.01)
        {
            return Err(
                "material calibration does not match the acquisition profile or scintillator stack"
                    .into(),
            );
        }
        Ok(())
    }

    fn forward(&self, position: [f64; 2]) -> ([f64; 2], [[f64; 2]; 2]) {
        let mut cell = [0; 2];
        let mut fraction = [0.; 2];
        let mut spacing = [0.; 2];
        for i in 0..2 {
            let g = &self.thickness_grid_mm[i];
            cell[i] = g
                .partition_point(|v| *v <= position[i])
                .saturating_sub(1)
                .min(g.len() - 2);
            spacing[i] = g[cell[i] + 1] - g[cell[i]];
            fraction[i] = ((position[i] - g[cell[i]]) / spacing[i]).clamp(0., 1.);
        }
        let nx = self.thickness_grid_mm[0].len();
        let idx = cell[1] * nx + cell[0];
        let (x, y) = (fraction[0], fraction[1]);
        let a = self.log_attenuation[idx];
        let b = self.log_attenuation[idx + 1];
        let c = self.log_attenuation[idx + nx];
        let d = self.log_attenuation[idx + nx + 1];
        let mut value = [0.; 2];
        let mut j = [[0.; 2]; 2];
        for band in 0..2 {
            value[band] = a[band] * (1. - x) * (1. - y)
                + b[band] * x * (1. - y)
                + c[band] * (1. - x) * y
                + d[band] * x * y;
            j[band][0] = ((b[band] - a[band]) * (1. - y) + (d[band] - c[band]) * y) / spacing[0];
            j[band][1] = ((c[band] - a[band]) * (1. - x) + (d[band] - b[band]) * x) / spacing[1];
        }
        (value, j)
    }

    pub fn invert(&self, observed: [f64; 2]) -> Option<Estimate> {
        if observed.iter().any(|v| !v.is_finite() || *v < 0.) {
            return None;
        }
        let nearest = self
            .log_attenuation
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let distance =
                    |v: &[f64; 2]| (v[0] - observed[0]).powi(2) + (v[1] - observed[1]).powi(2);
                distance(a.1).total_cmp(&distance(b.1))
            })?
            .0;
        let nx = self.thickness_grid_mm[0].len();
        let mut position = [
            self.thickness_grid_mm[0][nearest % nx],
            self.thickness_grid_mm[1][nearest / nx],
        ];
        for _ in 0..24 {
            let (value, j) = self.forward(position);
            let residual = [observed[0] - value[0], observed[1] - value[1]];
            let error = residual[0].hypot(residual[1]);
            let cond = condition(j);
            if cond > 100. {
                return None;
            }
            if error < 1e-5 {
                return Some(Estimate {
                    thickness_mm: position,
                    residual: error,
                    condition: cond,
                });
            }
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-10 {
                return None;
            }
            let delta = [
                (residual[0] * j[1][1] - residual[1] * j[0][1]) / det,
                (j[0][0] * residual[1] - j[1][0] * residual[0]) / det,
            ];
            let mut accepted = false;
            for scale in [1., 0.5, 0.25, 0.125, 0.0625] {
                let next = std::array::from_fn(|i| {
                    (position[i] + scale * delta[i])
                        .clamp(0., *self.thickness_grid_mm[i].last().unwrap())
                });
                let (predicted, _) = self.forward(next);
                if (predicted[0] - observed[0]).hypot(predicted[1] - observed[1]) < error {
                    position = next;
                    accepted = true;
                    break;
                }
            }
            if !accepted {
                return None;
            }
        }
        None
    }
}

fn condition(j: [[f64; 2]; 2]) -> f64 {
    let trace = j.iter().flatten().map(|v| v * v).sum::<f64>();
    let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
    let discriminant = (trace * trace - 4. * det * det).max(0.).sqrt();
    let small = (trace - discriminant) * 0.5;
    let large = (trace + discriminant) * 0.5;
    if small <= 0. {
        f64::INFINITY
    } else {
        (large / small).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn phantom() -> Calibration {
        let response = |a: f64, b: f64| [0.3 * a + 0.8 * b, 0.2 * a + 0.25 * b];
        Calibration {
            schema_version: 1,
            measurement: "synthetic".into(),
            capture_profile_hash: "test".into(),
            voltage_kv: 60.,
            layers: vec![],
            basis_names: ["polymer".into(), "metal".into()],
            thickness_grid_mm: [vec![0., 1., 2.], vec![0., 1., 2.]],
            log_attenuation: (0..3)
                .flat_map(|b| (0..3).map(move |a| response(a as f64, b as f64)))
                .collect(),
            validation_points: [[0.3, 0.7], [1.2, 0.1], [0.2, 1.8]]
                .map(|t| ValidationPoint {
                    thickness_mm: t,
                    log_attenuation: response(t[0], t[1]),
                })
                .to_vec(),
            source_description: "analytical synthetic two-basis test, not hardware evidence".into(),
            blue_to_green_affine: [[1., 0., 0.], [0., 1., 0.]],
            registration_rms_pixels: 0.,
            geometry_mm: [40., 25., 40.],
            mirror_x: false,
            rotation_direction: "counterclockwise".into(),
            center_offset_mm: [0., 0.],
            detector_shape: [DETECTOR_N, DETECTOR_N],
            material_library: None,
        }
    }
    #[test]
    fn measured_grid_inverts_independent_mixture_and_rejects_extrapolation() {
        let c = phantom();
        c.validate().unwrap();
        let e = c
            .invert([0.3 * 0.63 + 0.8 * 1.37, 0.2 * 0.63 + 0.25 * 1.37])
            .unwrap();
        assert!((e.thickness_mm[0] - 0.63).abs() < 1e-6);
        assert!((e.thickness_mm[1] - 1.37).abs() < 1e-6);
        assert!(c.invert([20., 20.]).is_none());
    }
    #[test]
    fn similar_spectral_responses_and_false_validation_are_rejected() {
        let mut c = phantom();
        for v in &mut c.log_attenuation {
            v[1] = v[0];
        }
        assert!(c.validate().is_err());
        let mut c = phantom();
        c.validation_points[0].thickness_mm = [1.97, 1.97];
        assert!(c
            .validate()
            .unwrap_err()
            .contains("thickness error exceeds"));
        let mut c = phantom();
        c.validation_points[0] = ValidationPoint {
            thickness_mm: [2.05, 0.7],
            log_attenuation: [0.3 * 2. + 0.8 * 0.7, 0.2 * 2. + 0.25 * 0.7],
        };
        assert!(c.validate().unwrap_err().contains("invalid held-out"));
    }
    #[test]
    fn real_material_labels_require_measured_matching_profile_stack_and_geometry() {
        let profile = crate::capture_profile::test_profile();
        let manifest = Manifest {
            schema_version: 3,
            voltage_kv: 60.,
            capture_profile: Some(profile.clone()),
            stage: None,
            max_xray_sec: None,
            capture_destination: None,
            acquisition_status: None,
            data_status: None,
            capture_records: vec![],
            completed: true,
            projection_count: 0,
            frames: vec![],
            references: References {
                pre_dark: vec![],
                pre_flat: vec![],
            },
            geometry: Geometry {
                sod_mm: 40.,
                object_to_detector_mm: 25.,
                detector_width_mm: 40.,
                center_offset_x_mm: 0.,
                center_offset_y_mm: 0.,
                rotation_direction: "counterclockwise".into(),
                mirror_x: false,
                measurement: "measured".into(),
                scintillator_layers: vec![],
            },
        };
        let mut calibration = phantom();
        calibration.capture_profile_hash = profile.parameters_hash;
        assert!(calibration
            .validate_scan(&manifest, &[])
            .unwrap_err()
            .contains("synthetic"));
        calibration.measurement = "measured".into();
        calibration.validate_scan(&manifest, &[]).unwrap();
        calibration.mirror_x = true;
        assert!(calibration.validate_scan(&manifest, &[]).is_err());
        calibration.mirror_x = false;
        calibration.geometry_mm[0] += 1.;
        assert!(calibration.validate_scan(&manifest, &[]).is_err());
        calibration.geometry_mm[0] -= 1.;
        calibration.capture_profile_hash = "another profile".into();
        assert!(calibration.validate_scan(&manifest, &[]).is_err());
        calibration.registration_rms_pixels = 0.6;
        assert!(calibration.validate().is_err());
    }
    #[test]
    fn unknown_material_matching_profiles_amplitude_and_preserves_ambiguity() {
        let mut library = BasisLibrary {
            basis_names: ["Al".into(), "Cu".into()],
            candidates: vec![
                BasisCandidate {
                    name: "candidate A".into(),
                    basis_signature: [1., 0.2],
                },
                BasisCandidate {
                    name: "candidate B".into(),
                    basis_signature: [0.1, 1.],
                },
            ],
            covariance: [[0.0001, 0.], [0., 0.0001]],
            maximum_squared_error: 9.,
            ambiguity_squared_error_gap: 1.,
            source_description: "synthetic independent signature test".into(),
        };
        let prepared = library.prepare(&library.basis_names).unwrap();
        for scale in [0.07, 0.63, 2.8] {
            let estimate = prepared.classify([0.1 * scale, scale]);
            assert_eq!(estimate.label, 2);
            assert!((estimate.density_scale - scale).abs() < 1e-12);
        }
        assert_eq!(prepared.classify([0., 0.]).label, 0);
        assert_eq!(prepared.classify([-0.1, 1.]).label, 0);
        assert_eq!(prepared.classify([1., 1.]).label, 0);
        library.candidates.push(BasisCandidate {
            name: "indistinguishable B".into(),
            basis_signature: [0.2, 2.],
        });
        assert_eq!(
            library
                .prepare(&library.basis_names)
                .unwrap()
                .classify([0.1, 1.])
                .label,
            u16::MAX
        );
        library.covariance[1][0] = 1.;
        assert!(library.prepare(&library.basis_names).is_err());
        library.covariance = [[0.0001, 0.], [0., 0.0001]];
        for candidate in &mut library.candidates {
            candidate.basis_signature = [1e-300, 1e-300];
        }
        assert_eq!(
            library
                .prepare(&library.basis_names)
                .unwrap()
                .classify([0.1, 1.])
                .label,
            0
        );
    }
}
