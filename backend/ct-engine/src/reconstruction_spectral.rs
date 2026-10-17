//! Versioned offline colour-channel reconstruction. Old caches and NEFs stay
//! untouched. Material names require measured, matched two-basis calibration.
pub use super::channels::Band;
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StackLayer {
    pub material_name: String,
    pub emission_color: String,
    pub thickness_mm: f64,
}

pub fn validate_layers(layers: &[StackLayer]) -> Result<(), String> {
    if layers.len() > 5 {
        return Err("at most five scintillator layers may be configured".into());
    }
    for layer in layers {
        if layer.material_name.trim().is_empty()
            || layer.material_name.len() > 128
            || !matches!(
                layer.emission_color.as_str(),
                "red" | "green" | "blue" | "yellow" | "other"
            )
            || !layer.thickness_mm.is_finite()
            || layer.thickness_mm <= 0.
            || layer.thickness_mm > 100.
        {
            return Err("each configured layer needs material, emission colour and a positive thickness <=100 mm".into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Monochrome,
    Multicolor,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub band: Option<Band>,
    #[serde(default)]
    pub layers: Vec<StackLayer>,
    #[serde(default)]
    pub material_calibration_path: Option<PathBuf>,
}
impl Options {
    pub fn validate(&self) -> Result<(), String> {
        validate_layers(&self.layers)?;
        if self.mode == Mode::Multicolor && self.band.is_some() {
            return Err("multicolor mode selects available bands together".into());
        }
        if self.material_calibration_path.is_some() && self.mode != Mode::Multicolor {
            return Err("material decomposition requires multicolor mode".into());
        }
        Ok(())
    }
}

const ALGORITHM_VERSION: u32 = 3;

fn effective_options(scan: &Path, options: &Options) -> Result<Options, String> {
    let (manifest, _) = read_manifest(scan)?;
    let mut effective = options.clone();
    if effective.layers.is_empty() {
        effective.layers = manifest.geometry.scintillator_layers.clone();
    }
    if effective.layers.is_empty() {
        let sidecar = scan.join("reconstruction").join("detector-config.json");
        if sidecar.is_file() {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct DetectorConfiguration {
                manifest_sha256: String,
                layers: Vec<StackLayer>,
            }
            let (bytes, _) = hash_file(&sidecar)?;
            if bytes > 16384 {
                return Err("detector configuration exceeds size limit".into());
            }
            let config: DetectorConfiguration =
                serde_json::from_reader(File::open(sidecar).map_err(io_error)?)
                    .map_err(|e| e.to_string())?;
            if config.manifest_sha256 != hash_file(&scan.join("manifest.json"))?.1 {
                return Err(
                    "offline detector configuration does not match this sealed scan".into(),
                );
            }
            effective.layers = config.layers;
        }
    }
    if !manifest.geometry.scintillator_layers.is_empty()
        && effective.layers != manifest.geometry.scintillator_layers
    {
        return Err("reconstruction layers differ from the frozen scan stack".into());
    }
    effective.validate()?;
    if effective.mode == Mode::Multicolor && effective.layers.len() < 2 {
        return Err("multicolor reconstruction needs the actual stack configuration".into());
    }
    Ok(effective)
}

fn input_identity(
    scan: &Path,
    method: ReconstructionMethod,
    options: &Options,
) -> Result<(PathBuf, String, Option<material::Calibration>), String> {
    let mut identity = serde_json::json!({"algorithmVersion":ALGORITHM_VERSION,"method":method,
        "manifestSha256":hash_file(&scan.join("manifest.json"))?.1,"options":options});
    let calibration = if let Some(path) = &options.material_calibration_path {
        if fs::metadata(path).map_err(io_error)?.len() > 2 * 1024 * 1024 {
            return Err("material calibration exceeds 2 MiB".into());
        }
        let bytes = fs::read(path).map_err(io_error)?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("material calibration changed beyond its size limit".into());
        }
        let hash = format!("{:x}", Sha256::digest(&bytes));
        identity["calibrationSha256"] = serde_json::json!(hash);
        let calibration: material::Calibration = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid material calibration: {e}"))?;
        let (manifest, _) = read_manifest(scan)?;
        calibration.validate_scan(&manifest, &options.layers)?;
        Some(calibration)
    } else {
        None
    };
    let hash = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&identity).map_err(|e| e.to_string())?)
    );
    Ok((
        scan.join("reconstruction")
            .join("spectral")
            .join(format!("v{ALGORITHM_VERSION}-{}", &hash[..20])),
        hash,
        calibration,
    ))
}

pub fn cached_result(
    scan: &Path,
    method: ReconstructionMethod,
    options: &Options,
) -> Result<Option<ReconstructionResult>, String> {
    let options = effective_options(scan, options)?;
    let (root, identity, _) = input_identity(scan, method, &options)?;
    cached_result_root(scan,method,&root,&identity)
}

fn cached_result_root(scan:&Path,method:ReconstructionMethod,root:&Path,identity:&str)->Result<Option<ReconstructionResult>,String> {
    let Some(result) = cached_result_at(scan, method, &root)? else {
        return Ok(None);
    };
    let report: serde_json::Value =
        serde_json::from_reader(File::open(root.join("spectral.json")).map_err(io_error)?)
            .map_err(|e| e.to_string())?;
    if report["identity"].as_str() != Some(identity) {
        return Ok(None);
    }
    for entry in report["artifacts"]
        .as_array()
        .ok_or("spectral artifact manifest missing")?
    {
        let relative = entry["path"]
            .as_str()
            .ok_or("spectral artifact path missing")?;
        let path = root.join(relative);
        if !path.is_file() {
            return Ok(None);
        }
        let canonical = path.canonicalize().map_err(io_error)?;
        if !canonical.starts_with(root.canonicalize().map_err(io_error)?) {
            return Err("spectral artifact leaves result directory".into());
        }
        let expected = entry["sha256"]
            .as_str()
            .ok_or("spectral artifact hash missing")?;
        if hash_file(&path)?.1 != expected {
            return Ok(None);
        }
    }
    Ok(Some(result))
}

