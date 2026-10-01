//! Offline reconstruction of a sealed scan. This module never commands devices.
//!
//! The numerical path is linear Bayer-blue NEF -> measured dark/flat correction
//! -> circular cone-beam FDK -> float volume and a bounded uint8 preview. The
//! scientific volume keeps signed values; preview windowing is display-only.

use rawloader::RawImageData;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::f64::consts::PI;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

#[path = "reconstruction_preprocess.rs"]
mod preprocessing;

const DETECTOR_N: usize = 256;
const VOLUME_N: usize = 256;
const PREVIEW_N: usize = 128;
const SIRT_N: usize = 64;
const SIRT_ITERATIONS: usize = 5;
const SIRT_CPU_WORK_BUDGET: u64 = 200_000_000;
const REFERENCE_COUNT: usize = 10;
const CACHE_SCHEMA: u32 = 5;
// Small rim-fit guard; the physical screen diameter is NOT enlarged by padding.
const SCREEN_CROP_SCALE: f64 = 1.01;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconstructionMethod { Fdk, Sirt, Cgls }

impl ReconstructionMethod {
    pub fn id(self) -> &'static str {
        match self { Self::Fdk => "fdk", Self::Sirt => "sirt", Self::Cgls => "cgls" }
    }
}

#[derive(Clone, Debug)]
pub struct MethodAvailability {
    pub method: ReconstructionMethod,
    pub enabled: bool,
    pub reason: Option<String>,
}

/// A method is enabled only when its numerical backend really exists.
/// GPU display adapters alone do not imply a usable compute backend.
pub fn available_methods() -> Vec<MethodAvailability> {
    vec![
        MethodAvailability { method: ReconstructionMethod::Fdk, enabled: true, reason: None },
        MethodAvailability { method: ReconstructionMethod::Sirt, enabled: true, reason: None },
        MethodAvailability { method: ReconstructionMethod::Cgls, enabled: false,
            reason: Some("缺少已验证的 GPU 迭代后端".into()) },
    ]
}

/// Conservative CPU work gate: two matched projector passes per iteration.
/// SIRT currently has no GPU kernel; never advertise it as GPU accelerated.
pub fn sirt_feasible(projection_count: u32) -> bool {
    (projection_count as u64).saturating_mul((SIRT_N as u64).pow(3))
        .saturating_mul((SIRT_ITERATIONS * 2) as u64) <= SIRT_CPU_WORK_BUDGET
}

#[derive(Clone, Debug)]
pub struct ReconstructionRequest {
    pub scan_dir: PathBuf,
    pub method: ReconstructionMethod,
}

#[derive(Clone, Debug)]
pub struct ReconstructionProgress {
    pub status: &'static str,
    pub percent: u8,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct ReconstructionResult {
    pub method: ReconstructionMethod,
    pub cache_path: PathBuf,
    pub volume_path: PathBuf,
    pub preview_path: PathBuf,
}

pub struct ReconstructionHandle {
    progress: Arc<Mutex<ReconstructionProgress>>,
    outcome: mpsc::Receiver<Result<ReconstructionResult, String>>,
    worker: Option<JoinHandle<()>>,
}

impl ReconstructionHandle {
    pub fn start(request: ReconstructionRequest) -> Result<Self, String> {
        if !available_methods().iter().any(|entry| entry.method == request.method && entry.enabled) {
            return Err(format!("{} is not available", request.method.id()));
        }
        let progress = Arc::new(Mutex::new(ReconstructionProgress {
            status: "running", percent: 0, message: "准备重构".into(),
        }));
        let worker_progress = Arc::clone(&progress);
        let (sender, outcome) = mpsc::channel();
        let worker = thread::Builder::new().name("ct-reconstruction".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    reconstruct(&request, &worker_progress)
                })).unwrap_or_else(|_| Err("reconstruction worker panicked".into()));
                match &result {
                    Ok(_) => set_progress(&worker_progress, "completed", 100, "重构完成"),
                    Err(error) => {
                        let percent = worker_progress.lock().expect("reconstruction progress mutex poisoned").percent;
                        set_progress(&worker_progress, "fault", percent, error);
                    }
                }
                let _ = sender.send(result);
            }).map_err(|error| format!("could not start reconstruction: {error}"))?;
        Ok(Self { progress, outcome, worker: Some(worker) })
    }

    pub fn progress(&self) -> ReconstructionProgress {
        self.progress.lock().expect("reconstruction progress mutex poisoned").clone()
    }

    pub fn try_finish(&mut self) -> Option<Result<ReconstructionResult, String>> {
        match self.outcome.try_recv() {
            Ok(result) => {
                if let Some(worker) = self.worker.take() {
                    if worker.join().is_err() { return Some(Err("reconstruction worker panicked".into())); }
                }
                Some(result)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(worker) = self.worker.take() { let _ = worker.join(); }
                Some(Err("reconstruction worker exited without a result".into()))
            }
        }
    }
}

