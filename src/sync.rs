//! 同步: the whole library (documents, assets and `.quiet` settings) mirrored
//! to WebDAV or an S3-compatible bucket. Manual upload/download and a periodic
//! two-way sync share one engine; credentials and the last-sync state stay in
//! this computer's application data, never in the library.

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::hash::{Hash as _, Hasher as _};
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MANIFEST_KEY: &str = ".quietnext-sync/manifest.json";
const USER_AGENT: &str = concat!("QuietNext/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
    Both,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// "webdav" or "s3"; empty is WebDAV.
    pub protocol: String,
    pub webdav_url: String,
    pub webdav_user: String,
    pub webdav_password: String,
    pub s3_endpoint: String,
    pub s3_region: String,
    pub s3_bucket: String,
    pub s3_prefix: String,
    pub s3_access_key: String,
    pub s3_secret_key: String,
    pub auto: bool,
    pub last_sync: u64,
    /// A short, already-translated result line from the last run.
    pub last_status: String,
}

impl Config {
    pub fn is_s3(&self) -> bool { self.protocol == "s3" }
    pub fn validate(&self) -> Result<()> {
        if self.is_s3() {
            if self.s3_endpoint.trim().is_empty() { bail!("请填写 S3 端点 / Enter an S3 endpoint") }
            if self.s3_bucket.trim().is_empty() { bail!("请填写存储桶 / Enter a bucket") }
            if self.s3_access_key.trim().is_empty() { bail!("请填写 Access Key") }
            if self.s3_secret_key.is_empty() { bail!("请填写 Secret Key") }
        } else if self.webdav_url.trim().is_empty() {
            bail!("请填写 WebDAV 地址 / Enter a WebDAV URL")
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Report {
    pub uploaded: u32,
    pub downloaded: u32,
    pub conflicts: u32,
    pub skipped: u32,
}

static BUSY: AtomicBool = AtomicBool::new(false);
static AUTO_LAST: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
pub fn busy() -> bool { BUSY.load(Ordering::Acquire) }

/// True at most once per five minutes, so several windows do not all sync.
pub fn auto_due() -> bool {
    let t = now();
    let mut last = AUTO_LAST.lock().unwrap();
    if t >= last.saturating_add(300) { *last = t; true } else { false }
}

/// One sync at a time across all windows. Dropping it releases the lock.
pub struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) { BUSY.store(false, Ordering::Release); }
}

pub fn begin() -> Option<BusyGuard> {
    BUSY.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok().map(|_| BusyGuard)
}

/// `quiet-md/sync/<library>` under the local application data folder.
fn data_dir(root: &Path) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.to_string_lossy().hash(&mut h);
    dirs::data_local_dir().or_else(dirs::home_dir).unwrap_or_else(std::env::temp_dir)
        .join("quiet-md").join("sync").join(format!("{:016x}", h.finish()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let tmp = path.with_file_name(format!(".{}.qsync", path.file_name().and_then(|n| n.to_str()).unwrap_or("sync")));
    fs::write(&tmp, bytes)?;
    if path.exists() { fs::remove_file(path)?; }
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn config(root: &Path) -> Config {
    fs::read(data_dir(root).join("config.json")).ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_config(root: &Path, cfg: &Config) -> Result<()> {
    write_atomic(&data_dir(root).join("config.json"), &serde_json::to_vec_pretty(cfg)?)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Sig {
    size: u64,
    /// Nanoseconds since the epoch: two saves in the same second still differ.
    modified: u64,
    /// Content fingerprint, so a save inside one file-system timestamp tick
    /// (and with the same size) is still noticed.
    #[serde(default)]
    hash: u64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Manifest {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    updated: u64,
    #[serde(default)]
    files: BTreeMap<String, Sig>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    local: BTreeMap<String, Sig>,
    #[serde(default)]
    remote: BTreeMap<String, Sig>,
    #[serde(default)]
    last_sync: u64,
}

fn state_path(root: &Path) -> PathBuf { data_dir(root).join("state.json") }
fn load_state(root: &Path) -> State {
    fs::read(state_path(root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

fn sig_of(path: &Path) -> Option<Sig> {
    let m = fs::metadata(path).ok()?;
    if !m.is_file() { return None }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut file = fs::File::open(path).ok()?;
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk).ok()?;
        if n == 0 { break }
        chunk[..n].hash(&mut hasher);
    }
    Some(Sig {
        size: m.len(),
        modified: m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64,
        hash: hasher.finish(),
    })
}

fn skipped(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    rel == ".quiet/.lock"
        || name.starts_with('~') || name == ".DS_Store" || name == "Thumbs.db"
        || name.ends_with(".tmp") || name.contains(".tmp.")
}

fn scan(root: &Path) -> BTreeMap<String, Sig> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Sig>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if meta.is_dir() {
                if name == "target" || name == "node_modules" { continue }
                walk(root, &path, out);
            } else if meta.is_file() && !skipped(&rel) && let Some(sig) = sig_of(&path) {
                out.insert(rel, sig);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn safe_rel(rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || rel.contains('\\') && cfg!(not(windows)) { bail!("无效的同步路径 / Invalid sync path: {rel}") }
    if !p.components().all(|c| matches!(c, Component::Normal(_))) { bail!("无效的同步路径 / Invalid sync path: {rel}") }
    Ok(p.to_path_buf())
}

fn remote_path(root: &Path, rel: &str) -> Result<PathBuf> {
    Ok(root.join(safe_rel(rel)?))
}

fn backup(root: &Path, rel: &str) -> Result<()> {
    let src = remote_path(root, rel)?;
    if !src.is_file() { return Ok(()) }
    let dst = data_dir(root).join("backups").join(now().to_string()).join(safe_rel(rel)?);
    if let Some(parent) = dst.parent() { fs::create_dir_all(parent)?; }
    fs::copy(&src, &dst)?;
    Ok(())
}

fn conflict_rel(root: &Path, rel: &str) -> String {
    let (y, m, d, h, mi) = crate::time::civil(now());
    let stamp = format!("{y:04}-{m:02}-{d:02} {h:02}{mi:02}");
    let p = Path::new(rel);
    let parent = p.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_string_lossy().replace('\\', "/"));
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "note".into());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let base = if parent.as_deref().is_some_and(|p| !p.is_empty()) { format!("{}/{} (cloud conflict {stamp}){ext}", parent.unwrap(), stem) } else { format!("{stem} (cloud conflict {stamp}){ext}") };
    let mut candidate = base;
    let mut n = 2;
    while remote_path(root, &candidate).is_ok_and(|p| p.exists()) {
        let parent = p.parent().filter(|p| !p.as_os_str().is_empty()).map(|p| p.to_string_lossy().replace('\\', "/"));
        let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        candidate = if parent.as_deref().is_some_and(|p| !p.is_empty()) { format!("{}/{} (cloud conflict {stamp} {n}){ext}", parent.unwrap(), stem) } else { format!("{stem} (cloud conflict {stamp} {n}){ext}") };
        n += 1;
    }
    candidate
}

fn write_download(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let path = remote_path(root, rel)?;
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    let tmp = path.with_file_name(format!(".{}.qsync", path.file_name().and_then(|n| n.to_str()).unwrap_or("file")));
    fs::write(&tmp, bytes)?;
    if path.exists() { fs::remove_file(&path)?; }
    fs::rename(tmp, path)?;
    Ok(())
}

fn encode_segment(s: &str) -> String {
    let mut out = String::new();
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
            _ => { let _ = write!(out, "%{b:02X}"); }
        }
    }
    out
}

fn encode_path(path: &str) -> String { path.split('/').map(encode_segment).collect::<Vec<_>>().join("/") }

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(180))
        .timeout_write(Duration::from_secs(180))
        .build()
}

fn read_body(resp: ureq::Response) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

struct Dav {
    base: String,
    auth: Option<String>,
    agent: ureq::Agent,
    made: std::cell::RefCell<HashSet<String>>,
}

impl Dav {
    fn new(cfg: &Config) -> Result<Self> {
        let mut base = cfg.webdav_url.trim().trim_end_matches('/').to_string();
        if !base.starts_with("http://") && !base.starts_with("https://") { base = format!("https://{base}") }
        url::Url::parse(&base).context("WebDAV 地址无效 / Invalid WebDAV URL")?;
        let auth = (!cfg.webdav_user.is_empty() || !cfg.webdav_password.is_empty()).then(|| {
            format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", cfg.webdav_user, cfg.webdav_password)))
        });
        Ok(Self { base, auth, agent: agent(), made: Default::default() })
    }
    fn url(&self, key: &str) -> String { format!("{}/{}", self.base, encode_path(key)) }
    fn auth(&self, req: ureq::Request) -> ureq::Request {
        let req = req.set("User-Agent", USER_AGENT);
        match &self.auth { Some(a) => req.set("Authorization", a), None => req }
    }
    fn mkcol(&self, dir: &str) -> Result<()> {
        if dir.is_empty() || self.made.borrow().contains(dir) { return Ok(()) }
        let mut url = format!("{}/{}", self.base, encode_path(dir));
        if !url.ends_with('/') { url.push('/'); }
        match self.auth(self.agent.request("MKCOL", &url)).call() {
            Ok(_) => {}
            Err(ureq::Error::Status(405 | 301, _)) => {}
            Err(e) => return Err(anyhow!("创建 WebDAV 目录失败 / WebDAV MKCOL failed: {e}")),
        }
        self.made.borrow_mut().insert(dir.to_string());
        Ok(())
    }
    fn ensure_parent(&self, key: &str) -> Result<()> {
        let mut current = String::new();
        for part in key.split('/').rev().skip(1).collect::<Vec<_>>().into_iter().rev() {
            if !current.is_empty() { current.push('/'); }
            current.push_str(part);
            self.mkcol(&current)?;
        }
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self.auth(self.agent.get(&self.url(key))).call() {
            Ok(r) => Ok(Some(read_body(r)?)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(anyhow!("WebDAV 下载失败 / WebDAV GET failed: {e}")),
        }
    }
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        self.ensure_parent(key)?;
        match self.auth(self.agent.put(&self.url(key))).set("Content-Type", "application/octet-stream").send_bytes(bytes) {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow!("WebDAV 上传失败 / WebDAV PUT failed: {e}")),
        }
    }
}

struct S3 {
    scheme: String,
    host: String,
    base_path: String,
    bucket: String,
    prefix: String,
    region: String,
    access: String,
    secret: String,
    /// AWS wants the bucket in the host; MinIO and other compatible stores use a path.
    virtual_host: bool,
    agent: ureq::Agent,
}

impl S3 {
    fn new(cfg: &Config) -> Result<Self> {
        let mut endpoint = cfg.s3_endpoint.trim().to_string();
        if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") { endpoint = format!("https://{endpoint}") }
        let base = url::Url::parse(&endpoint).context("S3 端点无效 / Invalid S3 endpoint")?;
        if !matches!(base.scheme(), "http" | "https") { bail!("S3 端点必须是 http(s) / S3 endpoint must be http(s)") }
        let host = base.host_str().ok_or_else(|| anyhow!("S3 端点缺少主机名 / S3 endpoint has no host"))?;
        let aws = host == "s3.amazonaws.com" || (host.starts_with("s3.") && host.ends_with(".amazonaws.com"));
        let bucket = cfg.s3_bucket.trim().trim_matches('/').to_string();
        let host = match (aws, base.port()) {
            (true, _) => format!("{}.{}", bucket, host),
            (false, Some(p)) => format!("{host}:{p}"),
            (false, None) => host.to_string(),
        };
        Ok(Self {
            scheme: base.scheme().to_string(),
            host,
            base_path: base.path().trim_end_matches('/').to_string(),
            bucket,
            prefix: cfg.s3_prefix.trim().trim_matches('/').to_string(),
            region: if cfg.s3_region.trim().is_empty() { "us-east-1".into() } else { cfg.s3_region.trim().to_string() },
            access: cfg.s3_access_key.trim().to_string(),
            secret: cfg.s3_secret_key.clone(),
            virtual_host: aws,
            agent: agent(),
        })
    }
    fn full_key(&self, key: &str) -> String {
        if self.prefix.is_empty() { key.to_string() } else { format!("{}/{}", self.prefix, key) }
    }
    fn url_and_path(&self, key: &str) -> (String, String) {
        let full = self.full_key(key);
        let path = if self.virtual_host {
            format!("{}/{}", self.base_path, encode_path(&full))
        } else {
            format!("{}/{}/{}", self.base_path, encode_segment(&self.bucket), encode_path(&full))
        };
        (format!("{}://{}{}", self.scheme, self.host, path), path)
    }
    fn headers(&self, method: &str, path: &str, body: &[u8]) -> Vec<(&'static str, String)> {
        let amz_date = utc_stamp(now());
        let date = &amz_date[..8];
        let payload = hex::encode(Sha256::digest(body));
        let canonical_headers = format!("host:{}\nx-amz-content-sha256:{payload}\nx-amz-date:{amz_date}\n", self.host);
        let signed = "host;x-amz-content-sha256;x-amz-date";
        let canonical = format!("{method}\n{path}\n\n{canonical_headers}\n{signed}\n{payload}");
        let scope = format!("{date}/{}/s3/aws4_request", self.region);
        let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", hex::encode(Sha256::digest(canonical.as_bytes())));
        let k_date = hmac(format!("AWS4{}", self.secret).as_bytes(), date.as_bytes());
        let k_region = hmac(&k_date, self.region.as_bytes());
        let k_service = hmac(&k_region, b"s3");
        let k_signing = hmac(&k_service, b"aws4_request");
        let signature = hex::encode(hmac(&k_signing, to_sign.as_bytes()));
        let authorization = format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}", self.access);
        vec![("x-amz-content-sha256", payload), ("x-amz-date", amz_date), ("Authorization", authorization)]
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let (url, path) = self.url_and_path(key);
        let mut req = self.agent.get(&url).set("User-Agent", USER_AGENT);
        for (name, value) in self.headers("GET", &path, b"") { req = req.set(name, &value); }
        match req.call() {
            Ok(r) => Ok(Some(read_body(r)?)),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(e) => Err(anyhow!("S3 下载失败 / S3 GET failed: {e}")),
        }
    }
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let (url, path) = self.url_and_path(key);
        let mut req = self.agent.put(&url).set("User-Agent", USER_AGENT).set("Content-Type", "application/octet-stream");
        for (name, value) in self.headers("PUT", &path, bytes) { req = req.set(name, &value); }
        match req.send_bytes(bytes) {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow!("S3 上传失败 / S3 PUT failed: {e}")),
        }
    }
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// UTC `YYYYMMDDTHHMMSSZ`, independent of the displayed local time.
fn utc_stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, (rem % 3600) / 60, rem % 60)
}