fn observation_bands(layers: &[StackLayer], calibrated: bool) -> Vec<Band> {
    if calibrated {
        return vec![Band::Green, Band::Blue];
    }
    let mut bands = Vec::new();
    for layer in layers {
        let candidates: &[Band] = match layer.emission_color.as_str() {
            "red" => &[Band::Red],
            "green" => &[Band::Green],
            "blue" => &[Band::Blue],
            "yellow" => &[Band::Red, Band::Green],
            _ => &Band::ALL,
        };
        for band in candidates {
            if !bands.contains(band) {
                bands.push(*band);
            }
        }
    }
    bands
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BandScore {
    band: Band,
    reference_snr: f64,
    usable_fraction: f64,
    score: f64,
}

fn band_score(
    band: Band,
    dark: &preprocessing::ReferenceStats,
    flat: &preprocessing::ReferenceStats,
) -> BandScore {
    let mut snrs = Vec::new();
    let mut count = 0usize;
    let n = DETECTOR_N;
    for i in 0..n * n {
        let x = (i % n) as f64 - (n - 1) as f64 * 0.5;
        let y = (i / n) as f64 - (n - 1) as f64 * 0.5;
        if x.hypot(y) > n as f64 * 0.4 {
            continue;
        }
        count += 1;
        let gain = flat.mean[i] - dark.mean[i];
        let noise = (dark.variance[i] * (1. + 1. / REFERENCE_COUNT as f32)
            + flat.variance[i] / REFERENCE_COUNT as f32)
            .max(1.)
            .sqrt();
        if gain.is_finite() && gain > noise * 5. {
            snrs.push(f64::from(gain / noise));
        }
    }
    snrs.sort_by(f64::total_cmp);
    let reference_snr = snrs.get(snrs.len() / 2).copied().unwrap_or(0.);
    let usable_fraction = snrs.len() as f64 / count.max(1) as f64;
    BandScore {
        band,
        reference_snr,
        usable_fraction,
        score: reference_snr * usable_fraction,
    }
}

fn reference_stats(
    scan: &Path,
    frames: &[ReferenceFrame],
    screen: &ScreenMap,
    mirror: bool,
    calibration: Option<&material::Calibration>,
    progress: &Mutex<ReconstructionProgress>,
    label: &str,
) -> Result<[preprocessing::ReferenceStats; 3], String> {
    let len = DETECTOR_N * DETECTOR_N;
    let mut stats: [preprocessing::ReferenceStats; 3] =
        std::array::from_fn(|_| preprocessing::ReferenceStats {
            mean: vec![0.; len],
            variance: vec![0.; len],
        });
    let mut invalid: [Vec<bool>; 3] = std::array::from_fn(|_| vec![false; len]);
    for (k, frame) in frames.iter().enumerate() {
        let image = channels::decode(&verified_path(
            scan,
            &frame.path,
            frame.bytes,
            &frame.sha256,
        )?)?;
        let mut sampled = channels::sample(&image, screen, mirror)?;
        register_blue(&mut sampled, calibration);
        for b in 0..3 {
            for i in 0..len {
                invalid[b][i] |= sampled.saturated[b][i];
                let delta = sampled.values[b][i] - stats[b].mean[i];
                stats[b].mean[i] += delta / (k + 1) as f32;
                stats[b].variance[i] += delta * (sampled.values[b][i] - stats[b].mean[i]);
            }
        }
        set_progress(
            progress,
            "running",
            5 + k as u8,
            &format!("{label} {}/{}", k + 1, frames.len()),
        );
    }
    for b in 0..3 {
        for i in 0..len {
            stats[b].variance[i] /= frames.len().saturating_sub(1).max(1) as f32;
            if invalid[b][i] {
                stats[b].mean[i] = f32::NAN;
            }
        }
    }
    Ok(stats)
}

fn register_blue(sampled: &mut channels::Samples, calibration: Option<&material::Calibration>) {
    let Some(calibration) = calibration else {
        return;
    };
    let source = sampled.values[2].clone();
    let source_mask = sampled.saturated[2].clone();
    let n = DETECTOR_N;
    let a = calibration.blue_to_green_affine;
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            let xx = a[0][0] * x as f64 + a[0][1] * y as f64 + a[0][2];
            let yy = a[1][0] * x as f64 + a[1][1] * y as f64 + a[1][2];
            if xx < 0. || yy < 0. || xx >= (n - 1) as f64 || yy >= (n - 1) as f64 {
                sampled.values[2][i] = f32::NAN;
                sampled.saturated[2][i] = true;
                continue;
            }
            sampled.values[2][i] = sample_detector(&source, xx, yy);
            let j = yy.floor() as usize * n + xx.floor() as usize;
            sampled.saturated[2][i] = [j, j + 1, j + n, j + n + 1].iter().any(|k| source_mask[*k]);
        }
    }
}

fn floats(path: &Path) -> Result<Vec<f32>, String> {
    let bytes = fs::read(path).map_err(io_error)?;
    if bytes.len() % 4 != 0 {
        return Err("float artifact length is invalid".into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect())
}

fn write_float_file(path: &Path, values: &[f32]) -> Result<(), String> {
    let mut writer = BufWriter::new(File::create(path).map_err(io_error)?);
    write_floats(&mut writer, values)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)
}