fn set_progress(progress: &Mutex<ReconstructionProgress>, status: &'static str, percent: u8, message: &str) {
    let mut state = progress.lock().expect("reconstruction progress mutex poisoned");
    state.status = status;
    state.percent = percent;
    state.message = message.into();
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u32,
    completed: bool,
    projection_count: u32,
    geometry: Geometry,
    frames: Vec<ProjectionFrame>,
    references: References,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Geometry {
    sod_mm: f64,
    object_to_detector_mm: f64,
    detector_width_mm: f64,
    center_offset_x_mm: f64,
    center_offset_y_mm: f64,
    rotation_direction: String,
    mirror_x: bool,
    measurement: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectionFrame {
    index: u32,
    angle_deg: f64,
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceFrame {
    index: u32,
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct References {
    pre_dark: Vec<ReferenceFrame>,
    pre_flat: Vec<ReferenceFrame>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheMetadata {
    schema_version: u32,
    method: ReconstructionMethod,
    manifest_sha256: String,
    shape: [usize; 3],
    spacing_mm: [f64; 3],
    preview_shape: [usize; 3],
    preview_spacing_mm: [f64; 3],
    volume_sha256: String,
    preview_sha256: String,
    coverage_sha256: String,
    preprocessing_sha256: String,
    calibration_sha256: String,
    backend: String,
    cpu_estimated_ms: u64,
    gpu_estimated_ms: Option<u64>,
    backend_selection: String,
    geometry: Geometry,
    note: String,
}

#[derive(Clone)]
struct BackendSelection {
    name: String,
    cpu_estimated_ms: u64,
    gpu_estimated_ms: Option<u64>,
    reason: String,
}

fn gpu_beats_cpu(cpu: Duration, gpu: Duration) -> bool {
    // Equivalent to gpu <= 0.85 * cpu without floating rounding surprises.
    gpu.as_nanos().saturating_mul(100) <= cpu.as_nanos().saturating_mul(85)
}

fn estimated_duration(sample: Duration, views: usize) -> Duration {
    sample.saturating_mul(u32::try_from(views).unwrap_or(u32::MAX))
}

fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewMetadata {
    shape: [usize; 3],
    spacing_mm: [f64; 3],
    window_min: f32,
    window_max: f32,
    sha256: String,
}

fn cache_dir(scan_dir: &Path, method: ReconstructionMethod) -> PathBuf {
    scan_dir.join("reconstruction").join(method.id())
}

pub fn cached_result(scan_dir: &Path, method: ReconstructionMethod) -> Result<Option<ReconstructionResult>, String> {
    let root = cache_dir(scan_dir, method);
    let metadata_path = root.join("result.json");
    if !metadata_path.is_file() { return Ok(None); }
    let Ok(meta) = serde_json::from_reader::<_, CacheMetadata>(
        File::open(&metadata_path).map_err(io_error)?) else { return Ok(None); };
    if meta.schema_version != CACHE_SCHEMA || meta.method != method { return Ok(None); }
    for (name,expected) in [("preprocessing.json",&meta.preprocessing_sha256),("calibration.json",&meta.calibration_sha256)] {
        let path=root.join(name);
        if !path.is_file() || &hash_file(&path)?.1!=expected {return Ok(None);}
    }
    let manifest_path = scan_dir.join("manifest.json");
    if !manifest_path.is_file() || hash_file(&manifest_path)?.1 != meta.manifest_sha256 { return Ok(None); }
    let volume_path = root.join("volume.f32le");
    let preview_path = root.join("preview.bin");
    let coverage_path = root.join("coverage.bin");
    let preview_meta_path = root.join("preview.json");
    if !preview_meta_path.is_file() || !volume_path.is_file() || !preview_path.is_file() || !coverage_path.is_file() {
        return Ok(None);
    }
    let Ok(preview_meta) = serde_json::from_reader::<_, PreviewMetadata>(
        File::open(&preview_meta_path).map_err(io_error)?) else { return Ok(None); };
    let voxels = meta.shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b))
        .ok_or("invalid reconstruction shape")?;
    let preview_voxels = meta.preview_shape.iter().try_fold(1usize, |a, b| a.checked_mul(*b))
        .ok_or("invalid preview shape")?;
    let (expected_volume_n, expected_preview_n) = match method {
        ReconstructionMethod::Fdk => (VOLUME_N, PREVIEW_N),
        ReconstructionMethod::Sirt => (SIRT_N, SIRT_N),
        ReconstructionMethod::Cgls => return Ok(None),
    };
    if meta.shape != [expected_volume_n; 3] || meta.preview_shape != [expected_preview_n; 3]
        || meta.spacing_mm.iter().any(|value| !value.is_finite() || *value <= 0.0)
        || meta.preview_spacing_mm.iter().any(|value| !value.is_finite() || *value <= 0.0)
        || preview_meta.shape != meta.preview_shape || preview_meta.spacing_mm != meta.preview_spacing_mm
        || !preview_meta.window_min.is_finite() || !preview_meta.window_max.is_finite()
        || preview_meta.window_max <= preview_meta.window_min
        || preview_meta.sha256 != meta.preview_sha256
        || hash_file(&volume_path)? != ((voxels * 4) as u64, meta.volume_sha256)
        || hash_file(&preview_path)? != (preview_voxels as u64, meta.preview_sha256)
        || hash_file(&coverage_path)? != (voxels as u64, meta.coverage_sha256) {
        return Ok(None);
    }
    Ok(Some(ReconstructionResult { method, cache_path: root, volume_path, preview_path }))
}

fn read_manifest(scan_dir: &Path) -> Result<(Manifest, String), String> {
    let path = scan_dir.join("manifest.json");
    let digest = hash_file(&path)?.1;
    let manifest: Manifest = serde_json::from_reader(File::open(path).map_err(io_error)?)
        .map_err(|error| format!("invalid scan manifest: {error}"))?;
    if manifest.schema_version != 2 || !manifest.completed { return Err("scan is not sealed".into()); }
    if manifest.frames.len() != manifest.projection_count as usize || manifest.frames.len() < 3 {
        return Err("projection count does not match manifest".into());
    }
    for (name, group) in [
        ("preDark", &manifest.references.pre_dark), ("preFlat", &manifest.references.pre_flat),
    ] {
        if group.len() != REFERENCE_COUNT { return Err(format!("{name} needs 10 confirmed frames")); }
        for (i, frame) in group.iter().enumerate() {
            if frame.index != i as u32 + 1 { return Err(format!("{name} indices are incomplete")); }
        }
    }
    for (i, frame) in manifest.frames.iter().enumerate() {
        if frame.index != i as u32 + 1 || !frame.angle_deg.is_finite() {
            return Err("projection indices or angles are invalid".into());
        }
    }
    let g = &manifest.geometry;
    if !g.sod_mm.is_finite() || !g.object_to_detector_mm.is_finite() || !g.detector_width_mm.is_finite()
        || !g.center_offset_x_mm.is_finite() || !g.center_offset_y_mm.is_finite()
        || g.sod_mm <= 0.0 || g.object_to_detector_mm <= 0.0 || g.detector_width_mm <= 0.0
        || !matches!(g.rotation_direction.as_str(), "clockwise" | "counterclockwise")
        || g.measurement != "measured" {
        return Err("scan geometry is incomplete or invalid".into());
    }
    let first_step = manifest.frames[1].angle_deg - manifest.frames[0].angle_deg;
    if first_step.abs() < 1e-6 || (first_step.abs() * manifest.frames.len() as f64 - 360.0).abs() > 0.1
        || manifest.frames.windows(2).any(|pair| ((pair[1].angle_deg - pair[0].angle_deg) - first_step).abs() > 0.002) {
        return Err("FDK needs equally spaced full-circle angles without a duplicate endpoint".into());
    }
    Ok((manifest, digest))
}

fn verified_path(scan_dir: &Path, path: &str, bytes: u64, sha256: &str) -> Result<PathBuf, String> {
    let canonical_root = scan_dir.canonicalize().map_err(io_error)?;
    let candidate = Path::new(path);
    let candidate = if candidate.is_absolute() || candidate.is_file() {
        candidate.to_path_buf()
    } else {
        scan_dir.join(candidate)
    };
    let canonical = candidate.canonicalize().map_err(io_error)?;
    if !canonical.starts_with(canonical_root)
        || !canonical.extension().is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("nef")) {
        return Err("manifest frame path leaves the scan directory or is not NEF".into());
    }
    if hash_file(&canonical)? != (bytes, sha256.to_owned()) {
        return Err(format!("NEF size or SHA-256 mismatch: {}", canonical.display()));
    }
    Ok(canonical)
}

struct BlueImage { width: usize, height: usize, pixels: Vec<f32>, white: f32 }

fn decode_blue(path: &Path) -> Result<BlueImage, String> {
    let mut file = File::open(path).map_err(io_error)?;
    let raw = rawloader::decode(&mut file).map_err(|error| format!("NEF decode failed: {error}"))?;
    if raw.cpp != 1 || raw.cfa.width != 2 || raw.cfa.height != 2 {
        return Err("only single-plane 2x2 Bayer RAW is supported".into());
    }
    let data = match &raw.data {
        RawImageData::Integer(data) => data,
        RawImageData::Float(_) => return Err("floating RAW input is unsupported".into()),
    };
    let [top, right, bottom, left] = raw.crops;
    let visible_w = raw.width.checked_sub(left + right).ok_or("invalid RAW horizontal crop")?;
    let visible_h = raw.height.checked_sub(top + bottom).ok_or("invalid RAW vertical crop")?;
    if visible_w < 512 || visible_h < 512 || data.len() != raw.width * raw.height {
        return Err("unexpected RAW sensor dimensions".into());
    }
    if raw.whitelevels[2] == 0 { return Err("RAW blue white level is unavailable".into()); }
    let cfa = raw.cropped_cfa();
    let blue_positions: Vec<_> = (0..2).flat_map(|row| (0..2).map(move |col| (row, col)))
        .filter(|&(row, col)| cfa.color_at(row, col) == 2).collect();
    if blue_positions.len() != 1 { return Err("RAW CFA has no unique blue sample".into()); }
    let (blue_row, blue_col) = blue_positions[0];
    let height = (visible_h - blue_row + 1) / 2;
    let width = (visible_w - blue_col + 1) / 2;
    // The measured dark frame is subtracted after this stage, so retain the
    // original integer values instead of subtracting a metadata black level.
    let mut pixels = Vec::with_capacity(width * height);
    for row in 0..height {
        let start = (top + blue_row + row * 2) * raw.width + left + blue_col;
        for col in 0..width { pixels.push(data[start + col * 2] as f32); }
    }
    Ok(BlueImage { width, height, pixels, white: raw.whitelevels[2] as f32 })
}

#[derive(Clone, Debug, Serialize)]
struct ScreenMap { cx: f64, cy: f64, rx: f64, ry: f64, width: usize, height: usize }

fn find_screen(flat: &BlueImage, dark: &BlueImage) -> Result<ScreenMap, String> {
    if flat.width != dark.width || flat.height != dark.height { return Err("reference RAW dimensions changed".into()); }
    let width = flat.width;
    let height = flat.height;
    // Fit the luminous circular rim, not row/column counts of noisy pixels.
    // The latter can span the entire sensor even with a small central screen.
    // A deterministic robust fit rejects isolated hot pixels and the stand.
    // This calibrated path requires a near-frontoparallel circular screen;
    // insufficient angular support must fail rather than guess a full-frame ROI.
    let step = (width.min(height) / 500).max(1);
    let mut samples = Vec::new();
    for row in (0..height).step_by(step) {
        for col in (0..width).step_by(step) {
            samples.push((col as f64, row as f64, flat.pixels[row * width + col] - dark.pixels[row * width + col]));
        }
    }
    let mut values: Vec<f32> = samples.iter().map(|s| s.2).collect();
    values.sort_by(f32::total_cmp);
    let threshold = values[values.len() * 995 / 1000];
    if !threshold.is_finite() || threshold < 10.0 { return Err("flat screen signal is too weak to locate".into()); }
    let points: Vec<_> = samples.iter().filter(|s| s.2 > threshold).map(|s| (s.0, s.1)).collect();
    if points.len() < 40 { return Err("flat screen rim has insufficient support".into()); }
    let mut seed = 123u64;
    let mut pick = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); (seed >> 32) as usize % points.len() };
    let mut best = None;
    let mut best_count = 0;
    for _ in 0..1200 {
        let (a, b, c) = (points[pick()], points[pick()], points[pick()]);
        let (bx, by, cx, cy) = (b.0-a.0, b.1-a.1, c.0-a.0, c.1-a.1);
        let det = 2.0*(bx*cy-by*cx);
        if det.abs() < 1.0 { continue; }
        let (bb, cc) = (bx*bx+by*by, cx*cx+cy*cy);
        let x = a.0+(bb*cy-cc*by)/det;
        let y = a.1+(bx*cc-cx*bb)/det;
        let r = (x-a.0).hypot(y-a.1);
        if r < 100.0 || r > width.min(height) as f64*0.48
            || x-r < 1.0 || y-r < 1.0 || x+r >= (width-1) as f64 || y+r >= (height-1) as f64 { continue; }
        let tolerance = (r*0.015).max(3.0);
        let mut bins = [false; 24];
        let count = points.iter().filter(|&&(px,py)| {
            let matches = ((px-x).hypot(py-y)-r).abs() < tolerance;
            if matches { bins[(((py-y).atan2(px-x)+PI)/(2.0*PI)*24.0) as usize % 24] = true; }
            matches
        }).count();
        if count > best_count && bins.iter().filter(|b| **b).count() >= 18 {
            best_count = count;
            best = Some(ScreenMap {cx:x,cy:y,rx:r,ry:r,width,height});
        }
    }
    if best_count < points.len()/4 { return Err("flat screen rim is ambiguous; verify screen alignment and references".into()); }
    best.ok_or_else(|| "circular flat screen outline not found".into())
}

fn bilinear(image: &BlueImage, x: f64, y: f64) -> f32 {
    if x < 0.0 || y < 0.0 || x >= (image.width - 1) as f64 || y >= (image.height - 1) as f64 { return 0.0; }
    let col = x.floor() as usize;
    let row = y.floor() as usize;
    let tx = (x - col as f64) as f32;
    let ty = (y - row as f64) as f32;
    let i = row * image.width + col;
    let a = image.pixels[i] * (1.0 - tx) + image.pixels[i + 1] * tx;
    let b = image.pixels[i + image.width] * (1.0 - tx) + image.pixels[i + image.width + 1] * tx;
    a * (1.0 - ty) + b * ty
}

fn remap(image: &BlueImage, map: &ScreenMap, mirror_x: bool) -> Result<Vec<f32>, String> {
    remap_at_size(image,map,mirror_x,DETECTOR_N)
}