enum Remote { Dav(Dav), S3(S3) }

impl Remote {
    fn new(cfg: &Config) -> Result<Self> {
        cfg.validate()?;
        Ok(if cfg.is_s3() { Self::S3(S3::new(cfg)?) } else { Self::Dav(Dav::new(cfg)?) })
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match self { Self::Dav(d) => d.get(key), Self::S3(s) => s.get(key) }
    }
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        match self { Self::Dav(d) => d.put(key, bytes), Self::S3(s) => s.put(key, bytes) }
    }
}

fn load_manifest(remote: &Remote) -> Result<Manifest> {
    match remote.get(MANIFEST_KEY)? {
        Some(bytes) => {
            let mut m: Manifest = serde_json::from_slice(&bytes).context("云端同步清单损坏 / Corrupt remote manifest")?;
            m.files.retain(|rel, _| !skipped(rel));
            Ok(m)
        }
        None => Ok(Manifest { version: 1, updated: now(), files: BTreeMap::new() }),
    }
}

fn save_manifest(remote: &Remote, manifest: &Manifest) -> Result<()> {
    remote.put(MANIFEST_KEY, &serde_json::to_vec_pretty(manifest)?)
}

fn upload_file(rel: &str, bytes: &[u8], remote: &Remote) -> Result<()> {
    remote.put(rel, bytes)
}

