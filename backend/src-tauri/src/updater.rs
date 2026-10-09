//! Restricted release transport, common signed-package verification and early
//! update helper. Device shutdown authority is supplied by ct-engine/main.
use base64::{engine::general_purpose::STANDARD, Engine};
use fs2::FileExt;
use futures_util::StreamExt;
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::AsyncWriteExt;

const APP_ID: &str = "edu.lzu.microct.workstation";
const TARGET: &str = "windows-x86_64";
const REPOSITORY: &str = "Holy1one/Micro-CT-Workstation";
const MAX_METADATA: u64 = 256 * 1024;
const MAX_SIGNATURE: u64 = 8 * 1024;
const MAX_ARTIFACT: u64 = 1024 * 1024 * 1024;
const PUBLIC_KEY: &str = include_str!("../update-public-key.pub");

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub update_id: Option<String>,
    pub phase: String,
    pub current_version: String,
    pub target_version: Option<String>,
    pub channel: String,
    pub kind: String,
    pub source: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub verified: bool,
    pub notice: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub app_id: String,
    pub version: String,
    pub channel: String,
    pub published_at: String,
    pub source_commit: String,
    pub release_notes: String,
    pub artifacts: Vec<Artifact>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Artifact {
    pub target: String,
    pub kind: String,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub signature: String,
    pub url: String,
    #[serde(default)]
    pub installer_product_version: Option<String>,
}
#[derive(Clone)]
pub struct VerifiedUpdate {
    id: String,
    directory: PathBuf,
    manifest: ReleaseManifest,
    artifact: Artifact,
    manifest_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub creation_time: u64,
    pub exit_code: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateAuthority {
    pub session_id: String,
    pub operation_id: String,
    pub status: String,
    pub ready: bool,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub turntable_stop: String,
    pub xray_off: String,
    pub camera_cleanup: String,
    pub no_active_transactions: bool,
    pub error: Option<String>,
    pub maintenance_locked: bool,
}
impl UpdateAuthority {
    pub fn validate(&self) -> Result<(), String> {
        let confirmed = |value: &str| matches!(value, "confirmed" | "never-owned");
        if self.session_id.is_empty()
            || self.operation_id.is_empty()
            || self.status != "ready"
            || !self.ready
            || self.completed_at.is_none()
            || !self.no_active_transactions
            || !self.maintenance_locked
            || self.error.is_some()
            || !confirmed(&self.turntable_stop)
            || !confirmed(&self.xray_off)
            || !confirmed(&self.camera_cleanup)
        {
            return Err("本次维护停机权威报告未完成或仍有未知确认项".into());
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApplyIntent {
    schema_version: u32,
    update_id: String,
    manifest_hash: String,
    version: String,
    kind: String,
    target: PathBuf,
    target_hash: String,
    data_directory: PathBuf,
    owner: ProcessIdentity,
    engine: ProcessIdentity,
    authority: UpdateAuthority,
}
#[derive(Clone)]
pub struct UpdaterService {
    inner: Arc<Inner>,
}
struct Inner {
    data: PathBuf,
    cache: PathBuf,
    state: Mutex<UpdateState>,
    selected: Mutex<Option<VerifiedUpdate>>,
    cancelled: AtomicBool,
    busy: AtomicBool,
    _instance: File,
}

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn identifier() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
fn key() -> Result<PublicKey, String> {
    PublicKey::decode(&decode_outer(PUBLIC_KEY)?).map_err(err)
}
fn decode_outer(value: &str) -> Result<String, String> {
    if value.len() as u64 > MAX_SIGNATURE {
        return Err("签名或公钥过大".into());
    }
    String::from_utf8(STANDARD.decode(value.trim()).map_err(err)?).map_err(err)
}
fn signature(value: &str) -> Result<Signature, String> {
    Signature::decode(&decode_outer(value)?).map_err(err)
}
fn signed_version(sig: &Signature, expected: &str) -> Result<(), String> {
    let fields: Vec<_> = sig
        .trusted_comment()
        .split('\t')
        .filter_map(|field| field.strip_prefix("version:"))
        .collect();
    if fields.len() != 1 || fields[0] != expected {
        return Err("签名必须包含唯一且匹配的受签应用版本".into());
    }
    Ok(())
}
fn basename(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 180
        && value
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"._-".contains(&v))
        && !value.contains("..")
}
fn artifact_filename(version: &str, kind: &str) -> Result<String, String> {
    let suffix = match kind {
        "nsis" => "-setup.exe",
        "msi" => ".msi",
        _ => return Err("未知更新包类型".into()),
    };
    Ok(format!(
        "micro-ct-workstation_{version}_windows_x86_64{suffix}"
    ))
}
pub fn distribution_kind() -> Result<String, String> {
    match tauri::utils::platform::bundle_type() {
        None => Ok("development".into()),
        Some(tauri::utils::config::BundleType::Nsis) => Ok("nsis".into()),
        Some(tauri::utils::config::BundleType::Msi) => Ok("msi".into()),
        _ => Err("本机分发身份不支持 Windows 更新".into()),
    }
}
fn validate_channel(value: &str) -> Result<(), String> {
    if matches!(value, "stable" | "candidate") {
        Ok(())
    } else {
        Err("未知发行通道".into())
    }
}
fn verify_manifest_with_key(
    raw: &[u8],
    sig: &str,
    public: &PublicKey,
    channel: &str,
    kind: &str,
    current: &str,
    allow_equal: bool,
) -> Result<(ReleaseManifest, Artifact), String> {
    if raw.len() as u64 > MAX_METADATA {
        return Err("发行清单超过上限".into());
    }
    let sig = signature(sig)?;
    public
        .verify(raw, &sig, false)
        .map_err(|error| format!("发行清单双签名校验失败: {error}"))?;
    let manifest: ReleaseManifest = serde_json::from_slice(raw).map_err(err)?;
    signed_version(&sig, &manifest.version)?;
    validate_channel(&manifest.channel)?;
    let version = semver::Version::parse(&manifest.version).map_err(err)?;
    let current = semver::Version::parse(current).map_err(err)?;
    if manifest.schema_version != 1
        || manifest.app_id != APP_ID
        || manifest.channel != channel
        || manifest.source_commit.len() != 40
        || !manifest
            .source_commit
            .bytes()
            .all(|v| v.is_ascii_hexdigit())
        || manifest.artifacts.is_empty()
        || manifest.artifacts.len() > 3
        || (channel == "stable" && !version.pre.is_empty())
        || (version.cmp_precedence(&current).is_lt()
            || (!allow_equal && version.cmp_precedence(&current).is_eq()))
    {
        return Err("发行身份/通道/版本不匹配，或更新不是较高版本".into());
    }
    let mut seen = HashSet::new();
    for artifact in &manifest.artifacts {
        if artifact.target != TARGET
            || !seen.insert(artifact.kind.clone())
            || artifact.file_name != artifact_filename(&manifest.version, &artifact.kind)?
            || !basename(&artifact.file_name)
            || artifact.size == 0
            || artifact.size > MAX_ARTIFACT
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|v| v.is_ascii_hexdigit())
        {
            return Err("发行文件目标/类型/名称/大小/SHA不合法".into());
        }
        validate_asset_url(&artifact.url, Some(&artifact.file_name))?;
        if artifact.kind == "msi"
            && artifact
                .installer_product_version
                .as_deref()
                .is_none_or(|v| numeric_msi_version(v).is_err())
        {
            return Err("MSI缺少有效的安装版本".into());
        }
        signature(&artifact.signature)?;
    }
    let artifact = manifest
        .artifacts
        .iter()
        .find(|a| a.kind == kind)
        .cloned()
        .ok_or("发行不含本机包类型，禁止跨类型安装")?;
    Ok((manifest, artifact))
}
fn numeric_msi_version(value: &str) -> Result<(), String> {
    let parts: Vec<_> = value
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<_, _>>()
        .map_err(err)?;
    if parts.len() != 3 || parts[0] > 255 || parts[1] > 255 || parts[2] == 0 || parts[2] > 65535 {
        return Err("MSI版本超出范围".into());
    }
    Ok(())
}
pub fn verify_release_directory(
    directory: &Path,
    channel: &str,
    kind: &str,
    current: &str,
    allow_equal: bool,
) -> Result<String, String> {
    let raw = read_bounded(&directory.join("release-manifest.json"), MAX_METADATA)?;
    let sig = read_text(&directory.join("release-manifest.json.sig"), MAX_SIGNATURE)?;
    let public = key()?;
    let (manifest, artifact) =
        verify_manifest_with_key(&raw, &sig, &public, channel, kind, current, allow_equal)?;
    let external = read_text(
        &directory.join(format!("{}.sig", artifact.file_name)),
        MAX_SIGNATURE,
    )?;
    if external.trim() != artifact.signature.trim() {
        return Err("包签名与受签清单不一致".into());
    }
    verify_artifact(
        &directory.join(&artifact.file_name),
        &manifest.version,
        &artifact,
        &public,
        None,
    )?;
    Ok(manifest.version)
}
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(err)?;
    if file.metadata().map_err(err)?.len() > max {
        return Err("文件超过大小上限".into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(err)?;
    if bytes.len() as u64 > max {
        return Err("读取文件超过大小上限".into());
    }
    Ok(bytes)
}
fn read_text(path: &Path, max: u64) -> Result<String, String> {
    String::from_utf8(read_bounded(path, max)?).map_err(err)
}
fn verify_artifact(
    path: &Path,
    version: &str,
    artifact: &Artifact,
    public: &PublicKey,
    cancelled: Option<&AtomicBool>,
) -> Result<(), String> {
    let file = File::open(path).map_err(err)?;
    if file.metadata().map_err(err)?.len() != artifact.size {
        return Err("更新文件字节数不匹配".into());
    }
    verify_reader(file, version, artifact, public, cancelled)
}
fn verify_reader(
    mut file: impl Read,
    version: &str,
    artifact: &Artifact,
    public: &PublicKey,
    cancelled: Option<&AtomicBool>,
) -> Result<(), String> {
    let sig = signature(&artifact.signature)?;
    let mut verifier = public.verify_stream(&sig).map_err(err)?;
    let mut sha = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0u64;
    loop {
        if cancelled.is_some_and(|v| v.load(Ordering::SeqCst)) {
            return Err("更新已取消".into());
        }
        let count = file.read(&mut buffer).map_err(err)?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > artifact.size {
            return Err("更新文件增长超过声明大小".into());
        }
        sha.update(&buffer[..count]);
        verifier.update(&buffer[..count]);
    }
    verifier
        .finalize()
        .map_err(|e| format!("更新包双签名校验失败: {e}"))?;
    signed_version(&sig, version)?;
    if total != artifact.size
        || format!("{:x}", sha.finalize()) != artifact.sha256.to_ascii_lowercase()
    {
        return Err("更新包SHA或大小不匹配".into());
    }
    Ok(())
}
struct OfflinePackage {
    zip: zip::ZipArchive<File>,
    raw: Vec<u8>,
    manifest: ReleaseManifest,
    artifact: Artifact,
}
fn validate_zip_entries<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
) -> Result<HashSet<String>, String> {
    if zip.len() < 4 || zip.len() > 8 {
        return Err("离线包文件数量不合法".into());
    }
    let mut names = HashSet::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(err)?;
        if !basename(entry.name())
            || entry.is_dir()
            || entry.encrypted()
            || entry.compression() != zip::CompressionMethod::Stored
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0 && mode & 0o170000 != 0o100000)
            || !names.insert(entry.name().to_owned())
        {
            return Err("离线包包含路径、链接、加密、压缩或重复项".into());
        }
        total = total.checked_add(entry.size()).ok_or("离线包大小溢出")?;
        if total > MAX_ARTIFACT + 2 * MAX_METADATA {
            return Err("离线包展开超限".into());
        }
    }
    Ok(names)
}
fn open_verified_offline(
    path: &Path,
    kind: &str,
    current: &str,
    allow_equal: bool,
) -> Result<OfflinePackage, String> {
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_none_or(|s| !s.eq_ignore_ascii_case("zip"))
    {
        return Err("离线更新必须是ZIP文件".into());
    }
    let file = File::open(path).map_err(err)?;
    if file.metadata().map_err(err)?.len() > MAX_ARTIFACT + 2 * MAX_METADATA {
        return Err("离线包超过大小上限".into());
    }
    let mut zip = zip::ZipArchive::new(file).map_err(err)?;
    let names = validate_zip_entries(&mut zip)?;
    let raw = read_zip(&mut zip, "release-manifest.json", MAX_METADATA)?;
    let sig = String::from_utf8(read_zip(
        &mut zip,
        "release-manifest.json.sig",
        MAX_SIGNATURE,
    )?)
    .map_err(err)?;
    let untrusted: ReleaseManifest = serde_json::from_slice(&raw).map_err(err)?;
    validate_channel(&untrusted.channel)?;
    let (manifest, artifact) = verify_manifest_with_key(
        &raw,
        &sig,
        &key()?,
        &untrusted.channel,
        kind,
        current,
        allow_equal,
    )?;
    let allowed = HashSet::from([
        "release-manifest.json".to_owned(),
        "release-manifest.json.sig".to_owned(),
        artifact.file_name.clone(),
        format!("{}.sig", artifact.file_name),
        "LICENSE.txt".into(),
        "THIRD-PARTY-NOTICES.md".into(),
        "third-party-licenses.json".into(),
    ]);
    if !names.is_subset(&allowed) {
        return Err("离线包包含未授权文件".into());
    }
    for name in &names {
        if name != &artifact.file_name
            && zip.by_name(name).map_err(err)?.size()
                > if name.ends_with(".sig") {
                    MAX_SIGNATURE
                } else {
                    MAX_METADATA
                }
        {
            return Err("离线包附属文件超限".into());
        }
    }
    let external = String::from_utf8(read_zip(
        &mut zip,
        &format!("{}.sig", artifact.file_name),
        MAX_SIGNATURE,
    )?)
    .map_err(err)?;
    if external.trim() != artifact.signature.trim() {
        return Err("离线文件签名与清单不一致".into());
    }
    if zip.by_name(&artifact.file_name).map_err(err)?.size() != artifact.size {
        return Err("离线文件大小不一致".into());
    }
    Ok(OfflinePackage {
        zip,
        raw,
        manifest,
        artifact,
    })
}
pub fn verify_offline_package(path: &Path, kind: &str, current: &str) -> Result<String, String> {
    let mut package = open_verified_offline(path, kind, current, true)?;
    let file = package
        .zip
        .by_name(&package.artifact.file_name)
        .map_err(err)?;
    verify_reader(
        file,
        &package.manifest.version,
        &package.artifact,
        &key()?,
        None,
    )?;
    Ok(package.manifest.version)
}
fn validate_asset_url(value: &str, file: Option<&str>) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(err)?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("更新地址必须为本仓公开HTTPS资源".into());
    }
    let prefix = format!("/{REPOSITORY}/releases/download/");
    let tail = url
        .path()
        .strip_prefix(&prefix)
        .ok_or("更新地址不属于本仓Release资源")?;
    let parts: Vec<_> = tail.split('/').collect();
    if parts.len() != 2
        || !basename(parts[0])
        || !basename(parts[1])
        || file.is_some_and(|f| parts[1] != f)
    {
        return Err("更新地址标签/文件不匹配".into());
    }
    Ok(url)
}
fn allowed_transport(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
                    | "gh-proxy.org"
            )
        )
}
fn transport() -> Result<reqwest::Client, String> {
    transport_with_redirects(true)
}
fn transport_with_redirects(follow: bool) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .user_agent("MicroCTWorkstation-Updater/0.9")
        .redirect(if follow {reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 || !allowed_transport(attempt.url()) {
                attempt.error("更新重定向不在HTTPS白名单内")
            } else {
                attempt.follow()
            }
        })} else {reqwest::redirect::Policy::none()})
        .build()
        .map_err(err)
}
fn sources(original: &str) -> Result<Vec<String>, String> {
    validate_asset_url(original, None)?;
    if std::env::var_os("MICRO_CT_UPDATE_DISABLE_MIRROR").is_some_and(|v| v == "1") {
        return Ok(vec![original.into()]);
    }
    Ok(vec![
        format!("https://gh-proxy.org/{original}"),
        original.into(),
    ])
}
async fn fetch_small(
    client: &reqwest::Client,
    url: &str,
    max: u64,
    cancelled: &AtomicBool,
) -> Result<Option<Vec<u8>>, String> {
    let parsed = reqwest::Url::parse(url).map_err(err)?;
    if !allowed_transport(&parsed) {
        return Err("禁止的更新通道".into());
    }
    let response = tokio::time::timeout(Duration::from_secs(25), client.get(parsed).send())
        .await
        .map_err(|_| "元数据查询超时")?
        .map_err(err)?;
    if response.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("更新源HTTP {}", response.status()));
    }
    if response.content_length().is_some_and(|size| size > max) {
        return Err("更新元数据过大".into());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("更新已取消".into());
        }
        let next = tokio::time::timeout(Duration::from_secs(20), stream.next())
            .await
            .map_err(|_| "元数据读取超时")?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(err)?;
        if bytes.len() as u64 + chunk.len() as u64 > max {
            return Err("更新元数据超限".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}
/// Latest-release metadata is discovery evidence, never installation authority.
#[derive(Deserialize)]
struct LatestRelease { tag_name: String, draft: bool, prerelease: bool }
fn latest_release(raw: &[u8]) -> Result<(String, semver::Version), String> {
    let release: LatestRelease = serde_json::from_slice(raw).map_err(err)?;
    let version = semver::Version::parse(release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name)).map_err(err)?;
    if release.draft || release.prerelease || !version.pre.is_empty() || !basename(&release.tag_name) {
        return Err("The latest stable release has invalid metadata.".into());
    }
    Ok((release.tag_name, version))
}
fn latest_redirect(location: &str) -> Result<(String, semver::Version), String> {
    let url=reqwest::Url::parse(location).map_err(err)?;
    let prefix=format!("/{REPOSITORY}/releases/tag/");
    if url.scheme()!="https" || url.host_str()!=Some("github.com") || !url.username().is_empty()
        || url.password().is_some() || url.port().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("The update server returned an unexpected release address.".into());
    }
    let tag=url.path().strip_prefix(&prefix).ok_or("The update server returned an unexpected release.")?;
    latest_release(&serde_json::to_vec(&serde_json::json!({"tag_name":tag,"draft":false,"prerelease":false})).map_err(err)?)
}
impl UpdaterService {
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&data_dir).map_err(err)?;
        let data = fs::canonicalize(data_dir).map_err(err)?;
        let cache = data.join("updates");
        fs::create_dir_all(&cache).map_err(err)?;
        let cache = fs::canonicalize(cache).map_err(err)?;
        if cache.parent() != Some(data.as_path()) {
            return Err("更新缓存不能是跨目录链接".into());
        }
        secure_directory(&cache)?;
        let instance = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(cache.join("instances.lock"))
            .map_err(err)?;
        FileExt::try_lock_shared(&instance)
            .map_err(|_| "软件更新正在应用，暂时不能启动另一实例")?;
        let kind = distribution_kind()?;
        key()?;
        let mut state = UpdateState {
            update_id: None,
            phase: "idle".into(),
            current_version: env!("CARGO_PKG_VERSION").into(),
            target_version: None,
            channel: if semver::Version::parse(env!("CARGO_PKG_VERSION"))
                .map_err(err)?
                .pre
                .is_empty()
            {
                "stable"
            } else {
                "candidate"
            }
            .into(),
            kind,
            source: None,
            downloaded_bytes: 0,
            total_bytes: None,
            verified: false,
            notice: None,
            error: None,
        };
        let feedback = check_receipts(&cache, &state.current_version, &state.kind)?;
        state.notice = feedback.notice;
        state.error = feedback.error;
        Ok(Self {
            inner: Arc::new(Inner {
                data,
                cache,
                state: Mutex::new(state),
                selected: Mutex::new(None),
                cancelled: AtomicBool::new(false),
                busy: AtomicBool::new(false),
                _instance: instance,
            }),
        })
    }
    pub fn status(&self) -> Result<UpdateState, String> {
        self.inner.state.lock().map(|v| v.clone()).map_err(err)
    }
    /// Native import calls this before opening its picker, including cancellation.
    pub fn clear_feedback(&self) -> Result<UpdateState, String> {
        let mut state = self.inner.state.lock().map_err(err)?;
        state.notice = None;
        state.error = None;
        Ok(state.clone())
    }
    fn set(&self, f: impl FnOnce(&mut UpdateState)) {
        if let Ok(mut state) = self.inner.state.lock() {
            f(&mut state)
        }
    }
    fn start(&self, channel: &str) -> Result<(String, String), String> {
        validate_channel(channel)?;
        let mut state = self.inner.state.lock().map_err(err)?;
        if matches!(state.phase.as_str(), "preparing" | "applying") {
            return Err("安装维护准备期间禁止新的更新操作".into());
        }
        if self.inner.busy.swap(true, Ordering::SeqCst) {
            return Err("已有更新操作正在进行".into());
        }
        self.inner.cancelled.store(false, Ordering::SeqCst);
        let id = identifier();
        let kind = state.kind.clone();
        *self.inner.selected.lock().map_err(err)? = None;
        {
            let s = &mut *state;
            s.update_id = Some(id.clone());
            s.channel = channel.into();
            s.phase = "checking".into();
            s.target_version = None;
            s.verified = false;
            s.error = None;
            s.notice = None;
            s.source = None;
            s.downloaded_bytes = 0;
            s.total_bytes = None;
        }
        Ok((id, kind))
    }
    fn mark_ready(&self) -> Result<(), String> {
        let mut state = self.inner.state.lock().map_err(err)?;
        if self.inner.cancelled.load(Ordering::SeqCst) {
            return Err("更新已取消".into());
        }
        state.phase = "ready".into();
        state.verified = true;
        Ok(())
    }
    fn finish(&self, result: Result<(), String>) -> Result<UpdateState, String> {
        if let Err(error) = result {
            self.set(|s| {
                s.phase = if self.inner.cancelled.load(Ordering::SeqCst) {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                s.notice = None;
                s.error = if self.inner.cancelled.load(Ordering::SeqCst) {
                    None
                } else {
                    Some(error)
                };
                s.verified = false;
            });
        }
        self.inner.busy.store(false, Ordering::SeqCst);
        self.status()
    }
    pub fn cancel(&self, update_id: String) -> Result<UpdateState, String> {
        let mut state = self.inner.state.lock().map_err(err)?;
        if state.update_id.as_deref() != Some(&update_id) {
            return Err("更新操作ID已失效".into());
        }
        if matches!(state.phase.as_str(), "preparing" | "applying") {
            return Err("安装准备由引擎维护取消入口处理".into());
        }
        self.inner.cancelled.store(true, Ordering::SeqCst);
        state.phase = "cancelled".into();
        state.verified = false;
        state.notice = None;
        state.error = None;
        Ok(state.clone())
    }
    pub async fn check(&self, channel: String) -> Result<UpdateState, String> {
        let (id, kind) = self.start(&channel)?;
        let result = self.check_inner(&id, &channel, &kind).await;
        self.finish(result)
    }
    async fn check_inner(&self, id: &str, channel: &str, kind: &str) -> Result<(), String> {
        let client = transport()?;
        let cancel = &self.inner.cancelled;
        if kind == "development" { return Err("Install the Windows application to check for updates.".into()); }
        // GitHub's stable-release redirect avoids unauthenticated API rate limits.
        // Read only response headers; a release page is not trusted package metadata.
        let endpoint = format!("https://github.com/{REPOSITORY}/releases/latest");
        self.set(|state| state.source=Some(endpoint.clone()));
        let response=tokio::time::timeout(Duration::from_secs(25),transport_with_redirects(false)?.get(&endpoint).send()).await
            .map_err(|_|"Checking for updates timed out. Try again.")?.map_err(err)?;
        if !matches!(response.status().as_u16(),301|302|303|307|308) {
            return Err(format!("Couldn't check for updates (HTTP {}). Try again.",response.status()));
        }
        let (tag, latest)=latest_redirect(response.headers().get(reqwest::header::LOCATION)
            .ok_or("The update server returned no release information.")?.to_str().map_err(err)?)?;
        if cancel.load(Ordering::SeqCst) { return Err("Update cancelled.".into()); }
        if !latest.cmp_precedence(&semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(err)?).is_gt() {
            let mut state=self.inner.state.lock().map_err(err)?;
            if cancel.load(Ordering::SeqCst) { return Err("Update cancelled.".into()); }
            state.phase="no-update".into();state.target_version=Some(latest.to_string());
            return Ok(());
        }
        let original = format!("https://github.com/{REPOSITORY}/releases/download/{tag}/release-manifest.json");
        let mut errors = Vec::new();
        let mut chosen = None;
        for source in sources(&original)? {
            self.set(|s| s.source = Some(source.clone()));
            let raw = match fetch_small(&client, &source, MAX_METADATA, cancel).await {
                Ok(Some(v)) => v,
                Ok(None) => {
                    return Err("The release manifest is missing.".into());
                }
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };
            let sig =
                match fetch_small(&client, &format!("{source}.sig"), MAX_SIGNATURE, cancel).await {
                    Ok(Some(v)) => String::from_utf8(v).map_err(err)?,
                    Ok(None) => return Err("发行清单缺少签名".into()),
                    Err(e) => {
                        errors.push(e);
                        continue;
                    }
                };
            // A cryptographic/identity failure is terminal and is not a network fallback.
            let (manifest, artifact) =
                verify_manifest_with_key(&raw, &sig, &key()?, channel, kind, "0.0.0", true)?;
            if semver::Version::parse(&manifest.version).map_err(err)?.cmp_precedence(&latest) != std::cmp::Ordering::Equal
                || !artifact.url.contains(&format!("/releases/download/{tag}/")) {
                return Err("The signed update does not match the latest release.".into());
            }
            chosen = Some((raw, sig, manifest, artifact));
            break;
        }
        let (raw, sig, manifest, artifact) =
            chosen.ok_or_else(|| format!("检查失败: {}", errors.join("; ")))?;
        if cancel.load(Ordering::SeqCst) {
            return Err("更新已取消".into());
        }
        let directory = self.inner.cache.join(id);
        fs::create_dir(&directory).map_err(err)?;
        write_new(&directory.join("release-manifest.json"), &raw)?;
        write_new(&directory.join("release-manifest.json.sig"), sig.as_bytes())?;
        write_new(
            &directory.join(format!("{}.sig", artifact.file_name)),
            artifact.signature.as_bytes(),
        )?;
        let update = VerifiedUpdate { id: id.into(), directory, manifest, artifact, manifest_hash: hash(&raw) };
        let mut state=self.inner.state.lock().map_err(err)?;
        if cancel.load(Ordering::SeqCst) { return Err("Update cancelled.".into()); }
        state.target_version=Some(update.manifest.version.clone());
        state.total_bytes=Some(update.artifact.size);state.phase="available".into();
        *self.inner.selected.lock().map_err(err)?=Some(update);
        Ok(())
    }
    fn begin_download(&self, id: &str) -> Result<VerifiedUpdate,String> {
        let mut state=self.inner.state.lock().map_err(err)?;
        if state.update_id.as_deref()!=Some(id) || state.phase!="available" {
            return Err("This update is no longer available. Check for updates again.".into());
        }
        let update=self.inner.selected.lock().map_err(err)?.clone().ok_or("No update selected.")?;
        if update.id!=id {return Err("Update identity mismatch.".into());}
        if self.inner.busy.swap(true,Ordering::SeqCst) {return Err("An update operation is already running.".into());}
        self.inner.cancelled.store(false,Ordering::SeqCst);
        state.phase="downloading".into();state.downloaded_bytes=0;state.error=None;state.notice=None;
        Ok(update)
    }
    pub async fn download_update(&self, id: String) -> Result<UpdateState,String> {
        let update=self.begin_download(&id)?;
        let result=self.download_selected(&update).await;
        self.finish(result)
    }
    async fn download_selected(&self, update: &VerifiedUpdate) -> Result<(),String> {
        let client=transport()?;let cancel=&self.inner.cancelled;
        let artifact=&update.artifact;
        let output=update.directory.join(&artifact.file_name);
        let part=update.directory.join("artifact.part");
        let mut errors=Vec::new();let mut downloaded=false;
        for source in sources(&artifact.url)? {
            self.set(|s| {s.source=Some(source.clone());s.downloaded_bytes=0;});
            match self.download(&client,&source,&part,artifact).await {
                Ok(()) => {downloaded=true;break;}
                Err(error) => {
                    let _=fs::remove_file(&part);errors.push(error);
                    if cancel.load(Ordering::SeqCst) {return Err("Update cancelled.".into());}
                }
            }
        }
        if !downloaded {return Err(format!("Download failed: {}",errors.join("; ")));}
        self.set(|s|s.phase="verifying".into());
        verify_artifact(&part,&update.manifest.version,artifact,&key()?,Some(cancel))?;
        fs::rename(&part,&output).map_err(err)?;
        self.mark_ready()
    }
    async fn download(
        &self,
        client: &reqwest::Client,
        url: &str,
        part: &Path,
        artifact: &Artifact,
    ) -> Result<(), String> {
        let response = client.get(url).send().await.map_err(err)?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }
        if response
            .content_length()
            .is_some_and(|v| v != artifact.size)
        {
            return Err("下载声明字节数与发行不一致".into());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(part)
            .await
            .map_err(err)?;
        let mut stream = response.bytes_stream();
        let mut total = 0u64;
        loop {
            if self.inner.cancelled.load(Ordering::SeqCst) {
                return Err("更新已取消".into());
            }
            let next = tokio::time::timeout(Duration::from_secs(20), stream.next())
                .await
                .map_err(|_| "下载读取超时")?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(err)?;
            total += chunk.len() as u64;
            if total > artifact.size || total > MAX_ARTIFACT {
                return Err("下载文件超过上限".into());
            }
            file.write_all(&chunk).await.map_err(err)?;
            self.set(|s| s.downloaded_bytes = total);
        }
        file.sync_all().await.map_err(err)?;
        if total != artifact.size {
            return Err("下载不完整".into());
        }
        Ok(())
    }
    pub async fn import_zip(&self, path: PathBuf) -> Result<UpdateState, String> {
        let channel = self.status()?.channel;
        let (id, kind) = self.start(&channel)?;
        let this = self.clone();
        let result =
            tauri::async_runtime::spawn_blocking(move || this.import_inner(&path, &id, &kind))
                .await
                .map_err(err)?;
        self.finish(result)
    }
    fn import_inner(&self, path: &Path, id: &str, kind: &str) -> Result<(), String> {
        let OfflinePackage {
            mut zip,
            raw,
            manifest,
            artifact,
        } = open_verified_offline(path, kind, env!("CARGO_PKG_VERSION"), false)?;
        let directory = self.inner.cache.join(id);
        fs::create_dir(&directory).map_err(err)?;
        for name in [
            "release-manifest.json".to_owned(),
            "release-manifest.json.sig".to_owned(),
            format!("{}.sig", artifact.file_name),
        ] {
            let max = if name == "release-manifest.json" {
                MAX_METADATA
            } else {
                MAX_SIGNATURE
            };
            write_new(&directory.join(&name), &read_zip(&mut zip, &name, max)?)?;
        }
        let mut input = zip.by_name(&artifact.file_name).map_err(err)?;
        if input.size() != artifact.size {
            return Err("离线文件大小不一致".into());
        }
        let output = directory.join(&artifact.file_name);
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(err)?;
        let mut copied = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            if self.inner.cancelled.load(Ordering::SeqCst) {
                return Err("更新已取消".into());
            }
            let n = input.read(&mut buffer).map_err(err)?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > artifact.size {
                return Err("离线文件展开超限".into());
            }
            file.write_all(&buffer[..n]).map_err(err)?;
            self.set(|s| s.downloaded_bytes = copied);
        }
        file.sync_all().map_err(err)?;
        drop(file);
        self.set(|s| {
            s.phase = "verifying".into();
            s.channel = manifest.channel.clone();
            s.source = Some("offline".into());
            s.target_version = Some(manifest.version.clone());
            s.total_bytes = Some(artifact.size);
        });
        verify_artifact(
            &output,
            &manifest.version,
            &artifact,
            &key()?,
            Some(&self.inner.cancelled),
        )?;
        *self.inner.selected.lock().map_err(err)? = Some(VerifiedUpdate {
            id: id.into(),
            directory,
            manifest,
            artifact,
            manifest_hash: hash(&raw),
        });
        self.mark_ready()
    }
    pub fn prepare_apply(
        &self,
        update_id: &str,
        expected_version: &str,
    ) -> Result<VerifiedUpdate, String> {
        let mut state = self.inner.state.lock().map_err(err)?;
        if state.phase != "ready"
            || !state.verified
            || state.update_id.as_deref() != Some(update_id)
            || state.target_version.as_deref() != Some(expected_version)
        {
            return Err("更新操作/版本已失效或文件尚未验证".into());
        }
        state.phase = "preparing".into();
        drop(state);
        let result: Result<VerifiedUpdate, String> = (|| {
            let update = self
                .inner
                .selected
                .lock()
                .map_err(err)?
                .clone()
                .ok_or("没有已验证更新")?;
            verify_release_directory(
                &update.directory,
                &update.manifest.channel,
                &update.artifact.kind,
                env!("CARGO_PKG_VERSION"),
                false,
            )?;
            Ok(update)
        })();
        if let Err(error) = &result {
            self.apply_failed(error.clone());
        }
        result
    }
    pub fn apply_failed(&self, error: String) {
        self.set(|s| {
            s.phase = "failed".into();
            s.notice = None;
            s.error = Some(error);
            s.verified = false;
        });
    }
    pub fn spawn_helper(
        &self,
        update: VerifiedUpdate,
        authority: UpdateAuthority,
        engine: ProcessIdentity,
    ) -> Result<(), String> {
        authority.validate()?;
        if engine.exit_code != 0 || engine.pid == 0 || engine.creation_time == 0 {
            return Err("引擎正常退出身份未确认".into());
        }
        let target = fs::canonicalize(std::env::current_exe().map_err(err)?).map_err(err)?;
        let directory = fs::canonicalize(&update.directory).map_err(err)?;
        if directory.parent() != Some(self.inner.cache.as_path()) || !basename(&update.id) {
            return Err("更新缓存路径身份不匹配".into());
        }
        let helper = directory.join("apply-helper.exe");
        fs::copy(&target, &helper).map_err(err)?;
        let intent = ApplyIntent {
            schema_version: 1,
            update_id: update.id,
            manifest_hash: update.manifest_hash,
            version: update.manifest.version,
            kind: update.artifact.kind,
            target: target.clone(),
            target_hash: file_hash(&target)?,
            data_directory: self.inner.data.clone(),
            owner: current_process_identity()?,
            engine,
            authority,
        };
        let intent_path = directory.join("intent.json");
        write_new(&intent_path, &serde_json::to_vec(&intent).map_err(err)?)?;
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new(&helper);
        command.arg("--apply-update").arg(&intent_path);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(err)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if directory.join("helper-ready.json").exists() {
                write_new(&directory.join("helper-accepted.json"), b"accepted")?;
                self.set(|s| s.phase = "applying".into());
                return Ok(());
            }
            if child.try_wait().map_err(err)?.is_some() {
                return Err(read_text(&directory.join("helper-error.txt"), MAX_METADATA)
                    .unwrap_or_else(|_| "更新助手提前退出".into()));
            }
            if std::time::Instant::now() >= deadline {
                write_new(
                    &directory.join("helper-cancelled.json"),
                    b"{\"reason\":\"handshake-timeout\"}",
                )?;
                return Err("更新助手未确认本次父进程/引擎身份，应用保持运行".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
fn read_zip<R: Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
    max: u64,
) -> Result<Vec<u8>, String> {
    let file = zip.by_name(name).map_err(err)?;
    if file.size() > max {
        return Err("离线包元数据超限".into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes).map_err(err)?;
    if bytes.len() as u64 > max {
        return Err("离线包元数据展开超限".into());
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(err)?;
    file.write_all(bytes).map_err(err)?;
    file.sync_all().map_err(err)
}
fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(err)?;
    let mut sha = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).map_err(err)?;
        if n == 0 {
            break;
        }
        sha.update(&buf[..n]);
    }
    Ok(format!("{:x}", sha.finalize()))
}

/// Called before Tauri/WebView/engine initialization. Also supplies a bounded
/// release-verification entry used by local distribution scripts.
pub fn early_helper() -> Option<Result<(), String>> {
    let args: Vec<_> = std::env::args_os().collect();
    let mode = args.get(1)?.to_str()?;
    if mode == "--verify-update-package" {
        return Some((|| {
            if args.len() != 6 {
                return Err(
                    "Package verifier expects directory/channel/kind/currentVersion".into(),
                );
            }
            verify_release_directory(
                Path::new(&args[2]),
                args[3].to_str().ok_or("Bad channel")?,
                args[4].to_str().ok_or("Bad kind")?,
                args[5].to_str().ok_or("Bad current version")?,
                true,
            )?;
            Ok(())
        })());
    }
    if mode == "--verify-offline-update" {
        return Some((|| {
            if args.len() != 5 {
                return Err("Offline verifier expects zip/kind/currentVersion".into());
            }
            verify_offline_package(
                Path::new(&args[2]),
                args[3].to_str().ok_or("Bad kind")?,
                args[4].to_str().ok_or("Bad current version")?,
            )?;
            Ok(())
        })());
    }
    if mode != "--apply-update" {
        return None;
    }
    Some((|| {
        if args.len() != 3 {
            return Err("更新助手仅接受一个本地意图文件".into());
        }
        let path = PathBuf::from(&args[2]);
        let result = run_apply_helper(&path);
        if let Err(error) = &result {
            if let Some(directory) = path.parent() {
                let _ = write_new(&directory.join("helper-error.txt"), error.as_bytes());
            }
        }
        result
    })())
}
fn run_apply_helper(path: &Path) -> Result<(), String> {
    let path = fs::canonicalize(path).map_err(err)?;
    let directory = path.parent().ok_or("意图无目录")?;
    let intent: ApplyIntent =
        serde_json::from_slice(&read_bounded(&path, MAX_METADATA)?).map_err(err)?;
    validate_intent(&intent, directory)?;
    let raw = read_bounded(&directory.join("release-manifest.json"), MAX_METADATA)?;
    if hash(&raw) != intent.manifest_hash {
        return Err("本地意图的清单哈希不匹配".into());
    }
    let sig = read_text(&directory.join("release-manifest.json.sig"), MAX_SIGNATURE)?;
    let untrusted: ReleaseManifest = serde_json::from_slice(&raw).map_err(err)?;
    let (manifest, artifact) = verify_manifest_with_key(
        &raw,
        &sig,
        &key()?,
        &untrusted.channel,
        &intent.kind,
        env!("CARGO_PKG_VERSION"),
        false,
    )?;
    if manifest.version != intent.version {
        return Err("本地意图版本不匹配".into());
    }
    let package = directory.join(&artifact.file_name);
    verify_artifact(&package, &manifest.version, &artifact, &key()?, None)?;
    #[cfg(windows)]
    {
        return windows_apply(directory, &intent, &package, &artifact);
    }
    #[cfg(not(windows))]
    {
        Err("更新安装只支持 Windows".into())
    }
}
fn helper_authorization_active(directory: &Path) -> Result<(), String> {
    if directory.join("helper-cancelled.json").exists() {
        Err("本次更新助手授权已撤销".into())
    } else {
        Ok(())
    }
}
fn validate_intent(intent: &ApplyIntent, directory: &Path) -> Result<(), String> {
    intent.authority.validate()?;
    let running = fs::canonicalize(std::env::current_exe().map_err(err)?).map_err(err)?;
    if running != directory.join("apply-helper.exe") || file_hash(&running)? != intent.target_hash {
        return Err("更新助手不是本次受信原应用的缓存副本".into());
    }
    let data = fs::canonicalize(&intent.data_directory).map_err(err)?;
    let target = fs::canonicalize(&intent.target).map_err(err)?;
    if intent.schema_version != 1
        || !basename(&intent.update_id)
        || directory != data.join("updates").join(&intent.update_id)
        || target != intent.target
        || intent.engine.exit_code != 0
        || intent.owner.exit_code != 0
        || intent.owner.pid == 0
        || intent.engine.pid == 0
        || intent.owner.creation_time == 0
        || intent.engine.creation_time == 0
        || intent.target_hash != file_hash(&target)?
        || intent.kind != distribution_kind()?
    {
        return Err("更新意图路径/包身份/进程身份不合法".into());
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ReceiptStatus {
    Started,
    InstallerExitedAwaitingStart,
    InstallerFailed,
    Completed,
}
impl ReceiptStatus {
    fn description(self) -> &'static str {
        match self {
            Self::Started => "安装程序已启动，退出结果尚未记录",
            Self::InstallerExitedAwaitingStart => "安装程序已结束，启动结果尚未确认",
            Self::InstallerFailed => "安装程序失败",
            Self::Completed => "完成记录与当前应用不一致",
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Receipt {
    version: String,
    kind: String,
    target: PathBuf,
    sha256: String,
    status: ReceiptStatus,
    exit_code: Option<i32>,
}
#[derive(Default, Debug)]
struct ReceiptFeedback {
    notice: Option<String>,
    error: Option<String>,
}
#[derive(Debug, PartialEq, Eq)]
enum ReceiptOutcome {
    Unrelated,
    Historical,
    Completed,
    Incomplete,
    Failed,
}
fn receipt_outcome(
    receipt: &Receipt,
    current: &str,
    kind: &str,
    target: &Path,
) -> Result<ReceiptOutcome, String> {
    if receipt.target != target || receipt.kind != kind {
        return Ok(ReceiptOutcome::Unrelated);
    }
    if receipt.status == ReceiptStatus::Completed && receipt.version != current {
        return Ok(ReceiptOutcome::Historical);
    }
    if receipt.status == ReceiptStatus::InstallerFailed {
        return Ok(ReceiptOutcome::Failed);
    }
    if receipt.status == ReceiptStatus::Started {
        return Ok(ReceiptOutcome::Incomplete);
    }
    if !matches!(kind,"nsis"|"msi") || !matches!(receipt.exit_code,Some(0|3010)) {
        return Ok(ReceiptOutcome::Failed);
    }
    if receipt.version!=current {return Ok(ReceiptOutcome::Incomplete);}
    Ok(ReceiptOutcome::Completed)
}
fn check_receipts(cache: &Path, current: &str, kind: &str) -> Result<ReceiptFeedback, String> {
    let target = fs::canonicalize(std::env::current_exe().map_err(err)?).map_err(err)?;
    check_receipts_for_target(cache, current, kind, &target)
}
fn check_receipts_for_target(
    cache: &Path,
    current: &str,
    kind: &str,
    target: &Path,
) -> Result<ReceiptFeedback, String> {
    // Select the latest related receipt before rewriting its completion result.
    // Historical successes and unrelated target/kind receipts cannot mask failure.
    let mut paths = fs::read_dir(cache)
        .map_err(err)?
        .flatten()
        .map(|entry| entry.path().join("receipt.json"))
        .filter(|path| path.is_file())
        .map(|path| {
            let modified = fs::metadata(&path)
                .and_then(|v| v.modified())
                .unwrap_or(UNIX_EPOCH);
            (modified, path)
        })
        .collect::<Vec<_>>();
    paths.sort_by(|a, b| b.cmp(a));
    for (_, path) in paths {
        let mut receipt: Receipt = match read_bounded(&path, MAX_METADATA)
            .and_then(|v| serde_json::from_slice(&v).map_err(err))
        {
            Ok(r) => r,
            Err(error) => {
                return Ok(ReceiptFeedback {
                    notice: None,
                    error: Some(format!("上次更新回执无法验证，请核对安装结果：{error}")),
                })
            }
        };
        match receipt_outcome(&receipt, current, kind, target)? {
            ReceiptOutcome::Unrelated | ReceiptOutcome::Historical => continue,
            ReceiptOutcome::Completed => {
                if receipt.status != ReceiptStatus::Completed {
                    receipt.status = ReceiptStatus::Completed;
                    fs::write(&path, serde_json::to_vec(&receipt).map_err(err)?).map_err(err)?;
                }
                return Ok(ReceiptFeedback {
                    notice: Some(format!("已核对更新完成：{current}")),
                    error: None,
                });
            }
            ReceiptOutcome::Incomplete => {
                return Ok(ReceiptFeedback {
                    notice: None,
                    error: Some(format!(
                        "上次更新未确认完成（目标版本 {}，{}），请核对安装结果",
                        receipt.version,
                        receipt.status.description()
                    )),
                })
            }
            ReceiptOutcome::Failed => {
                return Ok(ReceiptFeedback {
                    notice: None,
                    error: Some(format!(
                        "上次更新失败（目标版本 {}，安装退出码 {}），请核对安装结果",
                        receipt.version,
                        receipt
                            .exit_code
                            .map(|code| code.to_string())
                            .unwrap_or_else(|| "未记录".into())
                    )),
                })
            }
        }
    }
    Ok(ReceiptFeedback::default())
}

#[cfg(windows)]
fn current_sid() -> Result<String, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, LocalFree},
        Security::Authorization::ConvertSidToStringSidW,
        Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(format!("Token {}", GetLastError()));
        }
        let mut length = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length);
        let mut data = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        if GetTokenInformation(
            token,
            TokenUser,
            data.as_mut_ptr().cast(),
            length,
            &mut length,
        ) == 0
        {
            CloseHandle(token);
            return Err("无法读取当前用户身份".into());
        }
        let user = &*(data.as_ptr().cast::<TOKEN_USER>());
        let mut sid = std::ptr::null_mut();
        let ok = ConvertSidToStringSidW(user.User.Sid, &mut sid);
        CloseHandle(token);
        if ok == 0 {
            return Err("无法读取当前用户SID".into());
        }
        let mut n = 0;
        while *sid.add(n) != 0 {
            n += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, n));
        LocalFree(sid.cast());
        Ok(text)
    }
}
#[cfg(windows)]
fn secure_directory(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let exe = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot缺失")?)
        .join("System32/icacls.exe");
    let reset = std::process::Command::new(&exe)
        .creation_flags(0x08000000)
        .arg(path)
        .args(["/reset", "/T", "/Q"])
        .output()
        .map_err(err)?;
    if !reset.status.success() {
        return Err("无法清理更新缓存的旧ACL".into());
    }
    let status = std::process::Command::new(exe)
        .creation_flags(0x08000000)
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(format!("*{}:(OI)(CI)F", current_sid()?))
        .arg("*S-1-5-18:(OI)(CI)F")
        .output()
        .map_err(err)?;
    if !status.status.success() {
        return Err("无法为更新缓存设置当前用户ACL".into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn secure_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}
pub fn current_process_identity() -> Result<ProcessIdentity, String> {
    #[cfg(windows)]
    {
        let process = OwnedProcess::open(std::process::id(), None)?;
        Ok(ProcessIdentity {
            pid: std::process::id(),
            creation_time: process.creation_time()?,
            exit_code: 0,
        })
    }
    #[cfg(not(windows))]
    {
        Err("进程身份只支持Windows".into())
    }
}
#[cfg(windows)]
struct OwnedProcess(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl Drop for OwnedProcess {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
#[cfg(windows)]
impl OwnedProcess {
    fn open(pid: u32, creation: Option<u64>) -> Result<Self, String> {
        use windows_sys::Win32::{
            Foundation::GetLastError,
            System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            },
        };
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            );
            if handle.is_null() {
                return Err(format!("无法打开本次进程 {pid}: {}", GetLastError()));
            }
            let p = Self(handle);
            if creation.is_some_and(|v| p.creation_time().ok() != Some(v)) {
                return Err("PID已被重用".into());
            }
            Ok(p)
        }
    }
    fn creation_time(&self) -> Result<u64, String> {
        use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetProcessTimes};
        unsafe {
            let mut c: FILETIME = std::mem::zeroed();
            let mut e = c;
            let mut k = c;
            let mut u = c;
            if GetProcessTimes(self.0, &mut c, &mut e, &mut k, &mut u) == 0 {
                return Err("无法确认进程创建时间".into());
            }
            Ok(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
        }
    }
    fn image_path(&self) -> Result<PathBuf, String> {
        unsafe {
            let mut buffer = vec![0u16; 32768];
            let mut length = buffer.len() as u32;
            if windows_sys::Win32::System::Threading::QueryFullProcessImageNameW(
                self.0,
                0,
                buffer.as_mut_ptr(),
                &mut length,
            ) == 0
            {
                return Err("无法确认进程EXE路径".into());
            }
            fs::canonicalize(PathBuf::from(String::from_utf16_lossy(
                &buffer[..length as usize],
            )))
            .map_err(err)
        }
    }
    fn exit_code(&self) -> Result<u32, String> {
        unsafe {
            let mut code = 0;
            if windows_sys::Win32::System::Threading::GetExitCodeProcess(self.0, &mut code) == 0 {
                return Err("无法读进程退出码".into());
            }
            Ok(code)
        }
    }
    fn wait_normal(&self, budget: Duration) -> Result<(), String> {
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };
        unsafe {
            if WaitForSingleObject(self.0, budget.as_millis().min(u32::MAX as u128) as u32)
                != WAIT_OBJECT_0
            {
                return Err("父应用正常退出等待超时".into());
            }
            if self.exit_code()? != 0 {
                return Err("父应用未正常exit0".into());
            }
            Ok(())
        }
    }
}
#[cfg(windows)]
fn other_target_instance(target: &Path, excluded: &[u32]) -> Result<bool, String> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            Threading::QueryFullProcessImageNameW,
        },
    };
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err("无法检查其他应用实例".into());
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut valid = Process32FirstW(snapshot, &mut entry) != 0;
        let mut found = false;
        while valid {
            if !excluded.contains(&entry.th32ProcessID) {
                if let Ok(process) = OwnedProcess::open(entry.th32ProcessID, None) {
                    let mut buffer = vec![0u16; 32768];
                    let mut len = buffer.len() as u32;
                    if QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut len) != 0
                    {
                        let p = PathBuf::from(String::from_utf16_lossy(&buffer[..len as usize]));
                        if fs::canonicalize(p).is_ok_and(|p| p == target) {
                            found = true;
                            break;
                        }
                    }
                }
            }
            valid = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        Ok(found)
    }
}
#[cfg(windows)]
fn windows_apply(
    directory: &Path,
    intent: &ApplyIntent,
    package: &Path,
    artifact: &Artifact,
) -> Result<(), String> {
    let owner = OwnedProcess::open(intent.owner.pid, Some(intent.owner.creation_time))?;
    if owner.image_path()? != intent.target {
        return Err("更新父进程EXE与目标路径不匹配".into());
    }
    let engine = OwnedProcess::open(intent.engine.pid, Some(intent.engine.creation_time))?;
    if engine.exit_code()? != 0 {
        return Err("Owned engine不是正常exit0".into());
    }
    if other_target_instance(&intent.target, &[intent.owner.pid, std::process::id()])? {
        return Err("另一个同目标应用实例仍在运行".into());
    }
    helper_authorization_active(directory)?;
    write_new(
        &directory.join("helper-ready.json"),
        b"{\"ownerVerified\":true,\"engineExit0\":true}",
    )?;
    owner.wait_normal(Duration::from_secs(90))?;
    helper_authorization_active(directory)?;
    if read_bounded(&directory.join("helper-accepted.json"), MAX_SIGNATURE)?.as_slice()
        != b"accepted"
    {
        return Err("父应用没有确认本次助手握手".into());
    }
    if other_target_instance(&intent.target, &[std::process::id()])? {
        return Err("同目标应用实例阻止更新".into());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(intent.data_directory.join("updates/instances.lock"))
        .map_err(err)?;
    lock.try_lock_exclusive()
        .map_err(|_| "另一个同数据目录应用实例仍在运行")?;
    verify_artifact(package, &intent.version, artifact, &key()?, None)?;
    helper_authorization_active(directory)?;
    if file_hash(&intent.target)? != intent.target_hash {
        return Err("原应用在更新前发生变更".into());
    }
    let mut receipt = Receipt {
        version: intent.version.clone(),
        kind: intent.kind.clone(),
        target: intent.target.clone(),
        sha256: artifact.sha256.clone(),
        status: ReceiptStatus::Started,
        exit_code: None,
    };
    let receipt_path = directory.join("receipt.json");
    write_new(&receipt_path, &serde_json::to_vec(&receipt).map_err(err)?)?;
    use std::os::windows::process::CommandExt;
    let mut command = if intent.kind == "nsis" {
        let mut command=std::process::Command::new(package);command.args(["/UPDATE","/P"]);command
    } else if intent.kind == "msi" {
        let exe=PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot unavailable")?).join("System32/msiexec.exe");
        let mut command=std::process::Command::new(exe);
        command.arg("/i").arg(package).args(["/passive","/norestart"]);
        // Preserve the chosen installation location during upgrades.
        command.arg(format!("INSTALLDIR={}",intent.target.parent().ok_or("Application directory unavailable")?.display()));
        command
    } else {return Err("Unsupported Windows installer.".into());};
    command.creation_flags(0x08000000);
    let code=command.status().map_err(err)?.code().unwrap_or(-1);
    receipt.exit_code=Some(code);
    receipt.status=if matches!(code,0|3010) {ReceiptStatus::InstallerExitedAwaitingStart} else {ReceiptStatus::InstallerFailed};
    fs::write(&receipt_path, serde_json::to_vec(&receipt).map_err(err)?).map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../../tests/fixtures/update-signature-fixture.json")).unwrap()
    }
    fn test_directory(topic: &str) -> PathBuf {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache/tests/desktop")
            .join(format!("20261005-{}-{topic}", identifier()));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn verifies_real_cli_data_global_and_version_signature() {
        let fixture = fixture();
        let public = key().unwrap();
        let sig = signature(fixture["signature"].as_str().unwrap()).unwrap();
        let bytes = fixture["data"].as_str().unwrap().as_bytes();
        public.verify(bytes, &sig, false).unwrap();
        signed_version(&sig, fixture["version"].as_str().unwrap()).unwrap();
        assert!(public.verify(b"tampered", &sig, false).is_err());
        assert!(signed_version(&sig, "99.0.0").is_err());
        let global_tampered = decode_outer(fixture["signature"].as_str().unwrap())
            .unwrap()
            .replace("version:0.9.0-rc.1", "version:99.0.0");
        let altered = Signature::decode(&global_tampered).unwrap();
        assert!(public.verify(bytes, &altered, false).is_err());
        let mut verifier = public.verify_stream(&sig).unwrap();
        for chunk in bytes.chunks(3) {
            verifier.update(chunk);
        }
        verifier.finalize().unwrap();
    }
    #[test]
    fn artifact_sha_size_and_signed_version_are_all_required() {
        let fixture = fixture();
        let bytes = fixture["data"].as_str().unwrap().as_bytes();
        let mut artifact = Artifact {
            target: TARGET.into(),
            kind: "msi".into(),
            file_name: "fixture.txt".into(),
            size: bytes.len() as u64,
            sha256: hash(bytes),
            signature: fixture["signature"].as_str().unwrap().into(),
            url: String::new(),
            installer_product_version: None,
        };
        verify_reader(bytes, "0.9.0-rc.1", &artifact, &key().unwrap(), None).unwrap();
        assert!(verify_reader(bytes, "99.0.0", &artifact, &key().unwrap(), None).is_err());
        artifact.sha256 = "0".repeat(64);
        assert!(verify_reader(bytes, "0.9.0-rc.1", &artifact, &key().unwrap(), None).is_err());
        artifact.sha256 = hash(bytes);
        artifact.size += 1;
        assert!(verify_reader(bytes, "0.9.0-rc.1", &artifact, &key().unwrap(), None).is_err());
    }
    #[test]
    fn manifest_duplicate_fields_and_unsigned_payloads_are_rejected() {
        let fixture = fixture();
        assert!(verify_manifest_with_key(
            fixture["data"].as_str().unwrap().as_bytes(),
            fixture["signature"].as_str().unwrap(),
            &key().unwrap(),
            "candidate",
            "msi",
            "0.8.2",
            false
        )
        .is_err());
        assert!(serde_json::from_str::<ReleaseManifest>(
            r#"{"schemaVersion":1,"schemaVersion":1}"#
        )
        .is_err());
        assert!(semver::Version::parse("0.9.0+new")
            .unwrap()
            .cmp_precedence(&semver::Version::parse("0.9.0+old").unwrap())
            .is_eq());
    }
    #[test]
    fn anonymous_mirror_headers_have_no_credentials() {
        let url = format!("https://github.com/{REPOSITORY}/releases/download/v0.9.0-rc.1/file.exe");
        for source in sources(&url).unwrap() {
            let request = transport().unwrap().get(&source).build().unwrap();
            for name in ["authorization", "cookie", "proxy-authorization"] {
                assert!(!request.headers().contains_key(name));
            }
            if source.contains("gh-proxy.org") {
                assert_eq!(source, format!("https://gh-proxy.org/{url}"));
            }
        }
    }
    #[test]
    fn zip_rejects_absolute_parent_backslash_and_symlink_entries() {
        use std::io::Cursor;
        use zip::write::SimpleFileOptions;
        for name in [
            "../evil.exe",
            "/absolute.exe",
            "C:\\evil.exe",
            "nested/file.exe",
        ] {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for entry in [
                name,
                "release-manifest.json",
                "release-manifest.json.sig",
                "valid.exe",
            ] {
                writer
                    .start_file(
                        entry,
                        SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored),
                    )
                    .unwrap();
                writer.write_all(b"a").unwrap();
            }
            let mut archive =
                zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
            assert!(validate_zip_entries(&mut archive).is_err());
        }
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .add_symlink("link", "target", SimpleFileOptions::default())
            .unwrap();
        for entry in [
            "release-manifest.json",
            "release-manifest.json.sig",
            "valid.exe",
        ] {
            writer
                .start_file(
                    entry,
                    SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
                )
                .unwrap();
            writer.write_all(b"a").unwrap();
        }
        let mut archive =
            zip::ZipArchive::new(Cursor::new(writer.finish().unwrap().into_inner())).unwrap();
        assert!(validate_zip_entries(&mut archive).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn process_identity_rejects_pid_reuse_and_detects_target_instance() {
        let identity = current_process_identity().unwrap();
        OwnedProcess::open(identity.pid, Some(identity.creation_time)).unwrap();
        assert!(OwnedProcess::open(identity.pid, Some(identity.creation_time + 1)).is_err());
        let target = fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert!(other_target_instance(&target, &[]).unwrap());
    }
    #[test]
    fn data_instance_lock_and_cancel_commit_are_exclusive() {
        let directory = test_directory("instance-lock");
        let service = UpdaterService::new(directory.clone()).unwrap();
        let other = UpdaterService::new(directory.clone()).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(directory.join("updates/instances.lock"))
            .unwrap();
        assert!(lock.try_lock_exclusive().is_err());
        let (id, _) = service.start("candidate").unwrap();
        service.cancel(id).unwrap();
        assert!(service.mark_ready().is_err());
        assert_eq!(service.status().unwrap().phase, "cancelled");
        drop(service);
        drop(other);
        lock.try_lock_exclusive().unwrap();
    }
    #[test]
    fn failed_cached_verification_unlocks_update_selection_and_late_helper_is_revoked() {
        let directory = test_directory("failed-verification");
        let service = UpdaterService::new(directory.clone()).unwrap();
        service.set(|s| {
            s.update_id = Some("test".into());
            s.target_version = Some("9.0.0".into());
            s.phase = "ready".into();
            s.verified = true;
        });
        assert!(service.prepare_apply("test", "9.0.0").is_err());
        assert_eq!(service.status().unwrap().phase, "failed");
        assert!(service.start("candidate").is_ok());
        assert!(helper_authorization_active(&directory).is_ok());
        write_new(&directory.join("helper-cancelled.json"), b"{}").unwrap();
        assert!(helper_authorization_active(&directory).is_err());
    }
    #[test]
    fn rejects_urls_credentials_and_cross_repository() {
        for url in [
            "http://github.com/Holy1one/Micro-CT-Workstation/releases/download/v1/a.exe",
            "https://token@github.com/Holy1one/Micro-CT-Workstation/releases/download/v1/a.exe",
            "https://github.com/evil/repo/releases/download/v1/a.exe",
            "https://github.com/Holy1one/Micro-CT-Workstation/releases/download/v1/a.exe?token=bad",
        ] {
            assert!(validate_asset_url(url, None).is_err());
        }
    }
    fn receipt_fixture(
        target: &Path,
        kind: &str,
        status: ReceiptStatus,
        exit_code: Option<i32>,
    ) -> Receipt {
        Receipt {
            version: "0.9.0-rc.2".into(),
            kind: kind.into(),
            target: target.into(),
            sha256: file_hash(target).unwrap(),
            status,
            exit_code,
        }
    }
    fn save_receipt(cache: &Path, name: &str, receipt: &Receipt) -> PathBuf {
        let directory = cache.join(name);
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("receipt.json");
        fs::write(&path, serde_json::to_vec(receipt).unwrap()).unwrap();
        path
    }
    #[test]
    fn installer_completion_receipts_produce_notice_without_error() {
        for (kind, status, code) in [
            ("nsis", ReceiptStatus::InstallerExitedAwaitingStart, 0),
            ("msi", ReceiptStatus::InstallerExitedAwaitingStart, 3010),
        ] {
            let directory = test_directory("receipt-success");
            let target = directory.join("current.exe");
            fs::write(&target, b"current-app").unwrap();
            let receipt = receipt_fixture(&target, kind, status, Some(code));
            let path = save_receipt(&directory, "job", &receipt);
            let feedback =
                check_receipts_for_target(&directory, "0.9.0-rc.2", kind, &target).unwrap();
            assert!(feedback
                .notice
                .as_deref()
                .is_some_and(|v| v.contains("已核对更新完成")));
            assert!(feedback.error.is_none());
            let completed: Receipt = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(completed.status, ReceiptStatus::Completed);
            let again = check_receipts_for_target(&directory, "0.9.0-rc.2", kind, &target).unwrap();
            assert!(again.notice.is_some());
            assert!(again.error.is_none());
        }
    }
    #[test]
    fn failed_and_unfinished_receipts_never_become_success_from_matching_app_version() {
        for (status, code) in [
            (ReceiptStatus::InstallerFailed, Some(1603)),
            (ReceiptStatus::Started, None),
            (ReceiptStatus::InstallerExitedAwaitingStart, Some(1603)),
            (ReceiptStatus::Completed, Some(1603)),
        ] {
            let directory = test_directory("receipt-failed");
            let target = directory.join("current.exe");
            fs::write(&target, b"matching-current-app").unwrap();
            let receipt = receipt_fixture(&target, "nsis", status, code);
            let path = save_receipt(&directory, "job", &receipt);
            let feedback =
                check_receipts_for_target(&directory, "0.9.0-rc.2", "nsis", &target).unwrap();
            assert!(feedback.notice.is_none());
            assert!(feedback.error.is_some());
            let after: Receipt = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
            assert_eq!(after.status, status);
        }
    }
    #[test]
    fn installer_receipt_ignores_other_target_and_kind_and_requires_version() {
        let directory=test_directory("receipt-identity");let target=directory.join("current.exe");
        fs::write(&target,b"current-app").unwrap();
        let mut receipt=receipt_fixture(&target,"msi",ReceiptStatus::InstallerExitedAwaitingStart,Some(0));
        receipt.version="0.9.0-rc.1".into();
        assert_eq!(receipt_outcome(&receipt,"0.9.0-rc.2","msi",&target).unwrap(),ReceiptOutcome::Incomplete);
        receipt.status=ReceiptStatus::Completed;
        assert_eq!(receipt_outcome(&receipt,"0.9.0-rc.2","msi",&target).unwrap(),ReceiptOutcome::Historical);
        assert_eq!(receipt_outcome(&receipt,"0.9.0-rc.2","nsis",&target).unwrap(),ReceiptOutcome::Unrelated);
        assert_eq!(receipt_outcome(&receipt,"0.9.0-rc.2","msi",&directory.join("other.exe")).unwrap(),ReceiptOutcome::Unrelated);
    }
    #[test]
    fn latest_discovery_rejects_missing_invalid_and_prerelease_versions() {
        assert_eq!(latest_release(br#"{"tag_name":"v0.9.6","draft":false,"prerelease":false}"#).unwrap().1.to_string(),"0.9.6");
        for raw in [br#"{}"#.as_slice(),br#"{"tag_name":"v0.9.7-rc.1","draft":false,"prerelease":true}"#,br#"{"tag_name":"stable","draft":false,"prerelease":false}"#] {
            assert!(latest_release(raw).is_err());
        }
        let address=format!("https://github.com/{REPOSITORY}/releases/tag/v0.9.6");
        assert_eq!(latest_redirect(&address).unwrap().1.to_string(),"0.9.6");
        for invalid in [address.replace("https:","http:"),address.replace("github.com","untrusted.invalid"),
            format!("{address}?token=unexpected"),address.replace("releases/tag","releases/download"),
            address.replace(REPOSITORY,"other/repository")] {assert!(latest_redirect(&invalid).is_err());}
    }
    #[test]
    fn download_requires_available_correlated_selection_and_cannot_apply_metadata() {
        let service=UpdaterService::new(test_directory("download-selection")).unwrap();
        let (id,_)=service.start("stable").unwrap();service.finish(Ok(())).unwrap();
        assert!(service.begin_download(&id).is_err());
        service.set(|state|state.phase="available".into());
        assert!(service.begin_download("stale-id").is_err());assert!(service.begin_download(&id).is_err());
        assert!(service.prepare_apply(&id,"99.0.0").is_err());
        service.cancel(id.clone()).unwrap();assert!(service.begin_download(&id).is_err());
    }
    #[test]
    fn feedback_clear_check_cancel_and_failure_have_distinct_notice_error_fields() {
        let directory = test_directory("feedback-clear");
        let service = UpdaterService::new(directory).unwrap();
        service.set(|s| {
            s.notice = Some("old completion".into());
            s.error = Some("old error".into());
        });
        let cleared = service.clear_feedback().unwrap();
        assert!(cleared.notice.is_none() && cleared.error.is_none());
        service.set(|s| s.notice = Some("old completion".into()));
        let (id, _) = service.start("candidate").unwrap();
        assert!(service.status().unwrap().notice.is_none());
        service.set(|s| {
            s.notice = Some("stale completion".into());
            s.error = Some("stale failure".into());
        });
        let cancelled = service.cancel(id).unwrap();
        assert!(cancelled.notice.is_none() && cancelled.error.is_none());
        let finished = service
            .finish(Err("cancelled network wait".into()))
            .unwrap();
        assert_eq!(finished.phase, "cancelled");
        assert!(finished.notice.is_none() && finished.error.is_none());
        service.start("candidate").unwrap();
        let failed = service.finish(Err("signature mismatch".into())).unwrap();
        assert_eq!(failed.error.as_deref(), Some("signature mismatch"));
        assert!(failed.notice.is_none());
        let dto = serde_json::to_value(failed).unwrap();
        assert!(dto.get("notice").is_some() && dto.get("error").is_some());
    }
    #[test]
    fn validates_paths_and_msi_mapping() {
        for value in ["../evil.exe", "a/b", "C:\\evil.exe", "evil..exe", "a\n"] {
            assert!(!basename(value));
        }
        assert!(numeric_msi_version("0.9.1").is_ok());
        assert!(numeric_msi_version("0.9.0.1").is_err());
        assert!(numeric_msi_version("0.9.65536").is_err());
    }
    #[test]
    fn authority_unknown_and_exit_zero_are_not_safe() {
        let mut report = UpdateAuthority {
            session_id: "session".into(),
            operation_id: "op".into(),
            status: "ready".into(),
            ready: true,
            created_at: "now".into(),
            completed_at: Some("now".into()),
            turntable_stop: "never-owned".into(),
            xray_off: "never-owned".into(),
            camera_cleanup: "never-owned".into(),
            no_active_transactions: true,
            maintenance_locked: true,
            error: None,
        };
        assert!(report.validate().is_ok());
        report.xray_off = "unknown".into();
        assert!(report.validate().is_err());
        report.xray_off = "confirmed".into();
        report.operation_id.clear();
        assert!(report.validate().is_err());
    }
    #[test]
    fn signatures_require_unique_version() {
        let sig=Signature::decode("untrusted comment: signature from minisign secret key\nRUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\ntrusted comment: timestamp:1633700835\tfile:test\tprehashed\nwLMDjy9FLAuxZ3q4NlEvkgtyhrr0gtTu6KC4KBJdITbbOeAi1zBIYo0v4iTgt8jJpIidRJnp94ABQkJAgAooBQ==").unwrap();
        assert!(signed_version(&sig, "1.0.0").is_err());
    }
    #[test]
    fn redirect_policy_is_https_allowlisted() {
        for url in [
            "http://github.com/a",
            "https://evil.example/a",
            "https://token@gh-proxy.org/a",
        ] {
            assert!(!allowed_transport(&reqwest::Url::parse(url).unwrap()));
        }
        assert!(allowed_transport(
            &reqwest::Url::parse("https://gh-proxy.org/https://github.com/a").unwrap()
        ));
    }
}