/// Materialize the entire circumscribing square before detector resampling.
/// One-pixel interpolation guard surrounds the 1% rim-fit padding.
fn crop_screen_square(image:&BlueImage,map:&ScreenMap)->Result<(BlueImage,usize,usize),String> {
    if image.width!=map.width || image.height!=map.height {return Err("RAW dimensions changed during scan".into());}
    let half=map.rx.max(map.ry)*SCREEN_CROP_SCALE;
    let left=(map.cx-half-1.).floor();let top=(map.cy-half-1.).floor();
    let side=(2.*half).ceil() as usize+3;
    if left<0. || top<0. || left as usize+side>image.width || top as usize+side>image.height {
        return Err("full screen square leaves RAW image; verify screen alignment".into());
    }
    let (left,top)=(left as usize,top as usize);
    let mut pixels=Vec::with_capacity(side*side);
    for row in top..top+side {pixels.extend_from_slice(&image.pixels[row*image.width+left..row*image.width+left+side]);}
    Ok((BlueImage {width:side,height:side,pixels,white:image.white},left,top))
}

fn remap_at_size(image: &BlueImage, map: &ScreenMap, mirror_x: bool,n:usize) -> Result<Vec<f32>, String> {
    let (cropped,left,top)=crop_screen_square(image,map)?;
    let mut output = vec![0f32; n * n];
    for row in 0..n {
        for col in 0..n {
            let mut sum = 0.0;
            for sy in 0..3 {
                for sx in 0..3 {
                    let u = (col as f64 + (sx as f64 + 0.5) / 3.0) / n as f64 * 2.0 - 1.0;
                    let v = (row as f64 + (sy as f64 + 0.5) / 3.0) / n as f64 * 2.0 - 1.0;
                    let physical_u = if mirror_x { -u } else { u };
                    sum += bilinear(&cropped, map.cx-left as f64 + physical_u * map.rx*SCREEN_CROP_SCALE,
                        map.cy-top as f64 - v * map.ry*SCREEN_CROP_SCALE);
                }
            }
            output[row * n + col] = sum / 9.0;
        }
    }
    Ok(output)
}

#[cfg(test)]
fn mean_reference(scan_dir: &Path, frames: &[ReferenceFrame], map: &ScreenMap,
    mirror_x: bool, progress: &Mutex<ReconstructionProgress>, completed: &mut usize) -> Result<Vec<f32>, String> {
    let mut mean = vec![0f32; DETECTOR_N * DETECTOR_N];
    for frame in frames {
        let path = verified_path(scan_dir, &frame.path, frame.bytes, &frame.sha256)?;
        let image = decode_blue(&path)?;
        let sampled = remap(&image, map, mirror_x)?;
        for (dst, src) in mean.iter_mut().zip(sampled) { *dst += src / REFERENCE_COUNT as f32; }
        *completed += 1;
        set_progress(progress, "running", (10 + *completed * 10 / (REFERENCE_COUNT * 2)) as u8, "处理校正帧");
    }
    Ok(mean)
}

fn write_floats(writer: &mut impl Write, values: &[f32]) -> Result<(), String> {
    for value in values { writer.write_all(&value.to_le_bytes()).map_err(io_error)?; }
    Ok(())
}

/// Materialize a separate cropped linear detector stack before reconstruction.
/// NEFs remain read-only; geometry is referenced to the fitted full screen rim.
fn prepare_projections(scan_dir: &Path, manifest: &Manifest, screen: &ScreenMap,
    dark: &preprocessing::ReferenceStats, flat: &preprocessing::ReferenceStats, root: &Path, progress: &Mutex<ReconstructionProgress>) -> Result<PathBuf, String> {
    fs::create_dir_all(root).map_err(io_error)?;
    // Invalidate a previous completion seal before replacing derived files.
    let seal = root.join("result.json");
    if seal.exists() { fs::remove_file(seal).map_err(io_error)?; }
    let mut crops = BufWriter::new(File::create(root.join("detector-crops.f32le")).map_err(io_error)?);
    let corrected_path = root.join("projections.f32le");
    let mut corrected = BufWriter::new(File::create(&corrected_path).map_err(io_error)?);
    let mut masks=BufWriter::new(File::create(root.join("detector-quality.bin")).map_err(io_error)?);
    let support_threshold=preprocessing::support_threshold(dark,flat);
    let mut quality = Vec::new();
    for frame in &manifest.frames {
        let path = verified_path(scan_dir, &frame.path, frame.bytes, &frame.sha256)?;
        let image = decode_blue(&path)?;
        let raw = remap(&image, screen, manifest.geometry.mirror_x)?;
        write_floats(&mut crops, &raw)?;
        let correction=preprocessing::correct(&raw,image.white,dark,flat,support_threshold,frame.index)
            .map_err(|error| {let _=write_json(&root.join("quality-failure.json"),&serde_json::json!({"index":frame.index,"error":error}));error})?;
        quality.push(correction.quality);
        masks.write_all(&correction.flags).map_err(io_error)?;
        write_floats(&mut corrected, &correction.projection)?;
        set_progress(progress,"running",(20+frame.index as usize*30/manifest.frames.len()) as u8,
            &format!("Crop and correct {}/{}",frame.index,manifest.frames.len()));
    }
    for writer in [&mut crops,&mut corrected] {
        writer.flush().map_err(io_error)?;
        writer.get_ref().sync_all().map_err(io_error)?;
    }
    masks.flush().map_err(io_error)?;
    masks.get_ref().sync_all().map_err(io_error)?;
    for (name, data) in [("dark-crop.f32le",&dark.mean),("flat-crop.f32le",&flat.mean),
        ("dark-variance.f32le",&dark.variance),("flat-variance.f32le",&flat.variance)] {
        let mut writer = BufWriter::new(File::create(root.join(name)).map_err(io_error)?);
        write_floats(&mut writer,data)?;
        writer.flush().map_err(io_error)?;
    }
    write_json(&root.join("preprocessing.json"), &serde_json::json!({
        "schemaVersion":CACHE_SCHEMA,"manifestSha256":hash_file(&scan_dir.join("manifest.json"))?.1,
        "screenBluePixels":screen,"detectorShape":[DETECTOR_N,DETECTOR_N],
        "projectionCount":manifest.frames.len(),"detectorWidthMm":manifest.geometry.detector_width_mm,
        "cropSquareBluePixels":{
            "left":(screen.cx-screen.rx.max(screen.ry)*SCREEN_CROP_SCALE-1.).floor(),
            "top":(screen.cy-screen.rx.max(screen.ry)*SCREEN_CROP_SCALE-1.).floor(),
            "side":(2.*screen.rx.max(screen.ry)*SCREEN_CROP_SCALE).ceil() as usize+3},
        "cropPaddingScale":SCREEN_CROP_SCALE,
        "sampledSquareWidthMm":manifest.geometry.detector_width_mm*SCREEN_CROP_SCALE,
        "mirrorX":manifest.geometry.mirror_x,"rowOrder":"physical detector v increases with row",
        "validRadiusFraction":1.0,"format":"little-endian float32; view,row,column",
        "noiseFloor":"max(1 ADU, sqrt(dark sample variance * (1 + 1/10)))",
        "smoothingSigmaPixels":0.7,"taperRadiusFractions":[0.98,1.0],
        "lowGainWarningThresholdAdu":support_threshold,"driftCorrectionApplied":false,
        "qualityFlags":{"0":"outside screen","1":"measured","2":"noise censored, not quantitative","3":"missing reference","4":"saturated","5":"low gain retained, not excluded"},
        "qualitySha256":hash_file(&root.join("detector-quality.bin"))?.1,
        "cropSha256":hash_file(&root.join("detector-crops.f32le"))?.1,
        "correctedSha256":hash_file(&corrected_path)?.1,"quality":quality,
        "note":"Full circumscribing square is copied before resampling/correction. Entire disk is retained, including lower half. Low gain is a warning, not a crop. Only the outermost 2% radius is tapered. Coverage is diagnostic and never a default preview clipping mask."
    }))?;
    Ok(corrected_path)
}

#[derive(Clone, Copy, Default)]
struct Complex { re: f64, im: f64 }
impl Complex {
    fn mul(self, other: Self) -> Self { Self { re: self.re * other.re - self.im * other.im, im: self.re * other.im + self.im * other.re } }
}

fn fft(values: &mut [Complex], inverse: bool) {
    let n = values.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 { j ^= bit; bit >>= 1; }
        j ^= bit;
        if i < j { values.swap(i, j); }
    }
    let mut len = 2;
    while len <= n {
        let angle = if inverse { 2.0 * PI / len as f64 } else { -2.0 * PI / len as f64 };
        let unit = Complex { re: angle.cos(), im: angle.sin() };
        for start in (0..n).step_by(len) {
            let mut w = Complex { re: 1.0, im: 0.0 };
            for i in 0..len / 2 {
                let u = values[start + i];
                let v = values[start + i + len / 2].mul(w);
                values[start + i] = Complex { re: u.re + v.re, im: u.im + v.im };
                values[start + i + len / 2] = Complex { re: u.re - v.re, im: u.im - v.im };
                w = w.mul(unit);
            }
        }
        len <<= 1;
    }
    if inverse { for value in values { value.re /= n as f64; value.im /= n as f64; } }
}