fn prepare(
    scan: &Path,
    manifest: &Manifest,
    screen: &ScreenMap,
    root: &Path,
    dark: &[preprocessing::ReferenceStats; 3],
    flat: &[preprocessing::ReferenceStats; 3],
    active: &[Band],
    automatic: &defects::Plan,
    calibration: Option<&material::Calibration>,
    progress: &Mutex<ReconstructionProgress>,
) -> Result<(), String> {
    let roots: [PathBuf; 3] = std::array::from_fn(|b| root.join(Band::ALL[b].id()));
    let mut writers = Vec::new();
    let mut masks = Vec::new();
    let mut estimates = Vec::new();
    let mut quality: [Vec<preprocessing::Quality>; 3] = std::array::from_fn(|_| Vec::new());
    for b in 0..3 {
        fs::create_dir_all(&roots[b]).map_err(io_error)?;
        writers.push(BufWriter::new(
            File::create(roots[b].join("projections.f32le")).map_err(io_error)?,
        ));
        masks.push(BufWriter::new(
            File::create(roots[b].join("detector-quality.bin")).map_err(io_error)?,
        ));
        estimates.push(BufWriter::new(File::create(roots[b].join("estimated-rays.bin")).map_err(io_error)?));
        for (name, values) in [
            ("dark-crop.f32le", &dark[b].mean),
            ("dark-variance.f32le", &dark[b].variance),
            ("flat-crop.f32le", &flat[b].mean),
            ("flat-variance.f32le", &flat[b].variance),
        ] {
            write_float_file(&roots[b].join(name), values)?;
        }
        write_float_file(&roots[b].join("repaired-flat.f32le"), &automatic.flat[b].mean)?;
    }
    for frame in &manifest.frames {
        let image = channels::decode(&verified_path(
            scan,
            &frame.path,
            frame.bytes,
            &frame.sha256,
        )?)?;
        let mut sampled = channels::sample(&image, screen, manifest.geometry.mirror_x)?;
        register_blue(&mut sampled, calibration);
        for b in 0..3 {
            if !active.contains(&Band::ALL[b]) {
                continue;
            }
            for (i, saturated) in sampled.saturated[b].iter().enumerate() {
                if *saturated {
                    sampled.values[b][i] = sampled.white[b];
                }
            }
            let correction = preprocessing::correct_automatic(
                &sampled.values[b],
                sampled.white[b],
                &dark[b],
                &flat[b],
                preprocessing::support_threshold(&dark[b], &flat[b]),
                frame.index,
                automatic,
                b,
            )?;
            write_floats(&mut writers[b], &correction.projection)?;
            masks[b].write_all(&correction.flags).map_err(io_error)?;
            estimates[b].write_all(&correction.estimated).map_err(io_error)?;
            quality[b].push(correction.quality);
        }
        set_progress(
            progress,
            "running",
            (15 + frame.index as usize * 30 / manifest.frames.len()) as u8,
            &format!(
                "Linear RGB correction {}/{}",
                frame.index,
                manifest.frames.len()
            ),
        );
    }
    for writer in writers.iter_mut().chain(masks.iter_mut()).chain(estimates.iter_mut()) {
        writer.flush().map_err(io_error)?;
        writer.get_ref().sync_all().map_err(io_error)?;
    }
    for b in 0..3 {
        write_json(
            &roots[b].join("preprocessing.json"),
            &serde_json::json!({
                "schemaVersion":1,"algorithmVersion":ALGORITHM_VERSION,"band":Band::ALL[b],
                "screenCfaPixels":screen,"detectorShape":[DETECTOR_N,DETECTOR_N],"quality":quality[b],
                "cfaRegistration":"R,G1,G2,B offsets applied before averaging greens; native saturation propagated before detector downsampling",
                "logTransform":"measured dark and band-specific flat; no white balance or gamma",
                "noiseFloor":"dark measured variance; censored samples retained for display, excluded from quantitative colour confidence",
                "automaticDefects":"../detector-defects.json; reference-only colour-independent detection, local same-channel gain interpolation; raw invalid-data gate unchanged",
                "qualityFlags":{"0":"outside","1":"measured","2":"noise censored","3":"original reference missing","4":"original saturated","5":"measured low gain","6":"reference repaired","7":"sensor interpolated"},
                "estimatedRays":{"path":"estimated-rays.bin","0":"unmodified","1":"reference repaired","2":"projection interpolated","sha256":hash_file(&roots[b].join("estimated-rays.bin"))?.1},
                "projectionSha256":hash_file(&roots[b].join("projections.f32le"))?.1,
                "qualitySha256":hash_file(&roots[b].join("detector-quality.bin"))?.1
            }),
        )?;
    }
    Ok(())
}

fn copy_primary(child: &Path, root: &Path) -> Result<(), String> {
    for name in [
        "volume.f32le",
        "coverage.bin",
        "preview.bin",
        "preview.json",
        "preprocessing.json",
        "calibration.json",
    ] {
        fs::copy(child.join(name), root.join(name)).map_err(io_error)?;
    }
    Ok(())
}

/// Version-3 presentation only: robust signed window, without altering the
/// scientific volume or the numerical/window contract of any older cache.
fn automatic_window(volume:&[f32])->(f32,f32) {
    let mut values:Vec<_>=volume.iter().copied().filter(|x|x.is_finite()).collect();
    let low=values.len()/100;
    let high=values.len()*995/1000;
    let lo=*values.select_nth_unstable_by(low,f32::total_cmp).1;
    let hi=*values.select_nth_unstable_by(high,f32::total_cmp).1;
    (lo,hi.max(lo+1e-6))
}