fn download_file(root: &Path, local: &mut BTreeMap<String, Sig>, remote: &Remote, rel: &str, bytes: &[u8], report: &mut Report) -> Result<()> {
    backup(root, rel)?;
    write_download(root, rel, bytes)?;
    if let Some(sig) = sig_of(&remote_path(root, rel)?) { local.insert(rel.to_string(), sig); }
    report.downloaded += 1;
    let _ = remote;
    Ok(())
}

fn read_remote(remote: &Remote, rel: &str) -> Result<Vec<u8>> {
    remote.get(rel)?.ok_or_else(|| anyhow!("云端文件不存在 / Remote file is missing: {rel}"))
}

fn same_bytes(path: &Path, bytes: &[u8]) -> bool {
    fs::read(path).is_ok_and(|local| local == bytes)
}

/// Run one sync. Blocking (HTTP and file I/O): call it on the background executor.
pub fn run(root: &Path, cfg: &Config, direction: Direction) -> Result<Report> {
    let remote = Remote::new(cfg)?;
    let mut manifest = load_manifest(&remote)?;
    let mut local = scan(root);
    let mut state = load_state(root);
    let mut report = Report::default();
    let mut errors: Vec<String> = Vec::new();

    match direction {
        Direction::Upload => {
            for (rel, sig) in &local {
                if manifest.files.get(rel) == Some(sig) { report.skipped += 1; continue }
                match fs::read(remote_path(root, rel)?) {
                    Ok(bytes) => match upload_file(rel, &bytes, &remote) {
                        Ok(()) => { manifest.files.insert(rel.clone(), *sig); report.uploaded += 1; }
                        Err(e) => errors.push(format!("{rel}: {e}")),
                    },
                    Err(e) => errors.push(format!("{rel}: {e}")),
                }
            }
        }
        Direction::Download => {
            let files = manifest.files.clone();
            for (rel, sig) in files {
                if state.remote.get(&rel) == Some(&sig) && local.contains_key(&rel) { report.skipped += 1; continue }
                match read_remote(&remote, &rel) {
                    Ok(bytes) => if let Err(e) = download_file(root, &mut local, &remote, &rel, &bytes, &mut report) { errors.push(format!("{rel}: {e}")) },
                    Err(e) => errors.push(format!("{rel}: {e}")),
                }
            }
        }
        Direction::Both => {
            let mut keys: Vec<String> = manifest.files.keys().chain(local.keys()).chain(state.local.keys()).chain(state.remote.keys()).cloned().collect();
            keys.sort();
            keys.dedup();
            for rel in keys {
                let l = local.get(&rel).copied();
                let r = manifest.files.get(&rel).copied();
                let sl = state.local.get(&rel).copied();
                let sr = state.remote.get(&rel).copied();
                let local_changed = l != sl;
                let remote_changed = r != sr;
                match (l, r) {
                    (Some(ls), Some(rs)) => {
                        if ls == rs || (!local_changed && !remote_changed) { report.skipped += 1; continue }
                        if local_changed && !remote_changed {
                            match fs::read(remote_path(root, &rel)?) {
                                Ok(bytes) => match upload_file(&rel, &bytes, &remote) {
                                    Ok(()) => { manifest.files.insert(rel.clone(), ls); report.uploaded += 1; }
                                    Err(e) => errors.push(format!("{rel}: {e}")),
                                },
                                Err(e) => errors.push(format!("{rel}: {e}")),
                            }
                        } else if !local_changed && remote_changed {
                            match read_remote(&remote, &rel) {
                                Ok(bytes) => if let Err(e) = download_file(root, &mut local, &remote, &rel, &bytes, &mut report) { errors.push(format!("{rel}: {e}")) },
                                Err(e) => errors.push(format!("{rel}: {e}")),
                            }
                        } else {
                            // Both sides changed. Keep the local file and save the cloud copy beside it.
                            match read_remote(&remote, &rel) {
                                Ok(bytes) => {
                                    let local_path = remote_path(root, &rel)?;
                                    if !same_bytes(&local_path, &bytes) {
                                        let conflict = conflict_rel(root, &rel);
                                        if let Err(e) = write_download(root, &conflict, &bytes) {
                                            errors.push(format!("{rel}: {e}"));
                                            continue;
                                        }
                                        if let Some(sig) = sig_of(&remote_path(root, &conflict)?) {
                                            match upload_file(&conflict, &bytes, &remote) {
                                                Ok(()) => { manifest.files.insert(conflict.clone(), sig); local.insert(conflict, sig); }
                                                Err(e) => errors.push(format!("{conflict}: {e}")),
                                            }
                                        }
                                        report.conflicts += 1;
                                    }
                                    match fs::read(&local_path) {
                                        Ok(local_bytes) => match upload_file(&rel, &local_bytes, &remote) {
                                            Ok(()) => { manifest.files.insert(rel.clone(), ls); report.uploaded += 1; }
                                            Err(e) => errors.push(format!("{rel}: {e}")),
                                        },
                                        Err(e) => errors.push(format!("{rel}: {e}")),
                                    }
                                }
                                Err(e) => errors.push(format!("{rel}: {e}")),
                            }
                        }
                    }
                    (Some(ls), None) => {
                        // The cloud deleted it and local did not change: leave both as they are.
                        if sr.is_some() && !local_changed { report.skipped += 1; continue }
                        match fs::read(remote_path(root, &rel)?) {
                            Ok(bytes) => match upload_file(&rel, &bytes, &remote) {
                                Ok(()) => { manifest.files.insert(rel.clone(), ls); report.uploaded += 1; }
                                Err(e) => errors.push(format!("{rel}: {e}")),
                            },
                            Err(e) => errors.push(format!("{rel}: {e}")),
                        }
                    }
                    (None, Some(rs)) => {
                        // Local deleted it and the cloud did not change: keep the cloud copy.
                        if sl.is_some() && !remote_changed { report.skipped += 1; continue }
                        match read_remote(&remote, &rel) {
                            Ok(bytes) => if let Err(e) = download_file(root, &mut local, &remote, &rel, &bytes, &mut report) { errors.push(format!("{rel}: {e}")) },
                            Err(e) => errors.push(format!("{rel}: {e}")),
                        }
                        let _ = rs;
                    }
                    (None, None) => {}
                }
            }
        }
    }

    manifest.version = 1;
    manifest.updated = now();
    save_manifest(&remote, &manifest)?;
    if !errors.is_empty() {
        return Err(anyhow!("{}", errors.join("\n")));
    }
    match direction {
        Direction::Both => {
            // Keep the old entries too: a file deleted on one side is not deleted
            // on the other, and re-appears only if that other side changes.
            for (rel, sig) in scan(root) { state.local.insert(rel, sig); }
            for (rel, sig) in &manifest.files { state.remote.insert(rel.clone(), *sig); }
        }
        _ => {
            state.local = scan(root);
            state.remote = manifest.files;
        }
    }
    state.last_sync = now();
    write_atomic(&state_path(root), &serde_json::to_vec_pretty(&state)?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_skips_locks_and_temps() {
        let root = std::env::temp_dir().join(format!("qsync-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".quiet")).unwrap();
        fs::write(root.join("note.xml"), b"a").unwrap();
        fs::write(root.join(".quiet/.lock"), b"").unwrap();
        fs::write(root.join(".quiet/settings.json"), b"{}").unwrap();
        fs::write(root.join(".quiet/preferences.json"), b"{}").unwrap();
        fs::write(root.join("draft.xml.tmp"), b"x").unwrap();
        let found = scan(&root);
        assert!(found.contains_key("note.xml"));
        assert!(found.contains_key(".quiet/preferences.json"));
        assert!(!found.contains_key(".quiet/.lock"));
        assert!(found.contains_key(".quiet/settings.json"));
        assert!(!found.contains_key("draft.xml.tmp"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn paths_stay_inside_the_library() {
        assert!(safe_rel("a/b.xml").is_ok());
        assert!(safe_rel("../outside").is_err());
        assert!(safe_rel("/absolute").is_err());
        assert!(safe_rel("a/../../b").is_err());
    }

    #[test]
    fn utc_stamp_has_the_signing_shape() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(1_700_000_000), "20231114T221320Z");
    }

    #[test]
    fn config_needs_the_fields_its_protocol_uses() {
        let mut cfg = Config::default();
        assert!(cfg.validate().is_err());
        cfg.webdav_url = "https://dav.example.com/notes".into();
        assert!(cfg.validate().is_ok());
        cfg.protocol = "s3".into();
        assert!(cfg.validate().is_err());
        cfg.s3_endpoint = "https://s3.example.com".into();
        cfg.s3_bucket = "notes".into();
        cfg.s3_access_key = "key".into();
        cfg.s3_secret_key = "secret".into();
        assert!(cfg.validate().is_ok());
    }

    struct TestDav {
        addr: String,
        files: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Vec<u8>>>>,
        stop: std::sync::Arc<AtomicBool>,
    }

    impl TestDav {
        fn start() -> Self {
            use std::net::TcpListener;
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let addr = listener.local_addr().unwrap().to_string();
            let files = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let (f, st) = (files.clone(), stop.clone());
            std::thread::spawn(move || loop {
                if st.load(Ordering::Acquire) { break }
                match listener.accept() {
                    Ok((stream, _)) => serve(stream, &f),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            });
            Self { addr, files, stop }
        }
    }

    impl Drop for TestDav {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            let _ = std::net::TcpStream::connect(&self.addr);
        }
    }

    fn serve(mut stream: std::net::TcpStream, files: &std::sync::Mutex<BTreeMap<String, Vec<u8>>>) {
        use std::io::{Read as _, Write as _};
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let header_end = loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 { return }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") { break i }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
        let mut lines = head.lines();
        let first = lines.next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("/");
        let length: usize = lines.find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0))).unwrap_or(0);
        let body_start = header_end + 4;
        while buf.len() < body_start + length {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 { break }
            buf.extend_from_slice(&chunk[..n]);
        }
        let body = &buf[body_start..(body_start + length).min(buf.len())];
        let key = path.strip_prefix("/dav/").unwrap_or(path.trim_start_matches('/')).to_string();
        let (status, reply): (&str, Vec<u8>) = match method {
            "PUT" => { files.lock().unwrap().insert(key, body.to_vec()); ("201 Created", Vec::new()) }
            "GET" => match files.lock().unwrap().get(&key).cloned() {
                Some(bytes) => ("200 OK", bytes),
                None => ("404 Not Found", Vec::new()),
            },
            "MKCOL" => ("201 Created", Vec::new()),
            _ => ("405 Method Not Allowed", Vec::new()),
        };
        let header = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", reply.len());
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&reply);
    }

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qsync-{name}-{}-{}", std::process::id(), now()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn webdav_upload_download_and_conflicts() {
        let server = TestDav::start();
        let cfg = Config { protocol: "webdav".into(), webdav_url: format!("http://{}/dav", server.addr), ..Default::default() };
        let a = test_dir("a");
        let b = test_dir("b");
        fs::create_dir_all(a.join(".quiet")).unwrap();
        fs::write(a.join("note.xml"), b"one").unwrap();
        fs::write(a.join(".quiet/preferences.json"), b"{\"theme\":\"craft\"}").unwrap();
        fs::write(a.join(".quiet/.lock"), b"").unwrap();
        fs::write(a.join(".quiet/settings.json"), b"{}").unwrap();
        fs::write(a.join("draft.xml.tmp"), b"x").unwrap();

        let report = run(&a, &cfg, Direction::Upload).unwrap();
        assert!(report.uploaded >= 2);
        {
            let files = server.files.lock().unwrap();
            assert_eq!(files.get("note.xml").map(Vec::as_slice), Some(&b"one"[..]));
            assert!(files.contains_key(".quiet/preferences.json"));
            assert!(!files.contains_key(".quiet/.lock"));
            assert!(files.contains_key(".quiet/settings.json"));
            assert!(!files.contains_key("draft.xml.tmp"));
            assert!(files.contains_key(MANIFEST_KEY));
        }

        fs::create_dir_all(&b).unwrap();
        run(&b, &cfg, Direction::Download).unwrap();
        assert_eq!(fs::read(b.join("note.xml")).unwrap(), b"one");
        assert!(b.join(".quiet/preferences.json").is_file());

        fs::write(b.join("note.xml"), b"two").unwrap();
        run(&b, &cfg, Direction::Both).unwrap();
        assert_eq!(server.files.lock().unwrap().get("note.xml").map(Vec::as_slice), Some(&b"two"[..]));

        run(&a, &cfg, Direction::Download).unwrap();
        assert_eq!(fs::read(a.join("note.xml")).unwrap(), b"two");
        fs::write(a.join("note.xml"), b"from a").unwrap();
        fs::write(b.join("note.xml"), b"from b").unwrap();
        run(&a, &cfg, Direction::Both).unwrap();
        let report = run(&b, &cfg, Direction::Both).unwrap();
        assert!(report.conflicts >= 1);
        let files = server.files.lock().unwrap();
        assert_eq!(files.get("note.xml").map(Vec::as_slice), Some(&b"from b"[..]));
        assert!(files.keys().any(|k| (k.contains("cloud conflict") || k.contains("cloud%20conflict")) && k.ends_with(".xml")));
        drop(files);
        assert!(fs::read_dir(&b).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("cloud conflict")));

        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }
}