fn filter_projection(projection: &mut [f32], pitch: f64, sod: f64, sdd: f64,
    center_u: f64, center_v: f64) {
    let nfft = (2 * DETECTOR_N).next_power_of_two();
    let iso_pitch = pitch * sod / sdd;
    let mut frequency = vec![0f64; nfft];
    for (k, item) in frequency.iter_mut().enumerate() {
        let abs_k = k.min(nfft - k);
        let f = abs_k as f64 / (nfft as f64 * iso_pitch);
        let nyquist = 0.5 / iso_pitch;
        *item = f * 0.5 * (1.0 + (PI * f / nyquist).cos());
    }
    let mut buffer = vec![Complex::default(); nfft];
    for row in 0..DETECTOR_N {
        buffer.fill(Complex::default());
        let v = (row as f64 - (DETECTOR_N as f64 - 1.0) / 2.0) * pitch - center_v;
        for col in 0..DETECTOR_N {
            let u = (col as f64 - (DETECTOR_N as f64 - 1.0) / 2.0) * pitch - center_u;
            let xi = u * sod / sdd;
            let zeta = v * sod / sdd;
            buffer[col].re = projection[row * DETECTOR_N + col] as f64
                * sod / (sod * sod + xi * xi + zeta * zeta).sqrt();
        }
        fft(&mut buffer, false);
        for (value, weight) in buffer.iter_mut().zip(&frequency) { value.re *= weight; value.im *= weight; }
        fft(&mut buffer, true);
        for col in 0..DETECTOR_N { projection[row * DETECTOR_N + col] = buffer[col].re as f32; }
    }
}

fn sample_detector(image: &[f32], x: f64, y: f64) -> f32 {
    if x < 0.0 || y < 0.0 || x >= (DETECTOR_N - 1) as f64 || y >= (DETECTOR_N - 1) as f64 { return 0.0; }
    let col = x.floor() as usize;
    let row = y.floor() as usize;
    let tx = (x - col as f64) as f32;
    let ty = (y - row as f64) as f32;
    let i = row * DETECTOR_N + col;
    let a = image[i] * (1.0 - tx) + image[i + 1] * tx;
    let b = image[i + DETECTOR_N] * (1.0 - tx) + image[i + DETECTOR_N + 1] * tx;
    a * (1.0 - ty) + b * ty
}

fn backproject(volume: &mut [f32], coverage: &mut [u8], projection: &[f32],
    theta: f64, pitch: f64, voxel: f64, sod: f64, sdd: f64,
    center_u: f64, center_v: f64, angle_weight: f64) -> Result<(), String> {
    let (sin, cos) = theta.sin_cos();
    let half = (VOLUME_N as f64 - 1.0) / 2.0;
    let detector_half = (DETECTOR_N as f64 - 1.0) / 2.0;
    for y_idx in 0..VOLUME_N {
        let y = (y_idx as f64 - half) * voxel;
        for x_idx in 0..VOLUME_N {
            let x = (x_idx as f64 - half) * voxel;
            let along = sod + y * cos - x * sin;
            if along <= 0.0 { return Err("reconstruction volume intersects source plane".into()); }
            let magnification = sdd / along;
            let detector_x = (magnification * (x * cos + y * sin) + center_u) / pitch + detector_half;
            let weight = angle_weight * (sod / along).powi(2);
            for z_idx in 0..VOLUME_N {
                let z = (z_idx as f64 - half) * voxel;
                let detector_y = (magnification * z + center_v) / pitch + detector_half;
                let i = (z_idx * VOLUME_N + y_idx) * VOLUME_N + x_idx;
                if detector_x >= 0.0 && detector_y >= 0.0
                    && detector_x < (DETECTOR_N - 1) as f64 && detector_y < (DETECTOR_N - 1) as f64 {
                    volume[i] += sample_detector(projection, detector_x, detector_y) * weight as f32;
                    let ux = (detector_x - detector_half) / (DETECTOR_N as f64 / 2.0);
                    let vy = (detector_y - detector_half) / (DETECTOR_N as f64 / 2.0);
                    if (ux * ux + vy * vy)*SCREEN_CROP_SCALE.powi(2) > 1.0 { coverage[i] = 0; }
                    if projection.len()==2*DETECTOR_N*DETECTOR_N
                        && sample_detector(&projection[DETECTOR_N*DETECTOR_N..],detector_x,detector_y)<0.999 {
                        coverage[i]=0;
                    }
                } else { coverage[i] = 0; }
            }
        }
    }
    Ok(())
}

/// Low-resolution simultaneous iterative reconstruction. The nearest-detector
/// forward scatter and backward gather are an exact transpose pair; each view
/// uses measured, unfiltered line integrals rather than an FDK image prior.
fn sirt_detector(x: f64, y: f64, z: f64, sin: f64, cos: f64,
    sod: f64, sdd: f64, pitch: f64, center_u: f64, center_v: f64) -> Option<usize> {
    let along = sod + y * cos - x * sin;
    if along <= 0.0 { return None; }
    let magnification = sdd / along;
    let half = (SIRT_N as f64 - 1.0) * 0.5;
    let u = (magnification * (x * cos + y * sin) + center_u) / pitch + half;
    let v = (magnification * z + center_v) / pitch + half;
    if u < 0.0 || v < 0.0 || u >= SIRT_N as f64 || v >= SIRT_N as f64 { return None; }
    let u = u.round().clamp(0.0, SIRT_N as f64 - 1.0) as usize;
    let v = v.round().clamp(0.0, SIRT_N as f64 - 1.0) as usize;
    let du = (u as f64 - half) / (SIRT_N as f64 / 2.0);
    let dv = (v as f64 - half) / (SIRT_N as f64 / 2.0);
    ((du * du + dv * dv)*SCREEN_CROP_SCALE.powi(2) <= 1.0).then_some(v * SIRT_N + u)
}

fn downsample_sirt_projection(projection: &[f32]) -> Vec<f32> {
    let factor = DETECTOR_N / SIRT_N;
    let mut result = vec![0.0; SIRT_N * SIRT_N];
    for y in 0..SIRT_N {
        for x in 0..SIRT_N {
            let mut sum = 0.0;
            for dy in 0..factor {
                for dx in 0..factor {
                    sum += projection[(y * factor + dy) * DETECTOR_N + x * factor + dx];
                }
            }
            result[y * SIRT_N + x] = sum / (factor * factor) as f32;
        }
    }
    result
}

fn reconstruct_sirt(frames: &[(f64, Vec<f32>)], sod: f64, sdd: f64,
    detector_width: f64, center_u: f64, center_v: f64,
    progress: &Mutex<ReconstructionProgress>) -> Result<(Vec<f32>, Vec<u8>, f64), String> {
    let pitch = detector_width * SCREEN_CROP_SCALE / SIRT_N as f64;
    let voxel = detector_width * sod / sdd / SIRT_N as f64;
    if voxel * SIRT_N as f64 / 2.0 >= sod {
        return Err("scan geometry puts SIRT volume at source".into());
    }
    let voxels = SIRT_N * SIRT_N * SIRT_N;
    let mut positions = Vec::with_capacity(voxels);
    let half = (SIRT_N as f64 - 1.0) * 0.5;
    for z in 0..SIRT_N {
        for y in 0..SIRT_N {
            for x in 0..SIRT_N {
                positions.push(((x as f64 - half) * voxel,
                    (y as f64 - half) * voxel, (z as f64 - half) * voxel));
            }
        }
    }
    let mut volume = vec![0.0f32; voxels];
    let mut coverage = vec![1u8; voxels];
    let mut column_norm = vec![0.0f32; voxels];
    let mut residual_backprojection = vec![0.0f32; voxels];
    let ray_step = voxel as f32;
    for iteration in 0..SIRT_ITERATIONS {
        residual_backprojection.fill(0.0);
        for (view, (angle, measured)) in frames.iter().enumerate() {
            let (sin, cos) = angle.sin_cos();
            let mut forward = vec![0.0f32; SIRT_N * SIRT_N];
            let mut row_norm = vec![0.0f32; SIRT_N * SIRT_N];
            for (i, &(x, y, z)) in positions.iter().enumerate() {
                if let Some(detector) = sirt_detector(x, y, z, sin, cos,
                    sod, sdd, pitch, center_u, center_v) {
                    forward[detector] += volume[i] * ray_step;
                    row_norm[detector] += ray_step;
                } else if iteration == 0 { coverage[i] = 0; }
            }
            for pixel in 0..forward.len() {
                forward[pixel] = if row_norm[pixel] > 0.0 {
                    (measured[pixel] - forward[pixel]) / row_norm[pixel]
                } else { 0.0 };
            }
            for (i, &(x, y, z)) in positions.iter().enumerate() {
                if let Some(detector) = sirt_detector(x, y, z, sin, cos,
                    sod, sdd, pitch, center_u, center_v) {
                    residual_backprojection[i] += forward[detector] * ray_step;
                    if iteration == 0 { column_norm[i] += ray_step; }
                }
            }
            let fraction = (iteration * frames.len() + view + 1) as f32
                / (SIRT_ITERATIONS * frames.len()) as f32;
            set_progress(progress, "running", (55.0 + 39.0 * fraction) as u8,
                &format!("SIRT {}/{}", iteration + 1, SIRT_ITERATIONS));
        }
        for i in 0..voxels {
            if column_norm[i] > 0.0 {
                volume[i] = (volume[i] + 0.8 * residual_backprojection[i] / column_norm[i]).max(0.0);
            }
        }
    }
    Ok((volume, coverage, voxel))
}

/// A real compute path: the CPU performs the physical ramp filter, and this
/// shader performs the full 3-D FDK accumulation on a selected GPU adapter.
struct GpuFdk {
    name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    projection: wgpu::Buffer,
    parameters: wgpu::Buffer,
    volume: wgpu::Buffer,
    coverage: wgpu::Buffer,
    volume_bytes: u64,
}