fn finish_automatic(request:&ReconstructionRequest,manifest:&Manifest,manifest_hash:&str,
    child:&Path,axis:&preprocessing::AxisEstimate,iterative:bool,
    progress:&Mutex<ReconstructionProgress>)->Result<(),String> {
    let mut volume=floats(&child.join("volume.f32le"))?;
    if iterative {
        set_progress(progress,"running",95,"Validating cone-beam PWLS-Huber correction");
        let stack=floats(&child.join("projections.f32le"))?;
        let flags=fs::read(child.join("detector-quality.bin")).map_err(io_error)?;
        let dark=floats(&child.join("dark-crop.f32le"))?;
        let flat=floats(&child.join("repaired-flat.f32le"))?;
        let gain:Vec<_>=flat.iter().zip(dark).map(|(f,d)|f-d).collect();
        let angles:Vec<_>=manifest.frames.iter().map(|f|f.angle_deg.to_radians()*
            if manifest.geometry.rotation_direction=="counterclockwise" {1.}else{-1.}).collect();
        let g=&manifest.geometry;
        let report=refinement::refine(&mut volume,VOLUME_N,&stack,&flags,&angles,
            refinement::Geometry{sod:g.sod_mm,sdd:g.sod_mm+g.object_to_detector_mm,
                width:g.detector_width_mm,offset_u:axis.applied_offset_mm,offset_v:g.center_offset_y_mm},
            &gain,&floats(&child.join("dark-variance.f32le"))?,&floats(&child.join("flat-variance.f32le"))?);
        write_json(&child.join("refinement.json"),&report)?;
        if report.accepted {
            fs::copy(child.join("volume.f32le"),child.join("fdk-baseline.f32le")).map_err(io_error)?;
            let meta:CacheMetadata=serde_json::from_reader(File::open(child.join("result.json")).map_err(io_error)?).map_err(|e|e.to_string())?;
            let selection=BackendSelection{name:meta.backend,cpu_estimated_ms:meta.cpu_estimated_ms,
                gpu_estimated_ms:meta.gpu_estimated_ms,reason:meta.backend_selection};
            persist_result(&request.scan_dir,child,manifest,manifest_hash.into(),&volume,
                &fs::read(child.join("coverage.bin")).map_err(io_error)?,meta.spacing_mm[0],request.method,&selection)?;
        }
    } else {
        write_json(&child.join("refinement.json"),&serde_json::json!({"accepted":false,
            "reason":"automatic iterative correction is limited to uncalibrated monochrome; shared multi-band quantitative model retained"}))?;
    }
    let (window_min,window_max)=automatic_window(&volume);
    write_bytes_durable(&child.join("preview.bin"),&full_volume_preview(&volume,VOLUME_N,PREVIEW_N,window_min,window_max)?)?;
    let mut meta:CacheMetadata=serde_json::from_reader(File::open(child.join("result.json")).map_err(io_error)?).map_err(|e|e.to_string())?;
    meta.preview_sha256=hash_file(&child.join("preview.bin"))?.1;
    write_json(&child.join("preview.json"),&PreviewMetadata{shape:meta.preview_shape,spacing_mm:meta.preview_spacing_mm,
        window_min,window_max,sha256:meta.preview_sha256.clone()})?;
    meta.note.push_str("; v3 automatic reference defect estimates excluded from quantitative coverage; display window signed P1-P99.5; optional accepted coarse PWLS-Huber correction recorded in refinement.json");
    write_json(&child.join("result.json"),&meta)
}

pub(super) fn reconstruct(
    request: &ReconstructionRequest,
    options: &Options,
    progress: &Mutex<ReconstructionProgress>,
) -> Result<ReconstructionResult, String> {
    if request.method != ReconstructionMethod::Fdk {
        return Err("colour reconstruction currently requires validated FDK".into());
    }
    let options = effective_options(&request.scan_dir, options)?;
    if let Some(result) = cached_result(&request.scan_dir, request.method, &options)? {
        return Ok(result);
    }
    let (root, identity, calibration) =
        input_identity(&request.scan_dir, request.method, &options)?;
    reconstruct_fresh(request,options,progress,root,identity,calibration)
}