impl GpuFdk {
    fn new(adapter: &wgpu::Adapter) -> Result<Self, String> {
        let info = adapter.get_info();
        if !matches!(info.device_type, wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu) {
            return Err("software or unknown GPU adapter".into());
        }
        let volume_bytes = (VOLUME_N * VOLUME_N * VOLUME_N * 4) as u64;
        let limits = adapter.limits();
        if (limits.max_storage_buffer_binding_size as u64) < volume_bytes
            || limits.max_buffer_size < volume_bytes
            || limits.max_compute_workgroups_per_dimension < (VOLUME_N / 4) as u32 {
            return Err("GPU storage or dispatch limit is below the required volume".into());
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Micro-CT FDK"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        })).map_err(|error| format!("GPU device request failed: {error}"))?;
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Micro-CT FDK backprojection"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("fdk.wgsl"))),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Micro-CT FDK pipeline"),
            layout: None,
            module: &module,
            entry_point: Some("backproject"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            return Err(format!("GPU FDK shader validation failed: {error}"));
        }
        let projection = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FDK filtered projection"),
            size: (DETECTOR_N * DETECTOR_N * 8) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let parameters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FDK geometry"), size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let volume = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FDK volume"), size: volume_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut coverage_bytes = vec![0u8; volume_bytes as usize];
        for item in coverage_bytes.chunks_exact_mut(4) { item[0] = 1; }
        let coverage = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FDK geometric coverage"), contents: &coverage_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FDK bindings"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: projection.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: volume.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: coverage.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: parameters.as_entire_binding() },
            ],
        });
        Ok(Self { name: format!("{} ({:?})", info.name, info.backend), device, queue,
            pipeline, bind_group, projection, parameters, volume, coverage, volume_bytes })
    }

    fn project(&self, filtered: &[f32], theta: f64, pitch: f64, voxel: f64,
        sod: f64, sdd: f64, center_u: f64, center_v: f64, angle_weight: f64) -> Result<Duration, String> {
        let start = Instant::now();
        let mut pixels = Vec::with_capacity(filtered.len() * 4);
        for value in filtered { pixels.extend_from_slice(&value.to_le_bytes()); }
        if filtered.len()==DETECTOR_N*DETECTOR_N {
            for _ in 0..DETECTOR_N*DETECTOR_N {pixels.extend_from_slice(&1f32.to_le_bytes());}
        }
        self.queue.write_buffer(&self.projection, 0, &pixels);
        let values = [theta as f32, pitch as f32, voxel as f32, sod as f32, sdd as f32,
            center_u as f32, center_v as f32, angle_weight as f32];
        let mut parameters = Vec::with_capacity(48);
        for value in values { parameters.extend_from_slice(&value.to_le_bytes()); }
        for value in [VOLUME_N as u32, DETECTOR_N as u32, 0, 0] {
            parameters.extend_from_slice(&value.to_le_bytes());
        }
        self.queue.write_buffer(&self.parameters, 0, &parameters);
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("FDK view accumulation"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("FDK backprojection"), timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups((VOLUME_N / 4) as u32, (VOLUME_N / 4) as u32, (VOLUME_N / 4) as u32);
        }
        self.queue.submit(Some(encoder.finish()));
        self.device.poll(wgpu::PollType::wait())
            .map_err(|error| format!("GPU dispatch failed: {error}"))?;
        Ok(start.elapsed())
    }

    fn readback(&self) -> Result<(Vec<f32>, Vec<u8>, Duration), String> {
        let start = Instant::now();
        let volume_bytes = self.read_buffer(&self.volume)?;
        let coverage_bytes = self.read_buffer(&self.coverage)?;
        let volume = volume_bytes.chunks_exact(4).map(|chunk| {
            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
        }).collect();
        let coverage = coverage_bytes.chunks_exact(4).map(|chunk| u8::from(
            u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) != 0
        )).collect();
        Ok((volume, coverage, start.elapsed()))
    }

    fn read_buffer(&self, source: &wgpu::Buffer) -> Result<Vec<u8>, String> {
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("FDK readback"), size: self.volume_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("FDK download"),
        });
        encoder.copy_buffer_to_buffer(source, 0, &staging, 0, self.volume_bytes);
        self.queue.submit(Some(encoder.finish()));
        let (sender, receiver) = mpsc::channel();
        staging.slice(..).map_async(wgpu::MapMode::Read, move |result| { let _ = sender.send(result); });
        self.device.poll(wgpu::PollType::wait())
            .map_err(|error| format!("GPU readback failed: {error}"))?;
        receiver.recv().map_err(|_| "GPU readback callback was lost".to_owned())?
            .map_err(|error| format!("GPU readback mapping failed: {error}"))?;
        let bytes = staging.slice(..).get_mapped_range().to_vec();
        staging.unmap();
        Ok(bytes)
    }
}

fn gpu_matches_cpu(cpu: &[f32], gpu: &[f32], coverage: &[u8]) -> bool {
    if cpu.len() != gpu.len() || cpu.len() != coverage.len() { return false; }
    // An integer stride of 8192 on a 256^3 grid only visits x=0. Use a
    // deterministic coprime walk so all three axes are represented.
    let stride = 0x9e37_79b1usize;
    let mut tested = 0usize;
    let mut max_signal = 0f32;
    let mut max_error = 0f32;
    for sample in 0..2048.min(cpu.len()) {
        let i = sample.wrapping_mul(stride) % cpu.len();
        if coverage[i] == 0 { continue; }
        if !cpu[i].is_finite() || !gpu[i].is_finite() { return false; }
        tested += 1;
        max_signal = max_signal.max(cpu[i].abs());
        max_error = max_error.max((cpu[i] - gpu[i]).abs());
    }
    tested >= 16 && max_error <= 0.05 * max_signal + 1e-5
}

fn reconstruct(request: &ReconstructionRequest, progress: &Mutex<ReconstructionProgress>) -> Result<ReconstructionResult, String> {
    reconstruct_inner(request, progress, true, None)
}

fn reconstruct_inner(request: &ReconstructionRequest, progress: &Mutex<ReconstructionProgress>,
    allow_gpu: bool, fallback_reason: Option<String>) -> Result<ReconstructionResult, String> {
    if let Some(existing) = cached_result(&request.scan_dir, request.method)? { return Ok(existing); }
    if request.method == ReconstructionMethod::Cgls { return Err("method has no validated backend".into()); }
    set_progress(progress, "running", 1, "核对扫描数据");
    let (manifest, manifest_sha256) = read_manifest(&request.scan_dir)?;
    if request.method == ReconstructionMethod::Sirt
        && !sirt_feasible(manifest.projection_count) {
        return Err("SIRT CPU workload exceeds the interactive limit".into());
    }
    let scan_dir = &request.scan_dir;
    let first_dark = &manifest.references.pre_dark[0];
    let first_flat = &manifest.references.pre_flat[0];
    let dark_path = verified_path(scan_dir, &first_dark.path, first_dark.bytes, &first_dark.sha256)?;
    let flat_path = verified_path(scan_dir, &first_flat.path, first_flat.bytes, &first_flat.sha256)?;
    let dark_image = decode_blue(&dark_path)?;
    let flat_image = decode_blue(&flat_path)?;
    let screen = find_screen(&flat_image, &dark_image)?;
    set_progress(progress, "running", 10, "处理校正帧");
    let mut completed = 0;
    let pre_dark = preprocessing::references(scan_dir, &manifest.references.pre_dark, &screen,
        manifest.geometry.mirror_x, progress, &mut completed)?;
    let pre_flat = preprocessing::references(scan_dir, &manifest.references.pre_flat, &screen,
        manifest.geometry.mirror_x, progress, &mut completed)?;
    let prepared = prepare_projections(scan_dir, &manifest, &screen, &pre_dark, &pre_flat,
        &cache_dir(scan_dir, request.method), progress)?;
    let stack_bytes=fs::read(&prepared).map_err(io_error)?;
    let stack:Vec<f32>=stack_bytes.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
    drop(stack_bytes);
    let angles:Vec<f64>=manifest.frames.iter().map(|frame|if manifest.geometry.rotation_direction=="counterclockwise" {
        frame.angle_deg.to_radians()
    }else{-frame.angle_deg.to_radians()}).collect();
    let mut projections = BufReader::new(File::open(prepared).map_err(io_error)?);
    let mut quality_masks=BufReader::new(File::open(cache_dir(scan_dir,request.method).join("detector-quality.bin")).map_err(io_error)?);
    let sod = manifest.geometry.sod_mm;
    let sdd = sod + manifest.geometry.object_to_detector_mm;
    let pitch = manifest.geometry.detector_width_mm * SCREEN_CROP_SCALE / DETECTOR_N as f64;
    let axis=preprocessing::estimate_axis(&stack,&angles,pitch,sdd,
        manifest.geometry.center_offset_x_mm,manifest.geometry.center_offset_y_mm);
    drop(stack);
    write_json(&cache_dir(scan_dir,request.method).join("calibration.json"),&axis)?;
    let center_u = axis.applied_offset_mm;
    let center_v = manifest.geometry.center_offset_y_mm;
    let voxel = manifest.geometry.detector_width_mm * sod / sdd / VOLUME_N as f64;
    if voxel * VOLUME_N as f64 / 2.0 >= sod { return Err("scan geometry puts reconstruction volume at source".into()); }
    let mut volume = vec![0f32; VOLUME_N * VOLUME_N * VOLUME_N];
    let mut coverage = vec![1u8; volume.len()];
    let step = (manifest.frames[1].angle_deg - manifest.frames[0].angle_deg).to_radians().abs();
    let angle_weight = step / 2.0;
    let mut sirt_frames = Vec::new();
    let gpu_instance = (allow_gpu && request.method == ReconstructionMethod::Fdk)
        .then(wgpu::Instance::default);
    let mut selected_gpu: Option<GpuFdk> = None;
    let mut selection = BackendSelection {
        name: "cpu".into(), cpu_estimated_ms: 0, gpu_estimated_ms: None,
        reason: fallback_reason.unwrap_or_else(|| "no validated GPU candidate".into()),
    };
    for (index, frame) in manifest.frames.iter().enumerate() {
        let mut bytes = vec![0u8; DETECTOR_N * DETECTOR_N * 4];
        projections.read_exact(&mut bytes).map_err(io_error)?;
        let mut projection: Vec<f32> = bytes.chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        let mut flags=vec![0u8;DETECTOR_N*DETECTOR_N];
        quality_masks.read_exact(&mut flags).map_err(io_error)?;
        let angle = if manifest.geometry.rotation_direction == "counterclockwise" {
            frame.angle_deg.to_radians()
        } else { -frame.angle_deg.to_radians() };
        if request.method == ReconstructionMethod::Sirt {
            sirt_frames.push((angle, downsample_sirt_projection(&projection)));
            set_progress(progress, "running", (50 + (index * 5 / manifest.frames.len())) as u8,
                &format!("读取投影 {}/{}", index + 1, manifest.frames.len()));
            continue;
        }
        set_progress(progress, "running", (50 + (index * 45 / manifest.frames.len())) as u8,
            &format!("重构 {}/{}", index + 1, manifest.frames.len()));
        filter_projection(&mut projection, pitch, sod, sdd, center_u, center_v);
        projection.extend(flags.iter().map(|f|if matches!(*f,1|2|5) {1.}else{0.}));
        if index == 0 {
            let started = Instant::now();
            backproject(&mut volume, &mut coverage, &projection, angle, pitch, voxel,
                sod, sdd, center_u, center_v, angle_weight)?;
            let cpu_estimate = estimated_duration(started.elapsed(), manifest.frames.len());
            selection.cpu_estimated_ms = milliseconds(cpu_estimate);
            if let Some(instance) = &gpu_instance {
                let mut best: Option<(GpuFdk, Duration)> = None;
                for adapter in instance.enumerate_adapters(wgpu::Backends::all()) {
                    let candidate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| GpuFdk::new(&adapter)));
                    let Ok(Ok(candidate)) = candidate else { continue; };
                    let trial = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let dispatch = candidate.project(&projection, angle, pitch, voxel,
                            sod, sdd, center_u, center_v, angle_weight)?;
                        let (sample, mask, readback) = candidate.readback()?;
                        if !gpu_matches_cpu(&volume, &sample, &mask) || coverage!=mask {
                            return Err("GPU numerical check disagreed with CPU FDK".to_owned());
                        }
                        Ok::<_, String>(estimated_duration(dispatch, manifest.frames.len()) + readback)
                    }));
                    let Ok(Ok(estimate)) = trial else { continue; };
                    if best.as_ref().is_none_or(|(_, previous)| estimate < *previous) {
                        best = Some((candidate, estimate));
                    }
                }
                if let Some((candidate, gpu_estimate)) = best {
                    selection.gpu_estimated_ms = Some(milliseconds(gpu_estimate));
                    if gpu_beats_cpu(cpu_estimate, gpu_estimate) {
                        selection.name = format!("gpu:{}", candidate.name);
                        selection.reason = "GPU measured projection plus readback is at least 15% faster".into();
                        selected_gpu = Some(candidate);
                    } else {
                        selection.reason = "GPU estimate is within 15% of CPU or slower".into();
                    }
                }
            }
        } else if let Some(gpu) = &selected_gpu {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                gpu.project(&projection, angle, pitch, voxel, sod, sdd, center_u, center_v, angle_weight)
            }));
            if !matches!(result, Ok(Ok(_))) {
                set_progress(progress, "running", 35, "GPU 失败，改用 CPU 重算");
                drop(selected_gpu.take());
                drop(volume);
                drop(coverage);
                return reconstruct_inner(request, progress, false,
                    Some(format!("GPU failed at view {}; full CPU recomputation", index + 1)));
            }
        } else {
            backproject(&mut volume, &mut coverage, &projection, angle, pitch, voxel,
                sod, sdd, center_u, center_v, angle_weight)?;
        }
    }
    if request.method == ReconstructionMethod::Sirt {
        let started = Instant::now();
        let (sirt_volume, sirt_coverage, sirt_voxel) = reconstruct_sirt(&sirt_frames,
            sod, sdd, manifest.geometry.detector_width_mm, center_u, center_v, progress)?;
        if sirt_volume.iter().any(|value| !value.is_finite()) {
            return Err("non-finite SIRT reconstruction volume".into());
        }
        selection.name = "cpu".into();
        selection.cpu_estimated_ms = milliseconds(started.elapsed());
        selection.reason = "CPU only: five-iteration, matched-operator 64^3 SIRT; recorded duration is actual iterative time".into();
        set_progress(progress, "running", 96, "保存结果");
        return persist_result(scan_dir, &manifest, manifest_sha256, &sirt_volume,
            &sirt_coverage, sirt_voxel, request.method, &selection);
    }
    if let Some(gpu) = &selected_gpu {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gpu.readback()));
        match result {
            Ok(Ok((gpu_volume, gpu_coverage, _))) => {
                volume = gpu_volume;
                coverage = gpu_coverage;
            }
            _ => {
                set_progress(progress, "running", 35, "GPU 读取失败，改用 CPU 重算");
                drop(selected_gpu.take());
                drop(volume);
                drop(coverage);
                return reconstruct_inner(request, progress, false,
                    Some("GPU final readback failed; full CPU recomputation".into()));
            }
        }
    }
    if volume.iter().any(|value| !value.is_finite()) { return Err("non-finite reconstruction volume".into()); }
    set_progress(progress, "running", 96, "保存结果");
    persist_result(scan_dir, &manifest, manifest_sha256, &volume, &coverage,
        voxel, request.method, &selection)
}

/// Deliberately has no coverage argument: partial angular support must never
/// silently remove anatomy from the default display.
fn full_volume_preview(volume:&[f32],n:usize,preview_n:usize,lo:f32,hi:f32)->Result<Vec<u8>,String> {
    if preview_n==0 || n%preview_n!=0 || volume.len()!=n.pow(3) || !lo.is_finite() || !hi.is_finite() || hi<=lo {
        return Err("invalid preview shape or window".into());
    }
    let factor=n/preview_n;
    let mut preview=vec![0u8;preview_n.pow(3)];
    for z in 0..preview_n {for y in 0..preview_n {for x in 0..preview_n {
        let mut sum=0f32;
        for dz in 0..factor {for dy in 0..factor {for dx in 0..factor {
            sum+=volume[((z*factor+dz)*n+y*factor+dy)*n+x*factor+dx];
        }}}
        preview[(z*preview_n+y)*preview_n+x]=((sum/factor.pow(3) as f32-lo)/(hi-lo)*255.).clamp(0.,255.) as u8;
    }}}
    Ok(preview)
}

fn persist_result(scan_dir: &Path, manifest: &Manifest, manifest_sha256: String,
    volume: &[f32], coverage: &[u8], voxel: f64, method: ReconstructionMethod,
    selection: &BackendSelection) -> Result<ReconstructionResult, String> {
    let (volume_n, preview_n) = match method {
        ReconstructionMethod::Fdk => (VOLUME_N, PREVIEW_N),
        ReconstructionMethod::Sirt => (SIRT_N, SIRT_N),
        ReconstructionMethod::Cgls => return Err("method has no validated backend".into()),
    };
    if volume.len() != volume_n.pow(3) || coverage.len() != volume.len() {
        return Err("reconstruction volume shape is invalid".into());
    }
    if hash_file(&scan_dir.join("manifest.json"))?.1 != manifest_sha256 {
        return Err("scan manifest changed during reconstruction".into());
    }
    let root = cache_dir(scan_dir, method);
    fs::create_dir_all(&root).map_err(io_error)?;
    let seal = root.join("result.json");
    if seal.exists() { fs::remove_file(&seal).map_err(io_error)?; }
    let volume_path = root.join("volume.f32le");
    let preview_path = root.join("preview.bin");
    let coverage_path = root.join("coverage.bin");
    let mut writer = BufWriter::new(File::create(&volume_path).map_err(io_error)?);
    for value in volume { writer.write_all(&value.to_le_bytes()).map_err(io_error)?; }
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)?;
    write_bytes_durable(&coverage_path, coverage)?;
    // Coverage describes all-angle support, not the display domain. Applying
    // its intersection here used to cut away the entire obstructed lower half.
    let mut finite: Vec<f32> = volume.iter().copied().filter(|v|v.is_finite()).collect();
    if finite.len() < volume.len() / 100 { return Err("usable reconstruction coverage is too small".into()); }
    finite.sort_by(f32::total_cmp);
    let window_min = 0.0;
    let window_max = finite[finite.len() * 995 / 1000].max(window_min + 1e-6);
    let factor = volume_n / preview_n;
    if factor == 0 || factor * preview_n != volume_n {
        return Err("preview grid does not divide scientific volume".into());
    }
    let preview=full_volume_preview(volume,volume_n,preview_n,window_min,window_max)?;
    write_bytes_durable(&preview_path, &preview)?;
    let volume_sha256 = hash_file(&volume_path)?.1;
    let preview_sha256 = hash_file(&preview_path)?.1;
    let coverage_sha256 = hash_file(&coverage_path)?.1;
    let shape = [volume_n; 3];
    let spacing_mm = [voxel; 3];
    let preview_shape = [preview_n; 3];
    let preview_spacing_mm = [voxel * factor as f64; 3];
    write_json(&root.join("preview.json"), &PreviewMetadata {
        shape: preview_shape, spacing_mm: preview_spacing_mm,
        window_min, window_max, sha256: preview_sha256.clone(),
    })?;
    let seal_pending = root.join("result.json.pending");
    write_json(&seal_pending, &CacheMetadata {
        schema_version: CACHE_SCHEMA, method, manifest_sha256: manifest_sha256.clone(),
        shape, spacing_mm, preview_shape, preview_spacing_mm,
        volume_sha256, preview_sha256, coverage_sha256,
        preprocessing_sha256:hash_file(&root.join("preprocessing.json"))?.1,
        calibration_sha256:hash_file(&root.join("calibration.json"))?.1,
        backend: selection.name.clone(),
        cpu_estimated_ms: selection.cpu_estimated_ms,
        gpu_estimated_ms: selection.gpu_estimated_ms,
        backend_selection: selection.reason.clone(),
        geometry: Geometry {
            sod_mm: manifest.geometry.sod_mm,
            object_to_detector_mm: manifest.geometry.object_to_detector_mm,
            detector_width_mm: manifest.geometry.detector_width_mm,
            center_offset_x_mm: manifest.geometry.center_offset_x_mm,
            center_offset_y_mm: manifest.geometry.center_offset_y_mm,
            rotation_direction: manifest.geometry.rotation_direction.clone(),
            mirror_x: manifest.geometry.mirror_x,
            measurement: manifest.geometry.measurement.clone(),
        },
        note: format!("{}; geometry is recorded acquisition geometry, applied axis correction is in calibration.json; full volume preview, coverage is diagnostic only and never clips display; low-gain/censored/partially covered values remain approximate; signed volume is preserved; spacing is computational sampling, not measured resolution",
            if method == ReconstructionMethod::Fdk { "Circular cone-beam FDK approximation" }
            else { "Five-iteration nonnegative SIRT at 64^3 with matched nearest-detector operators" }),
    })?;
    if hash_file(&scan_dir.join("manifest.json"))?.1 != manifest_sha256 {
        return Err("scan manifest changed before cache publication".into());
    }
    fs::rename(&seal_pending, &seal).map_err(io_error)?;
    Ok(ReconstructionResult { method, cache_path: root, volume_path, preview_path })
}