fn reconstruct_fresh(request:&ReconstructionRequest,options:Options,progress:&Mutex<ReconstructionProgress>,
    root:PathBuf,identity:String,calibration:Option<material::Calibration>)->Result<ReconstructionResult,String> {
    fs::create_dir_all(&root).map_err(io_error)?;
    if root.join("result.json").exists() {
        fs::remove_file(root.join("result.json")).map_err(io_error)?;
    }
    let (manifest, manifest_hash) = read_manifest(&request.scan_dir)?;
    let load = |f: &ReferenceFrame| {
        channels::decode(&verified_path(
            &request.scan_dir,
            &f.path,
            f.bytes,
            &f.sha256,
        )?)
    };
    let screen = channels::screen(
        &load(&manifest.references.pre_flat[0])?,
        &load(&manifest.references.pre_dark[0])?,
    )?;
    let dark = reference_stats(
        &request.scan_dir,
        &manifest.references.pre_dark,
        &screen,
        manifest.geometry.mirror_x,
        calibration.as_ref(),
        progress,
        "Dark reference",
    )?;
    let flat = reference_stats(
        &request.scan_dir,
        &manifest.references.pre_flat,
        &screen,
        manifest.geometry.mirror_x,
        calibration.as_ref(),
        progress,
        "Flat reference",
    )?;
    set_progress(progress,"running",15,"Detecting reference defects and preserving support boundaries");
    let mut automatic = defects::detect(&dark,&flat,DETECTOR_N);
    let mut probes=Vec::new();
    let probe_count=manifest.frames.len().min(12);
    for k in 0..probe_count {
        let frame=&manifest.frames[k*manifest.frames.len()/probe_count];
        let image=channels::decode(&verified_path(&request.scan_dir,&frame.path,frame.bytes,&frame.sha256)?)?;
        let mut sample=channels::sample(&image,&screen,manifest.geometry.mirror_x)?;
        register_blue(&mut sample,calibration.as_ref());
        for b in 0..3 {for i in 0..DETECTOR_N*DETECTOR_N {if sample.saturated[b][i] {sample.values[b][i]=f32::NAN;}}}
        probes.push(sample.values);
    }
    defects::confirm_persistent_gain(&mut automatic,&dark,&flat,&probes,DETECTOR_N);
    drop(probes);
    write_bytes_durable(&root.join("detector-defects.bin"),&automatic.mask)?;
    let gain_mask:Vec<_>=automatic.persistent_gain.iter().flat_map(|v|v.iter().map(|&b|u8::from(b))).collect();
    write_bytes_durable(&root.join("persistent-gain.bin"),&gain_mask)?;
    write_json(&root.join("detector-defects.json"),&serde_json::json!({
        "report":automatic.report,"shape":[DETECTOR_N,DETECTOR_N],
        "maskBits":{"1":"reference anomaly candidate; applied estimates recorded per ray","2":"sensor defect","4":"protected support or boundary"},
        "maskSha256":hash_file(&root.join("detector-defects.bin"))?.1
        ,"persistentGain":{"path":"persistent-gain.bin","layout":"R,G,B planar 256x256 uint8; 1 retains original measured flat gain after >=8 valid probes and >=80% agreement","sha256":hash_file(&root.join("persistent-gain.bin"))?.1}
    }))?;
    let scores: Vec<_> = Band::ALL
        .iter()
        .map(|b| band_score(*b, &dark[b.index()], &flat[b.index()]))
        .collect();
    let best = scores
        .iter()
        .max_by(|a, b| a.score.total_cmp(&b.score))
        .ok_or("no camera bands")?
        .band;
    let selected = options.band.unwrap_or(best);
    if scores[selected.index()].usable_fraction < 0.5 {
        return Err("selected band has insufficient measured reference support".into());
    }
    let observations = observation_bands(&options.layers, calibration.is_some());
    if options.mode == Mode::Multicolor
        && observations
            .iter()
            .any(|b| scores[b.index()].usable_fraction < 0.5)
    {
        return Err("configured emission bands lack sufficient measured reference support".into());
    }
    let mut bands = vec![selected];
    if options.mode == Mode::Multicolor {
        for band in &observations {
            if !bands.contains(band) {
                bands.push(*band);
            }
        }
    }
    prepare(
        &request.scan_dir,
        &manifest,
        &screen,
        &root,
        &dark,
        &flat,
        &bands,
        &automatic,
        calibration.as_ref(),
        progress,
    )?;
    let primary = root.join(selected.id());
    let stack = floats(&primary.join("projections.f32le"))?;
    let angles: Vec<_> = manifest
        .frames
        .iter()
        .map(|f| {
            if manifest.geometry.rotation_direction == "counterclockwise" {
                f.angle_deg.to_radians()
            } else {
                -f.angle_deg.to_radians()
            }
        })
        .collect();
    let pitch = manifest.geometry.detector_width_mm * SCREEN_CROP_SCALE / DETECTOR_N as f64;
    let axis = preprocessing::estimate_axis_extended(
        &stack,
        &angles,
        pitch,
        manifest.geometry.sod_mm + manifest.geometry.object_to_detector_mm,
        manifest.geometry.center_offset_x_mm,
        manifest.geometry.center_offset_y_mm,
    );
    drop(stack);
    let axis_bytes = serde_json::to_vec(&axis).map_err(|e| e.to_string())?;
    for band in &bands {
        let child = root.join(band.id());
        let shared_axis = serde_json::from_slice(&axis_bytes).map_err(|e| e.to_string())?;
        reconstruct_prepared(
            request,
            progress,
            true,
            None,
            &manifest,
            manifest_hash.clone(),
            &child,
            &child.join("projections.f32le"),
            Some(shared_axis),
        )?;
        finish_automatic(request,&manifest,&manifest_hash,&child,&axis,
            options.mode==Mode::Monochrome && calibration.is_none(),progress)?;
    }
    // Do not expose a primary-only seal while additional artifacts are pending.
    copy_primary(&primary, &root)?;
    let mut material_report = serde_json::Value::Null;
    if let Some(calibration) = &calibration {
        material_report = decompose(
            request,
            &manifest,
            &manifest_hash,
            &root,
            calibration,
            &axis_bytes,
            progress,
        )?;
    }
    if options.mode == Mode::Multicolor {
        colour_preview(&root, &observations, calibration.as_ref())?;
    }
    let mut artifacts = Vec::new();
    for name in ["detector-defects.bin","detector-defects.json","persistent-gain.bin"] {
        artifacts.push(serde_json::json!({"path":name,"sha256":hash_file(&root.join(name))?.1}));
    }
    for band in &bands {
        for name in [
            "result.json",
            "volume.f32le",
            "coverage.bin",
            "projections.f32le",
            "detector-quality.bin",
            "estimated-rays.bin",
            "preprocessing.json",
            "repaired-flat.f32le",
            "refinement.json",
            "dark-crop.f32le",
            "dark-variance.f32le",
            "flat-crop.f32le",
            "flat-variance.f32le",
        ] {
            let relative = format!("{}/{}", band.id(), name);
            artifacts.push(
                serde_json::json!({"path":relative,"sha256":hash_file(&root.join(&relative))?.1}),
            );
        }
        let baseline=format!("{}/fdk-baseline.f32le",band.id());
        if root.join(&baseline).is_file() {
            artifacts.push(serde_json::json!({"path":baseline,"sha256":hash_file(&root.join(&baseline))?.1}));
        }
    }
    if options.mode == Mode::Multicolor {
        for name in ["multicolor-preview.bin", "multicolor-preview.json"] {
            artifacts
                .push(serde_json::json!({"path":name,"sha256":hash_file(&root.join(name))?.1}));
        }
    }
    if calibration.is_some() {
        for basis in ["basis-a", "basis-b"] {
            for name in ["volume.f32le", "coverage.bin", "result.json"] {
                let relative = format!("{basis}/{name}");
                artifacts.push(serde_json::json!({"path":relative,"sha256":hash_file(&root.join(&relative))?.1}));
            }
        }
    }
    if calibration
        .as_ref()
        .is_some_and(|c| c.material_library.is_some())
    {
        for name in [
            "material-candidates.u16le",
            "material-match-error.f32le",
            "material-density-scale.f32le",
            "material-matching.json",
        ] {
            artifacts
                .push(serde_json::json!({"path":name,"sha256":hash_file(&root.join(name))?.1}));
        }
    }
    write_json(
        &root.join("spectral.json"),
        &serde_json::json!({
            "schemaVersion":1,"algorithmVersion":ALGORITHM_VERSION,"identity":identity,"options":options,
            "selectedBand":selected,"bandScores":scores,
            "selectionReason":"automatic maximizes measured central reference SNR multiplied by usable fraction; estimates signal reliability, not universal material contrast optimality",
            "materialDecomposition":material_report,"quantitativeMaterials":calibration.is_some(),
            "observableCameraBands":3,"configuredLayers":options.layers.len(),
            "reconstructedBands":bands,"displayBands":observations,
            "layerSeparation":"camera colour channels contain optical mixing; repeated colours and >3 layers are not individually resolved",
            "geometry":"common CFA/rotation coordinates; calibrated material mode additionally requires measured cross-channel registration matching recorded geometry; uncalibrated layer depth and chromatic differences are unresolved",
            "artifacts":artifacts
        }),
    )?;
    let mut meta: CacheMetadata =
        serde_json::from_reader(File::open(primary.join("result.json")).map_err(io_error)?)
            .map_err(|e| e.to_string())?;
    meta.spectral_sha256 = Some(hash_file(&root.join("spectral.json"))?.1);
    meta.geometry.scintillator_layers = options.layers.clone();
    meta.note.push_str("; versioned RGB reconstruction, automatic band reliability selection, colour is qualitative unless a matched measured two-basis calibration passed held-out validation");
    write_json(&root.join("result.json.pending"), &meta)?;
    if hash_file(&request.scan_dir.join("manifest.json"))?.1 != manifest_hash {
        return Err("manifest changed before colour result publication".into());
    }
    fs::rename(root.join("result.json.pending"), root.join("result.json")).map_err(io_error)?;
    cached_result_root(&request.scan_dir, request.method, &root,&identity)?
        .ok_or("colour result failed final integrity verification".into())
}

fn decompose(
    request: &ReconstructionRequest,
    manifest: &Manifest,
    manifest_hash: &str,
    root: &Path,
    calibration: &material::Calibration,
    axis: &[u8],
    progress: &Mutex<ReconstructionProgress>,
) -> Result<serde_json::Value, String> {
    let green = floats(&root.join("green/projections.f32le"))?;
    let blue = floats(&root.join("blue/projections.f32le"))?;
    let gm = fs::read(root.join("green/detector-quality.bin")).map_err(io_error)?;
    let bm = fs::read(root.join("blue/detector-quality.bin")).map_err(io_error)?;
    let mut projections = [vec![0f32; green.len()], vec![0f32; green.len()]];
    let mut mask = vec![3u8; green.len()];
    let mut accepted = 0usize;
    let mut candidates = 0usize;
    let mut max_condition = 0f64;
    for i in 0..green.len() {
        if !matches!(gm[i], 1 | 5) || !matches!(bm[i], 1 | 5) {
            continue;
        }
        candidates += 1;
        if let Some(e) = calibration.invert([f64::from(green[i]), f64::from(blue[i])]) {
            projections[0][i] = e.thickness_mm[0] as f32;
            projections[1][i] = e.thickness_mm[1] as f32;
            mask[i] = 1;
            accepted += 1;
            max_condition = max_condition.max(e.condition);
        }
    }
    if candidates == 0 || accepted as f64 / (candidates as f64) < 0.8 {
        return Err("too many rays are outside the calibrated material domain; no material result published".into());
    }
    for (b, name) in ["basis-a", "basis-b"].iter().enumerate() {
        let child = root.join(name);
        fs::create_dir_all(&child).map_err(io_error)?;
        write_float_file(&child.join("projections.f32le"), &projections[b])?;
        write_bytes_durable(&child.join("detector-quality.bin"), &mask)?;
        write_json(
            &child.join("preprocessing.json"),
            &serde_json::json!({"units":"basis path length mm","basis":calibration.basis_names[b],
            "calibration":calibration,"acceptedRays":accepted,"candidateRays":candidates,"maximumJacobianCondition":max_condition}),
        )?;
        reconstruct_prepared(
            request,
            progress,
            true,
            None,
            manifest,
            manifest_hash.to_owned(),
            &child,
            &child.join("projections.f32le"),
            Some(serde_json::from_slice(axis).map_err(|e| e.to_string())?),
        )?;
    }
    let matching = if let Some(library) = &calibration.material_library {
        match_material_volume(root, calibration, library)?
    } else {
        serde_json::Value::Null
    };
    Ok(
        serde_json::json!({"basisNames":calibration.basis_names,"volumeUnits":"basis volume fraction under calibrated basis assumptions",
        "acceptedRays":accepted,"candidateRays":candidates,"maximumJacobianCondition":max_condition,
        "unknownMaterialMatching":matching,
        "validation":"independent calibration points <=5% of calibrated thickness range; circular FDK and incomplete coverage remain limitations"}),
    )
}