fn write_bytes_durable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let file = OpenOptions::new().create(true).truncate(true).write(true).open(path).map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(bytes).map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let file = OpenOptions::new().create(true).truncate(true).write(true).open(path).map_err(io_error)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)?;
    writer.get_ref().sync_all().map_err(io_error)
}

fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = BufReader::new(File::open(path).map_err(io_error)?);
    let mut hasher = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(io_error)?;
        if n == 0 { break; }
        count += n as u64;
        hasher.update(&buffer[..n]);
    }
    Ok((count, format!("{:x}", hasher.finalize())))
}

fn io_error(error: std::io::Error) -> String { error.to_string() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_square_crop_contains_all_four_screen_extremes() {
        let map=ScreenMap {cx:503.3,cy:401.7,rx:170.,ry:170.,width:1000,height:800};
        let mut image=BlueImage {width:1000,height:800,pixels:vec![0.;800000],white:16000.};
        for (x,y) in [(503,231),(503,572),(333,402),(674,402)] {image.pixels[y*1000+x]=123.;}
        let (crop,left,top)=crop_screen_square(&image,&map).unwrap();
        assert_eq!(crop.width,crop.height);
        assert!(crop.width<350);
        assert_eq!(crop.pixels.iter().filter(|v|**v==123.).count(),4);
        assert!((left as f64)<map.cx-map.rx && (top as f64)<map.cy-map.ry);
        assert!((left+crop.width) as f64>map.cx+map.rx && (top+crop.height) as f64>map.cy+map.ry);
    }

    #[test]
    fn preview_keeps_lower_and_upper_halves_without_coverage_clipping() {
        let mut volume=vec![0.25;64];volume[32..].fill(0.75);
        let preview=full_volume_preview(&volume,4,2,0.,1.).unwrap();
        assert!(preview[..4].iter().all(|v|*v==63));
        assert!(preview[4..].iter().all(|v|*v==191));
        assert_eq!(volume[0],0.25); // Display leaves scientific data untouched.
    }

    #[test]
    #[ignore]
    fn inspect_full_square_crops() {
        let scan=PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let out=PathBuf::from(std::env::var("CT_DIAGNOSTIC_DIR").expect("CT_DIAGNOSTIC_DIR"));
        fs::create_dir_all(&out).unwrap();
        let (manifest,_)=read_manifest(&scan).unwrap();
        let load=|f:&ReferenceFrame|decode_blue(&verified_path(&scan,&f.path,f.bytes,&f.sha256).unwrap()).unwrap();
        let d=load(&manifest.references.pre_dark[0]);let f=load(&manifest.references.pre_flat[0]);
        let map=find_screen(&f,&d).unwrap();
        let mut frames=vec![("dark".to_owned(),d),("flat".to_owned(),f)];
        for frame in manifest.frames.iter().step_by(90) {
            frames.push((format!("view-{}",frame.index),decode_blue(&verified_path(&scan,&frame.path,frame.bytes,&frame.sha256).unwrap()).unwrap()));
        }
        for (name,image) in frames {
            let (crop,left,top)=crop_screen_square(&image,&map).unwrap();
            let mut writer=BufWriter::new(File::create(out.join(format!("{name}-square.f32le"))).unwrap());
            write_floats(&mut writer,&crop.pixels).unwrap();writer.flush().unwrap();
            write_json(&out.join("square.json"),&serde_json::json!({"left":left,"top":top,"side":crop.width,"screen":map})).unwrap();
        }
    }

    #[test]
    fn screen_fit_rejects_full_sensor_noise_and_ignores_hot_pixels() {
        let (width,height)=(1000,800);
        let dark=BlueImage {width,height,pixels:vec![100.0;width*height],white:16000.0};
        let mut flat=BlueImage {width,height,pixels:dark.pixels.clone(),white:16000.0};
        let mut seed=7u64;
        for y in 0..height { for x in 0..width {
            seed=seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let noise=((seed>>32)%101) as f32-50.0;
            let radius=(x as f64-510.0).hypot(y as f64-420.0);
            flat.pixels[y*width+x]+=noise+if (radius-170.0).abs()<2.0 {2000.0} else if radius<170.0 {200.0} else {0.0};
            if (x*31+y*13)%997==0 {flat.pixels[y*width+x]=15000.0;}
        }}
        let map=find_screen(&flat,&dark).unwrap();
        assert!((map.cx-510.0).abs()<4.0 && (map.cy-420.0).abs()<4.0);
        assert!((map.rx-170.0).abs()<4.0);
        assert!(find_screen(&dark,&dark).is_err());
        let cropped=remap(&flat,&map,false).unwrap();
        assert!(cropped[128*DETECTOR_N+128]>200.0);
    }

    #[test]
    #[ignore]
    fn inspect_reference_statistics() {
        let scan=PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let out=PathBuf::from(std::env::var("CT_DIAGNOSTIC_DIR").expect("CT_DIAGNOSTIC_DIR"));
        fs::create_dir_all(&out).unwrap();
        let (manifest,_)=read_manifest(&scan).unwrap();
        let load=|f:&ReferenceFrame| decode_blue(&verified_path(&scan,&f.path,f.bytes,&f.sha256).unwrap()).unwrap();
        let d=load(&manifest.references.pre_dark[0]);
        let f=load(&manifest.references.pre_flat[0]);
        let map=find_screen(&f,&d).unwrap();
        for (name,frames) in [("dark",&manifest.references.pre_dark),("flat",&manifest.references.pre_flat)] {
            let mut stack=BufWriter::new(File::create(out.join(format!("{name}-stack.f32le"))).unwrap());
            let mut mean=vec![0f32;f.pixels.len()];
            for frame in frames {
                let image=load(frame);
                for (dst,src) in mean.iter_mut().zip(&image.pixels) {*dst+=*src/frames.len() as f32;}
                write_floats(&mut stack,&remap(&image,&map,manifest.geometry.mirror_x).unwrap()).unwrap();
            }
            stack.flush().unwrap();
            let mut writer=BufWriter::new(File::create(out.join(format!("{name}-mean-blue.f32le"))).unwrap());
            write_floats(&mut writer,&mean).unwrap();writer.flush().unwrap();
        }
        write_json(&out.join("screen.json"),&map).unwrap();
    }

    #[test]
    #[ignore]
    fn inspect_detector_sampling() {
        let scan=PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let out=PathBuf::from(std::env::var("CT_DIAGNOSTIC_DIR").expect("CT_DIAGNOSTIC_DIR"));
        let (manifest,_)=read_manifest(&scan).unwrap();
        let load=|f:&ReferenceFrame| decode_blue(&verified_path(&scan,&f.path,f.bytes,&f.sha256).unwrap()).unwrap();
        let d=load(&manifest.references.pre_dark[0]);let f=load(&manifest.references.pre_flat[0]);
        let map=find_screen(&f,&d).unwrap();
        for (name,frames) in [("dark",&manifest.references.pre_dark),("flat",&manifest.references.pre_flat)] {
            let mut writer=BufWriter::new(File::create(out.join(format!("{name}-512.f32le"))).unwrap());
            for frame in frames {write_floats(&mut writer,&remap_at_size(&load(frame),&map,manifest.geometry.mirror_x,512).unwrap()).unwrap();}
            writer.flush().unwrap();
        }
        let mut writer=BufWriter::new(File::create(out.join("crops-512.f32le")).unwrap());
        for frame in manifest.frames.iter().step_by(5) {
            let image=decode_blue(&verified_path(&scan,&frame.path,frame.bytes,&frame.sha256).unwrap()).unwrap();
            write_floats(&mut writer,&remap_at_size(&image,&map,manifest.geometry.mirror_x,512).unwrap()).unwrap();
            println!("512 detector view {}",frame.index);
        }writer.flush().unwrap();
    }

    #[test]
    #[ignore]
    fn inspect_axis_estimate() {
        let scan=PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let out=PathBuf::from(std::env::var("CT_DIAGNOSTIC_DIR").expect("CT_DIAGNOSTIC_DIR"));
        let (manifest,_)=read_manifest(&scan).unwrap();
        let bytes=fs::read(cache_dir(&scan,ReconstructionMethod::Fdk).join("projections.f32le")).unwrap();
        let stack:Vec<f32>=bytes.chunks_exact(4).map(|b|f32::from_le_bytes(b.try_into().unwrap())).collect();
        let angles:Vec<_>=manifest.frames.iter().map(|f|f.angle_deg.to_radians()*if manifest.geometry.rotation_direction=="counterclockwise" {1.}else{-1.}).collect();
        let axis=preprocessing::estimate_axis(&stack,&angles,manifest.geometry.detector_width_mm*SCREEN_CROP_SCALE/DETECTOR_N as f64,
            manifest.geometry.sod_mm+manifest.geometry.object_to_detector_mm,manifest.geometry.center_offset_x_mm,manifest.geometry.center_offset_y_mm);
        write_json(&out.join("axis-revised.json"),&axis).unwrap();
        println!("{}",serde_json::to_string_pretty(&axis).unwrap());
    }

    #[test]
    #[ignore]
    fn reconstruct_real_scan() {
        let scan_dir=PathBuf::from(std::env::var("CT_SCAN_DIR").expect("explicit CT_SCAN_DIR required"));
        let mut handle=ReconstructionHandle::start(ReconstructionRequest {scan_dir:scan_dir.clone(),method:ReconstructionMethod::Fdk}).unwrap();
        loop {
            if let Some(result)=handle.try_finish() {
                let result=result.expect("offline reconstruction");
                assert!(cached_result(&scan_dir,ReconstructionMethod::Fdk).unwrap().is_some());
                println!("PASS {}",result.cache_path.display());
                break;
            }
            println!("{:?}",handle.progress());
            thread::sleep(Duration::from_secs(5));
        }
    }

    /// Explicit opt-in diagnostic; never accesses devices or changes source NEFs.
    #[test]
    #[ignore]
    fn inspect_real_scan() {
        let scan = PathBuf::from(std::env::var("CT_SCAN_DIR").expect("CT_SCAN_DIR"));
        let out = PathBuf::from(std::env::var("CT_DIAGNOSTIC_DIR").expect("CT_DIAGNOSTIC_DIR"));
        fs::create_dir_all(&out).unwrap();
        let (manifest, _) = read_manifest(&scan).unwrap();
        let load = |f: &ReferenceFrame| decode_blue(&verified_path(&scan, &f.path, f.bytes, &f.sha256).unwrap()).unwrap();
        let dark = load(&manifest.references.pre_dark[0]);
        let flat = load(&manifest.references.pre_flat[0]);
        let map = find_screen(&flat, &dark).unwrap();
        println!("screen {}x{} center {},{} radii {},{} white {}", flat.width, flat.height, map.cx, map.cy, map.rx, map.ry, flat.white);
        for (name, image) in [("dark", &dark), ("flat", &flat)] {
            let floats: Vec<u8> = image.pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_bytes_durable(&out.join(format!("{name}.f32")), &floats).unwrap();
            let mut bytes = format!("P5\n{} {}\n255\n", image.width, image.height).into_bytes();
            bytes.extend(image.pixels.iter().map(|v| (v / image.white * 255.0).clamp(0.0,255.0) as u8));
            write_bytes_durable(&out.join(format!("{name}.pgm")), &bytes).unwrap();
        }
        let p = Mutex::new(ReconstructionProgress {status:"running",percent:0,message:String::new()});
        let mut done = 0;
        let d = mean_reference(&scan,&manifest.references.pre_dark,&map,false,&p,&mut done).unwrap();
        let f = mean_reference(&scan,&manifest.references.pre_flat,&map,false,&p,&mut done).unwrap();
        for frame in manifest.frames.iter().step_by(90) {
            let image = decode_blue(&verified_path(&scan,&frame.path,frame.bytes,&frame.sha256).unwrap()).unwrap();
            let raw = remap(&image,&map,false).unwrap();
            let floats: Vec<u8> = image.pixels.iter().flat_map(|v| v.to_le_bytes()).collect();
            write_bytes_durable(&out.join(format!("projection-{}.f32", frame.index)), &floats).unwrap();
            let (mut weak,mut saturated,mut negative,mut n)=(0,0,0,0);
            for i in 0..raw.len() {
                let u = ((i%DETECTOR_N) as f64+0.5)/DETECTOR_N as f64*2.0-1.0;
                let v = ((i/DETECTOR_N) as f64+0.5)/DETECTOR_N as f64*2.0-1.0;
                if (u*u+v*v)*SCREEN_CROP_SCALE.powi(2)>1.0 {continue;}
                n+=1;
                if f[i]-d[i]<=1.0 {weak+=1;}
                if raw[i]>=image.white {saturated+=1;}
                if raw[i]-d[i]<=0.0 {negative+=1;}
            }
            println!("view {} total {n} weak {weak} saturated {saturated} nonpositive {negative}",frame.index);
        }
    }

    #[test]
    fn gpu_route_needs_a_measured_fifteen_percent_gain() {
        let cpu = Duration::from_millis(1000);
        assert!(!gpu_beats_cpu(cpu, Duration::from_millis(851)));
        assert!(gpu_beats_cpu(cpu, Duration::from_millis(850)));
        assert!(gpu_beats_cpu(cpu, Duration::from_millis(300)));
    }

    #[test]
    fn gpu_numerical_probe_visits_distributed_samples() {
        let cpu = vec![1.0f32; 4096];
        let mut gpu = cpu.clone();
        let coverage = vec![1u8; cpu.len()];
        assert!(gpu_matches_cpu(&cpu, &gpu, &coverage));
        let sampled_index = 0x9e37_79b1usize % gpu.len();
        gpu[sampled_index] = 2.0;
        assert!(!gpu_matches_cpu(&cpu, &gpu, &coverage));
    }

    #[test]
    fn backprojection_uses_xyz_physical_orientation_and_mm_pitch() {
        let mut projection = vec![0.0f32; DETECTOR_N * DETECTOR_N];
        for row in 0..DETECTOR_N {
            for col in 0..DETECTOR_N {
                projection[row * DETECTOR_N + col] = col as f32 + row as f32 * 1000.0;
            }
        }
        let mut volume = vec![0.0f32; VOLUME_N * VOLUME_N * VOLUME_N];
        let mut coverage = vec![1u8; volume.len()];
        backproject(&mut volume, &mut coverage, &projection,
            0.0, 1.0, 1.0, 1000.0, 1000.0, 0.0, 0.0, 1.0).unwrap();
        let at = |z, y, x| volume[(z * VOLUME_N + y) * VOLUME_N + x];
        // +x maps to +detector column and +z maps to +detector row.
        assert!(at(128, 127, 130) > at(128, 127, 125));
        assert!(at(130, 127, 128) > at(125, 127, 128));
        assert!((at(128, 127, 130) - at(128, 127, 125) - 5.0).abs() < 0.1);
        assert_eq!(coverage[(128 * VOLUME_N + 127) * VOLUME_N + 128], 1);
    }

    #[test]
    fn sirt_forward_and_backprojection_share_the_same_discrete_operator() {
        let (sin, cos) = 0.41f64.sin_cos();
        let detector = sirt_detector(2.0, -1.0, 3.0, sin, cos,
            500.0, 700.0, 0.7, 0.0, 0.0).unwrap();
        let detector_again = sirt_detector(2.0, -1.0, 3.0, sin, cos,
            500.0, 700.0, 0.7, 0.0, 0.0).unwrap();
        assert_eq!(detector, detector_again);
        let mut sampled_ray = vec![0.0f32; SIRT_N * SIRT_N];
        sampled_ray[detector] = 2.5 * 0.7;
        let mut residual = vec![0.0f32; SIRT_N * SIRT_N];
        residual[detector] = 3.0;
        let left: f32 = sampled_ray.iter().zip(&residual).map(|(a, b)| a * b).sum();
        let right = 2.5 * residual[detector_again] * 0.7;
        assert!((left - right).abs() < 1e-6);
    }

    #[test]
    fn sirt_reduces_a_small_sphere_phantom_data_residual() {
        let sod = 500.0;
        let sdd = 700.0;
        let width = 48.0;
        let pitch = width * SCREEN_CROP_SCALE / SIRT_N as f64;
        let voxel = width * sod / sdd / SIRT_N as f64;
        let half = (SIRT_N as f64 - 1.0) * 0.5;
        let angles = [0.0, PI / 2.0, PI, 3.0 * PI / 2.0];
        let mut frames = Vec::new();
        for angle in angles {
            let (sin, cos) = angle.sin_cos();
            let mut detector = vec![0.0f32; SIRT_N * SIRT_N];
            for z in 0..SIRT_N {
                for y in 0..SIRT_N {
                    for x in 0..SIRT_N {
                        let xx = (x as f64 - half) * voxel;
                        let yy = (y as f64 - half) * voxel;
                        let zz = (z as f64 - half) * voxel;
                        if xx * xx + yy * yy + zz * zz > (3.0 * voxel).powi(2) { continue; }
                        if let Some(pixel) = sirt_detector(xx, yy, zz, sin, cos,
                            sod, sdd, pitch, 0.0, 0.0) {
                            detector[pixel] += voxel as f32;
                        }
                    }
                }
            }
            frames.push((angle, detector));
        }
        let initial_residual: f64 = frames.iter().flat_map(|(_, frame)| frame)
            .map(|v| (*v as f64).powi(2)).sum();
        assert!(initial_residual > 0.0);
        let progress = Mutex::new(ReconstructionProgress {
            status: "running", percent: 0, message: String::new(),
        });
        let (volume, _, _) = reconstruct_sirt(&frames, sod, sdd, width, 0.0, 0.0, &progress).unwrap();
        let mut final_residual = 0.0f64;
        for (angle, measured) in &frames {
            let (sin, cos) = angle.sin_cos();
            let mut predicted = vec![0.0f32; SIRT_N * SIRT_N];
            for z in 0..SIRT_N {
                for y in 0..SIRT_N {
                    for x in 0..SIRT_N {
                        let i = (z * SIRT_N + y) * SIRT_N + x;
                        let xx = (x as f64 - half) * voxel;
                        let yy = (y as f64 - half) * voxel;
                        let zz = (z as f64 - half) * voxel;
                        if let Some(pixel) = sirt_detector(xx, yy, zz, sin, cos,
                            sod, sdd, pitch, 0.0, 0.0) {
                            predicted[pixel] += volume[i] * voxel as f32;
                        }
                    }
                }
            }
            final_residual += predicted.iter().zip(measured)
                .map(|(a, b)| ((*a - *b) as f64).powi(2)).sum::<f64>();
        }
        assert!(final_residual < initial_residual * 0.9);
    }
}