fn match_material_volume(
    root: &Path,
    calibration: &material::Calibration,
    library: &material::BasisLibrary,
) -> Result<serde_json::Value, String> {
    let prepared = library.prepare(&calibration.basis_names)?;
    let a = floats(&root.join("basis-a/volume.f32le"))?;
    let b = floats(&root.join("basis-b/volume.f32le"))?;
    let am = fs::read(root.join("basis-a/coverage.bin")).map_err(io_error)?;
    let bm = fs::read(root.join("basis-b/coverage.bin")).map_err(io_error)?;
    if a.len() != VOLUME_N.pow(3)
        || b.len() != a.len()
        || am.len() != a.len()
        || bm.len() != a.len()
    {
        return Err("basis volume and coverage shapes differ".into());
    }
    let mut labels = Vec::with_capacity(a.len() * 2);
    let mut errors = Vec::with_capacity(a.len());
    let mut scales = Vec::with_capacity(a.len());
    let mut counts = vec![0usize; library.candidates.len()];
    let mut unknown = 0usize;
    let mut ambiguous = 0usize;
    let mut supported = 0usize;
    for i in 0..a.len() {
        let matched = if am[i] == 1 && bm[i] == 1 {
            supported += 1;
            prepared.classify([f64::from(a[i]), f64::from(b[i])])
        } else {
            material::CandidateMatch {
                label: 0,
                best_candidate: 0,
                density_scale: 0.,
                squared_error: f64::NAN,
            }
        };
        match matched.label {
            0 => unknown += 1,
            u16::MAX => ambiguous += 1,
            id => counts[id as usize - 1] += 1,
        }
        labels.extend_from_slice(&matched.label.to_le_bytes());
        errors.push(matched.squared_error as f32);
        scales.push(if matched.label != 0 && matched.label != u16::MAX {
            matched.density_scale as f32
        } else {
            f32::NAN
        });
    }
    write_bytes_durable(&root.join("material-candidates.u16le"), &labels)?;
    write_float_file(&root.join("material-match-error.f32le"), &errors)?;
    write_float_file(&root.join("material-density-scale.f32le"), &scales)?;
    let report = serde_json::json!({"schemaVersion":1,"shape":[VOLUME_N,VOLUME_N,VOLUME_N],
        "library":library,"candidateCounts":counts,"supportedVoxels":supported,
        "unknownVoxels":unknown,"ambiguousVoxels":ambiguous,
        "labelEncoding":"u16 little-endian; 0 unknown or unsupported; 65535 ambiguous; other values are 1-based library indices",
        "materialIdentityVerified":false,"targetMaterialRequired":false,"targetThicknessRequired":false,
        "densityScaleUnits":"relative to each candidate signature reference density; NaN outside accepted unique candidates",
        "interpretation":"candidate hypotheses from calibrated basis coordinates, unknown density amplitude profiled out; mixture voxels may imitate a pure material, so classification requires independent phantom validation"});
    write_json(&root.join("material-matching.json"), &report)?;
    Ok(report)
}

fn colour_preview(
    root: &Path,
    bands: &[Band],
    calibration: Option<&material::Calibration>,
) -> Result<(), String> {
    let material = calibration.is_some();
    let names: Vec<&str> = if material {
        vec!["basis-a", "basis-b"]
    } else {
        bands.iter().map(|b| b.id()).collect()
    };
    let mut views = Vec::new();
    let mut masks = Vec::new();
    for name in &names {
        let child = root.join(name);
        views.push(fs::read(child.join("preview.bin")).map_err(io_error)?);
        let mask = fs::read(child.join("coverage.bin")).map_err(io_error)?;
        let mut small = vec![1u8; PREVIEW_N.pow(3)];
        let factor = VOLUME_N / PREVIEW_N;
        for z in 0..PREVIEW_N {
            for y in 0..PREVIEW_N {
                for x in 0..PREVIEW_N {
                    for dz in 0..factor {
                        for dy in 0..factor {
                            for dx in 0..factor {
                                small[(z * PREVIEW_N + y) * PREVIEW_N + x] &=
                                    mask[((z * factor + dz) * VOLUME_N + y * factor + dy)
                                        * VOLUME_N
                                        + x * factor
                                        + dx];
                            }
                        }
                    }
                }
            }
        }
        masks.push(small);
    }
    let grayscale = fs::read(root.join("preview.bin")).map_err(io_error)?;
    let mut rgb = Vec::with_capacity(grayscale.len() * 3);
    for (i, gray) in grayscale.iter().enumerate() {
        if masks.iter().any(|mask| mask[i] == 0) {
            rgb.extend_from_slice(&[*gray; 3]);
            continue;
        }
        let mut pixel = [0u8; 3];
        if material {
            pixel = [
                views[0][i],
                views[1][i],
                ((u16::from(views[0][i]) + u16::from(views[1][i])) / 4) as u8,
            ];
        } else {
            for (b, band) in bands.iter().enumerate() {
                pixel[band.index()] = views[b][i];
            }
        }
        rgb.extend_from_slice(&pixel);
    }
    write_bytes_durable(&root.join("multicolor-preview.bin"), &rgb)?;
    write_json(
        &root.join("multicolor-preview.json"),
        &serde_json::json!({
            "shape":[PREVIEW_N,PREVIEW_N,PREVIEW_N],"components":3,"format":"interleaved RGB uint8; z,y,x",
            "sha256":hash_file(&root.join("multicolor-preview.bin"))?.1,
            "channels":names,"materialLabels":material,"grayscaleOutsideCommonQuantitativeCoverage":true,
            "legend":calibration.map(|c|c.basis_names.to_vec()).unwrap_or_else(||names.iter().map(|s|format!("{s} response")).collect()),
            "note":"colour previews use display windows only; quantitative volumes remain signed float32"
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_signed_window_ignores_isolated_extremes_without_clipping_negative_values() {
        let mut v:Vec<f32>=(0..10000).map(|i|i as f32/10000.-0.3).collect();
        v[0]=-1000.;v[9999]=1000.;
        let (lo,hi)=automatic_window(&v);
        assert!((-0.30..-0.28).contains(&lo));assert!((0.69..0.70).contains(&hi));
        assert_eq!(v[0],-1000.);
    }
    #[test]
    #[ignore = "explicit read-only sealed scan; all new results under repository .cache"]
    fn inspect_automatic_scan_offline() {
        let scan=PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let root=PathBuf::from(std::env::var("CT_DIAGNOSTIC_DIR").expect("CT_DIAGNOSTIC_DIR"));
        fs::create_dir_all(&root).unwrap();
        let root=root.canonicalize().unwrap();
        let allowed=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cache/tests/ct-engine").canonicalize().unwrap();
        assert!(root.starts_with(allowed));
        assert!(!root.join("result.json").exists(),"keep existing evidence");
        let options=effective_options(&scan,&Options::default()).unwrap();
        let (_,identity,cal)=input_identity(&scan,ReconstructionMethod::Fdk,&options).unwrap();
        let progress=Arc::new(Mutex::new(ReconstructionProgress{status:"running",percent:0,message:String::new(),events:Vec::new()}));
        let monitor=Arc::clone(&progress);let done=Arc::new(AtomicBool::new(false));let stop=Arc::clone(&done);
        let reporter=thread::spawn(move || {while !stop.load(Ordering::Relaxed) {
            {let p=monitor.lock().unwrap();println!("{}% {}",p.percent,p.message);}
            thread::sleep(Duration::from_secs(5));
        }});
        let result=reconstruct_fresh(&ReconstructionRequest{scan_dir:scan,method:ReconstructionMethod::Fdk},options,&progress,root,identity,cal);
        done.store(true,Ordering::Relaxed);reporter.join().unwrap();
        println!("PASS automatic offline reconstruction {}",result.unwrap().cache_path.display());
    }
    #[test]
    fn emission_bands_follow_the_configured_stack_and_measured_calibration() {
        let layers: Vec<_> = ["blue", "red", "green", "blue", "other"]
            .iter()
            .map(|color| StackLayer {
                material_name: "glass".into(),
                emission_color: color.to_string(),
                thickness_mm: 1.,
            })
            .collect();
        assert_eq!(
            observation_bands(&layers[..2], false),
            vec![Band::Blue, Band::Red]
        );
        assert_eq!(observation_bands(&layers, false).len(), 3);
        assert_eq!(
            observation_bands(&layers, true),
            vec![Band::Green, Band::Blue]
        );
        assert!(Options {
            mode: Mode::Multicolor,
            band: Some(Band::Green),
            ..Options::default()
        }
        .validate()
        .is_err());
    }
    #[test]
    fn reliability_selection_uses_measured_noise_and_rejects_saturated_reference_support() {
        let count = DETECTOR_N.pow(2);
        let dark = preprocessing::ReferenceStats {
            mean: vec![100.; count],
            variance: vec![16.; count],
        };
        let bright = preprocessing::ReferenceStats {
            mean: vec![500.; count],
            variance: vec![16.; count],
        };
        let weak = preprocessing::ReferenceStats {
            mean: vec![110.; count],
            variance: vec![16.; count],
        };
        assert!(band_score(Band::Green, &dark, &bright).score > 50.);
        assert_eq!(band_score(Band::Blue, &dark, &weak).score, 0.);
        let saturated = preprocessing::ReferenceStats {
            mean: vec![f32::NAN; count],
            variance: vec![16.; count],
        };
        assert_eq!(band_score(Band::Red, &dark, &saturated).usable_fraction, 0.);
    }
    #[test]
    fn stack_validation_preserves_order_and_limits_five_filled_rows() {
        let layer = StackLayer {
            material_name: "scintillator glass".into(),
            emission_color: "green".into(),
            thickness_mm: 1.,
        };
        assert!(validate_layers(&vec![layer.clone(); 5]).is_ok());
        assert!(validate_layers(&vec![layer.clone(); 6]).is_err());
        let mut bad = layer;
        bad.thickness_mm = f64::NAN;
        assert!(validate_layers(&[bad]).is_err());
    }
    #[test]
    #[ignore]
    fn reconstruct_task4_colour_offline() {
        let scan = PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let options: Options = serde_json::from_str(
            &std::env::var("CT_SPECTRAL_OPTIONS").expect("CT_SPECTRAL_OPTIONS"),
        )
        .unwrap();
        let mut handle = ReconstructionHandle::start_with_options(
            ReconstructionRequest {
                scan_dir: scan.clone(),
                method: ReconstructionMethod::Fdk,
            },
            options.clone(),
        )
        .unwrap();
        loop {
            if let Some(result) = handle.try_finish() {
                let result = result.unwrap();
                assert!(cached_result(&scan, ReconstructionMethod::Fdk, &options)
                    .unwrap()
                    .is_some());
                println!("PASS {}", result.cache_path.display());
                break;
            }
            let state = handle.progress();
            println!("{}% {}", state.percent, state.message);
            thread::sleep(Duration::from_secs(5));
        }
    }
}
