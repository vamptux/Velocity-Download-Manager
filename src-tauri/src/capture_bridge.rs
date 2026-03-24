use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use ts_rs::TS;

use crate::model::{DownloadRequestField, DownloadRequestMethod};

static PENDING_CAPTURE: LazyLock<Mutex<BTreeMap<String, PendingCaptureEntry>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
static CAPTURE_WINDOW_COUNTER: AtomicU64 = AtomicU64::new(1);

type HmacSha256 = Hmac<Sha256>;

pub const CAPTURE_PORT: u16 = 17_780;

const CAPTURE_BRIDGE_VERSION: &str = env!("CARGO_PKG_VERSION");

const MAX_BODY_LEN: usize = 64 * 1024;
const PENDING_CAPTURE_TTL_MS: i64 = 3 * 60 * 1000;
const PENDING_CAPTURE_CLEANUP_INTERVAL_MS: u64 = 30 * 1000;
const MAX_PENDING_CAPTURES: usize = 64;

const PAIRING_SECRET_BYTES: usize = 20;
const SESSION_NONCE_BYTES: usize = 16;
const AUTH_REQUEST_NONCE_TTL_MS: i64 = 5 * 60 * 1000;
const AUTH_TIMESTAMP_SKEW_MS: i64 = 5 * 60 * 1000;
const AUTH_STATE_FILENAME: &str = "capture-bridge-auth.json";
const AUTH_HEADER_CLIENT: &str = "x-vdm-client";
const AUTH_HEADER_TIMESTAMP: &str = "x-vdm-timestamp";
const AUTH_HEADER_REQUEST_NONCE: &str = "x-vdm-request-nonce";
const AUTH_HEADER_SIGNATURE: &str = "x-vdm-auth";
const AUTH_HEADER_EXTENSION_ORIGIN: &str = "x-vdm-extension-origin";
const CORS_ALLOW_HEADERS: &str = "Content-Type, X-VDM-Client, X-VDM-Extension-Origin, X-VDM-Timestamp, X-VDM-Request-Nonce, X-VDM-Auth";

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CapturePayload {
    pub url: String,
    pub referrer: Option<String>,
    pub filename: Option<String>,
    pub size_hint: Option<u64>,
    pub mime: Option<String>,
    #[serde(default)]
    pub request_cookies: Option<String>,
    #[serde(default)]
    pub request_method: DownloadRequestMethod,
    #[serde(default)]
    pub request_form_fields: Vec<DownloadRequestField>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum CaptureBridgePhase {
    #[default]
    Starting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CaptureBridgeStatus {
    pub phase: CaptureBridgePhase,
    pub bridge_url: String,
    pub port: u16,
    pub auth_required: bool,
    pub pending_captures: u32,
    pub last_capture_at: Option<i64>,
    pub last_capture_source: Option<String>,
    pub last_error: Option<String>,
    pub rotated_at: i64,
}

#[derive(Debug, Clone)]
struct PendingCaptureEntry {
    payload: CapturePayload,
    captured_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBridgeHealthResponse {
    status: String,
    version: String,
    auth_required: bool,
    authorized: bool,
    session_nonce: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBridgePairingResponse {
    pairing_code: String,
    bridge_url: String,
    rotated_at: i64,
    session_nonce: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBridgeOkResponse {
    ok: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBridgeUnauthorizedResponse {
    error: &'static str,
    auth_required: bool,
    session_nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedCaptureBridgeAuth {
    pairing_secret: String,
    rotated_at: i64,
}

#[derive(Debug)]
struct CaptureBridgeAuthRuntime {
    pairing_secret: String,
    session_nonce: String,
    rotated_at: i64,
    seen_request_nonces: VecDeque<(String, i64)>,
}

#[derive(Debug, Default)]
struct CaptureBridgeRuntime {
    phase: CaptureBridgePhase,
    last_capture_at: Option<i64>,
    last_capture_source: Option<String>,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CaptureBridgeState {
    auth: Arc<Mutex<CaptureBridgeAuthRuntime>>,
    runtime: Arc<Mutex<CaptureBridgeRuntime>>,
}

struct HttpRequest<'a> {
    method: &'a str,
    path: &'a str,
    headers: BTreeMap<String, String>,
    body: &'a [u8],
}

pub fn initialize_capture_bridge_state(app: &AppHandle) -> Result<CaptureBridgeState, String> {
    let base_path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("vdm"));
    let auth_path = base_path.join(AUTH_STATE_FILENAME);
    let persisted = load_or_create_persisted_auth(&auth_path)?;

    Ok(CaptureBridgeState {
        auth: Arc::new(Mutex::new(CaptureBridgeAuthRuntime {
            pairing_secret: persisted.pairing_secret,
            session_nonce: generate_random_hex(SESSION_NONCE_BYTES)?,
            rotated_at: persisted.rotated_at,
            seen_request_nonces: VecDeque::new(),
        })),
        runtime: Arc::new(Mutex::new(CaptureBridgeRuntime::default())),
    })
}

impl CaptureBridgeState {
    pub fn status(&self) -> Result<CaptureBridgeStatus, String> {
        let auth = self
            .auth
            .lock()
            .map_err(|_| "Capture bridge auth lock was poisoned.".to_string())?;
        let runtime = self
            .runtime
            .lock()
            .map_err(|_| "Capture bridge runtime lock was poisoned.".to_string())?;
        let pending_captures = PENDING_CAPTURE
            .lock()
            .map(|pending| pending.len() as u32)
            .unwrap_or_default();

        Ok(CaptureBridgeStatus {
            phase: runtime.phase,
            bridge_url: format!("http://127.0.0.1:{CAPTURE_PORT}"),
            port: CAPTURE_PORT,
            auth_required: true,
            pending_captures,
            last_capture_at: runtime.last_capture_at,
            last_capture_source: runtime.last_capture_source.clone(),
            last_error: runtime.last_error.clone(),
            rotated_at: auth.rotated_at,
        })
    }

    fn health_response(&self, authorized: bool) -> Result<CaptureBridgeHealthResponse, String> {
        let auth = self
            .auth
            .lock()
            .map_err(|_| "Capture bridge auth lock was poisoned.".to_string())?;
        Ok(CaptureBridgeHealthResponse {
            status: "ready".to_string(),
            version: CAPTURE_BRIDGE_VERSION.to_string(),
            auth_required: true,
            authorized,
            session_nonce: auth.session_nonce.clone(),
        })
    }

    fn extension_pairing_response(&self) -> Result<CaptureBridgePairingResponse, String> {
        let auth = self
            .auth
            .lock()
            .map_err(|_| "Capture bridge auth lock was poisoned.".to_string())?;
        Ok(CaptureBridgePairingResponse {
            pairing_code: auth.pairing_secret.clone(),
            bridge_url: format!("http://127.0.0.1:{CAPTURE_PORT}"),
            rotated_at: auth.rotated_at,
            session_nonce: auth.session_nonce.clone(),
        })
    }

    fn unauthorized_response(&self) -> Result<CaptureBridgeUnauthorizedResponse, String> {
        let auth = self
            .auth
            .lock()
            .map_err(|_| "Capture bridge auth lock was poisoned.".to_string())?;
        Ok(CaptureBridgeUnauthorizedResponse {
            error: "capture bridge authentication required",
            auth_required: true,
            session_nonce: auth.session_nonce.clone(),
        })
    }

    fn authorize_request(&self, request: &HttpRequest<'_>) -> Result<(), &'static str> {
        let client = request
            .headers
            .get(AUTH_HEADER_CLIENT)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or("missing auth client")?;
        let timestamp = request
            .headers
            .get(AUTH_HEADER_TIMESTAMP)
            .ok_or("missing auth timestamp")?
            .parse::<i64>()
            .map_err(|_| "invalid auth timestamp")?;
        let request_nonce = request
            .headers
            .get(AUTH_HEADER_REQUEST_NONCE)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or("missing auth request nonce")?;
        let provided_signature = request
            .headers
            .get(AUTH_HEADER_SIGNATURE)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or("missing auth signature")?;

        let now = current_timestamp_ms();
        if (now - timestamp).abs() > AUTH_TIMESTAMP_SKEW_MS {
            return Err("stale auth timestamp");
        }

        let mut auth = self.auth.lock().map_err(|_| "auth state unavailable")?;
        prune_seen_request_nonces(&mut auth.seen_request_nonces, now);
        if auth
            .seen_request_nonces
            .iter()
            .any(|(existing, _)| existing == request_nonce)
        {
            return Err("replayed auth nonce");
        }

        let mut mac = HmacSha256::new_from_slice(auth.pairing_secret.as_bytes())
            .map_err(|_| "invalid auth secret")?;
        mac.update(
            auth_payload(
                request.method,
                request.path,
                &auth.session_nonce,
                timestamp,
                request_nonce,
                client,
                request.body,
            )
            .as_bytes(),
        );

        let provided_bytes = decode_hex(provided_signature)?;
        mac.verify_slice(&provided_bytes)
            .map_err(|_| "invalid auth signature")?;
        auth.seen_request_nonces
            .push_back((request_nonce.to_string(), now));
        Ok(())
    }

    fn record_capture(&self, payload: &CapturePayload) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Capture bridge runtime lock was poisoned.".to_string())?;
        runtime.last_capture_at = Some(current_timestamp_ms());
        runtime.last_capture_source = payload.source.clone();
        runtime.last_error = None;
        Ok(())
    }

    fn set_phase(&self, phase: CaptureBridgePhase, error: Option<String>) -> Result<(), String> {
        let mut runtime = self
            .runtime
            .lock()
            .map_err(|_| "Capture bridge runtime lock was poisoned.".to_string())?;
        runtime.phase = phase;
        runtime.last_error = error;
        Ok(())
    }
}

pub fn take_pending_capture(window_label: Option<&str>) -> Option<CapturePayload> {
    let Ok(mut pending) = PENDING_CAPTURE.lock() else {
        return None;
    };
    prune_pending_capture_entries(&mut pending, current_timestamp_ms());

    if let Some(label) = window_label {
        return pending.remove(label).map(|entry| entry.payload);
    }

    let first_label = pending.keys().next().cloned()?;
    pending.remove(&first_label).map(|entry| entry.payload)
}

pub fn capture_window_ready(window_label: &str) -> Option<CapturePayload> {
    take_pending_capture(Some(window_label))
}

fn set_pending_capture(window_label: &str, payload: CapturePayload) {
    if let Ok(mut pending) = PENDING_CAPTURE.lock() {
        let now = current_timestamp_ms();
        prune_pending_capture_entries(&mut pending, now);
        pending.insert(
            window_label.to_string(),
            PendingCaptureEntry {
                payload,
                captured_at_ms: now,
            },
        );
        prune_pending_capture_entries(&mut pending, now);
    }
}

fn clear_pending_capture(window_label: &str) {
    if let Ok(mut pending) = PENDING_CAPTURE.lock() {
        pending.remove(window_label);
    }
}

fn next_capture_window_label() -> String {
    format!(
        "capture-{}",
        CAPTURE_WINDOW_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub fn spawn_capture_server(app: AppHandle, state: CaptureBridgeState) {
    tauri::async_runtime::spawn(async move {
        run_capture_server(app, state).await;
    });
}

pub fn spawn_pending_capture_cleanup(app: AppHandle, state: CaptureBridgeState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(PENDING_CAPTURE_CLEANUP_INTERVAL_MS)).await;
            if cleanup_pending_captures() {
                emit_capture_bridge_status(&app, &state);
            }
        }
    });
}

fn cleanup_pending_captures() -> bool {
    if let Ok(mut pending) = PENDING_CAPTURE.lock() {
        let before = pending.len();
        prune_pending_capture_entries(&mut pending, current_timestamp_ms());
        return pending.len() != before;
    }

    false
}

fn emit_capture_bridge_status(app: &AppHandle, state: &CaptureBridgeState) {
    if let Ok(status) = state.status() {
        let _ = app.emit("capture://bridge-status", status);
    }
}

fn prune_pending_capture_entries(
    pending: &mut BTreeMap<String, PendingCaptureEntry>,
    now_ms: i64,
) {
    pending.retain(|_, entry| {
        now_ms.saturating_sub(entry.captured_at_ms) <= PENDING_CAPTURE_TTL_MS
    });

    while pending.len() > MAX_PENDING_CAPTURES {
        let Some(oldest_label) = pending
            .iter()
            .min_by_key(|(_, entry)| entry.captured_at_ms)
            .map(|(label, _)| label.clone())
        else {
            break;
        };
        pending.remove(&oldest_label);
    }
}

async fn run_capture_server(app: AppHandle, state: CaptureBridgeState) {
    let addr = SocketAddr::from(([127, 0, 0, 1], CAPTURE_PORT));
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            let message = format!("Could not bind {addr}: {err}");
            let _ = state.set_phase(CaptureBridgePhase::Failed, Some(message));
            emit_capture_bridge_status(&app, &state);
            eprintln!("[VDM] capture bridge: could not bind {addr}: {err}");
            return;
        }
    };

    let _ = state.set_phase(CaptureBridgePhase::Ready, None);
    emit_capture_bridge_status(&app, &state);

    eprintln!("[VDM] capture bridge: listening on {addr}");

    loop {
        let Ok((socket, peer)) = listener.accept().await else {
            break;
        };
        if !peer.ip().is_loopback() {
            eprintln!("[VDM] capture bridge: non-loopback connection from {peer} rejected");
            continue;
        }

        let app = app.clone();
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            handle_connection(socket, app, state).await;
        });
    }
}

async fn handle_connection(mut socket: TcpStream, app: AppHandle, state: CaptureBridgeState) {
    let mut buf = Vec::with_capacity(4096);
    if read_request(&mut socket, &mut buf).await.is_err() {
        return;
    }

    let Some(request) = parse_http(&buf) else {
        let _ = write_response(&mut socket, 400, "Bad Request", &b"bad request"[..]).await;
        return;
    };

    if request.method == "OPTIONS" {
        let _ = write_cors_preflight(&mut socket).await;
        return;
    }

    let auth_result = state.authorize_request(&request);
    let authorized = auth_result.is_ok();

    match (request.method, request.path) {
        ("GET", "/health") => {
            let health = match state.health_response(authorized) {
                Ok(health) => health,
                Err(_) => return,
            };
            let body = serde_json::to_vec(&health).unwrap_or_else(|_| b"{}".to_vec());
            let _ = write_json_response(&mut socket, 200, "OK", &body).await;
        }

        ("GET", "/pair") => {
            let Some(allow_origin) = allowed_extension_origin(&request) else {
                let _ = write_response(&mut socket, 403, "Forbidden", &b"forbidden"[..]).await;
                return;
            };

            let pairing = match state.extension_pairing_response() {
                Ok(pairing) => pairing,
                Err(_) => return,
            };
            let body = serde_json::to_vec(&pairing).unwrap_or_else(|_| b"{}".to_vec());
            let _ = write_json_response_for_origin(
                &mut socket,
                200,
                "OK",
                &body,
                allow_origin.as_str(),
            )
            .await;
        }

        ("GET", "/focus") => {
            focus_main_window(&app);
            let body = serde_json::to_vec(&CaptureBridgeOkResponse { ok: true })
                .unwrap_or_else(|_| b"{}".to_vec());
            let _ = write_json_response(&mut socket, 200, "OK", &body).await;
        }

        ("POST", "/capture") => {
            if !authorized {
                log_unauthorized_request(&request, auth_result.err());
                let _ = write_unauthorized_response(&mut socket, &state).await;
                return;
            }

            let payload: CapturePayload = match serde_json::from_slice(request.body) {
                Ok(payload) => payload,
                Err(_) => {
                    let _ =
                        write_response(&mut socket, 400, "Bad Request", &b"invalid json"[..]).await;
                    return;
                }
            };

            if !payload.url.starts_with("http://") && !payload.url.starts_with("https://") {
                let _ = write_response(
                    &mut socket,
                    422,
                    "Unprocessable Entity",
                    &b"url must be http(s)"[..],
                )
                .await;
                return;
            }

            let _ = state.record_capture(&payload);
            show_capture_window(&app, &state, &payload);

            let body = serde_json::to_vec(&CaptureBridgeOkResponse { ok: true })
                .unwrap_or_else(|_| b"{}".to_vec());
            let _ = write_json_response(&mut socket, 200, "OK", &body).await;
        }

        _ => {
            let _ = write_response(&mut socket, 404, "Not Found", &b"not found"[..]).await;
        }
    }
}

fn log_unauthorized_request(request: &HttpRequest<'_>, reason: Option<&'static str>) {
    if let Some(reason) = reason {
        eprintln!(
            "[VDM] capture bridge: unauthorized {} {} ({reason})",
            request.method, request.path
        );
    }
}

async fn write_unauthorized_response(
    socket: &mut TcpStream,
    state: &CaptureBridgeState,
) -> std::io::Result<()> {
    let body = match state.unauthorized_response() {
        Ok(payload) => serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec()),
        Err(_) => b"{}".to_vec(),
    };
    write_json_response(socket, 401, "Unauthorized", body.as_slice()).await
}

fn show_capture_window(app: &AppHandle, state: &CaptureBridgeState, payload: &CapturePayload) {
    let window_label = next_capture_window_label();
    set_pending_capture(&window_label, payload.clone());
    emit_capture_bridge_status(app, state);

    let url = WebviewUrl::App("index.html?window=capture".into());
    let capture_window = match WebviewWindowBuilder::new(app, &window_label, url)
        .title("Add Download")
        .inner_size(460.0, 316.0)
        .min_inner_size(420.0, 280.0)
        .decorations(false)
        .always_on_top(true)
        .center()
        .build()
    {
        Ok(window) => window,
        Err(err) => {
            clear_pending_capture(&window_label);
            emit_capture_bridge_status(app, state);
            eprintln!("[VDM] capture bridge: could not create capture window: {err}");
            return;
        }
    };

    let _ = capture_window.show();
    let _ = capture_window.unminimize();
    let _ = capture_window.set_focus();
}

fn focus_main_window(app: &AppHandle) {
    if let Some(main_window) = app.get_webview_window("main") {
        let _ = main_window.show();
        let _ = main_window.unminimize();
        let _ = main_window.set_focus();
    }
}

fn load_or_create_persisted_auth(path: &Path) -> Result<PersistedCaptureBridgeAuth, String> {
    if !path.exists() {
        let persisted = PersistedCaptureBridgeAuth {
            pairing_secret: generate_random_hex(PAIRING_SECRET_BYTES)?,
            rotated_at: current_timestamp_ms(),
        };
        persist_capture_bridge_auth(path, &persisted)?;
        return Ok(persisted);
    }

    let raw = fs::read_to_string(path).map_err(|error| {
        format!(
            "Failed reading capture bridge auth state '{}': {error}",
            path.display()
        )
    })?;

    match serde_json::from_str::<PersistedCaptureBridgeAuth>(&raw) {
        Ok(persisted) if !persisted.pairing_secret.trim().is_empty() => Ok(persisted),
        Ok(_) | Err(_) => {
            let backup = corrupt_auth_backup_path(path);
            let _ = fs::rename(path, &backup);
            let persisted = PersistedCaptureBridgeAuth {
                pairing_secret: generate_random_hex(PAIRING_SECRET_BYTES)?,
                rotated_at: current_timestamp_ms(),
            };
            persist_capture_bridge_auth(path, &persisted)?;
            Ok(persisted)
        }
    }
}

fn persist_capture_bridge_auth(
    path: &Path,
    persisted: &PersistedCaptureBridgeAuth,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Invalid capture bridge auth path '{}'.", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "Failed creating capture bridge auth directory '{}': {error}",
            parent.display()
        )
    })?;

    let payload = serde_json::to_vec_pretty(persisted)
        .map_err(|error| format!("Failed serializing capture bridge auth state: {error}"))?;
    fs::write(path, payload).map_err(|error| {
        format!(
            "Failed writing capture bridge auth state '{}': {error}",
            path.display()
        )
    })
}

fn corrupt_auth_backup_path(path: &Path) -> PathBuf {
    path.with_extension(format!("auth.corrupt.{}.json", current_timestamp_ms()))
}

fn allowed_extension_origin(request: &HttpRequest<'_>) -> Option<String> {
    let client = request
        .headers
        .get(AUTH_HEADER_CLIENT)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())?;

    let matches_client = |value: &str| {
        let (origin, host) = parse_extension_origin(value)?;
        (host == client).then_some(origin)
    };

    ["origin", "referer"]
        .into_iter()
        .filter_map(|header| request.headers.get(header))
        .find_map(|value| matches_client(value))
        .or_else(|| {
            request
                .headers
                .get(AUTH_HEADER_EXTENSION_ORIGIN)
                .and_then(|value| matches_client(value))
        })
}

fn parse_extension_origin(value: &str) -> Option<(String, String)> {
    let remainder = value.trim().strip_prefix("chrome-extension://")?;
    let host = remainder
        .split(['/', '?', '#'])
        .next()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())?;
    Some((format!("chrome-extension://{host}"), host.to_string()))
}

fn prune_seen_request_nonces(entries: &mut VecDeque<(String, i64)>, now: i64) {
    while entries
        .front()
        .is_some_and(|(_, seen_at)| now.saturating_sub(*seen_at) > AUTH_REQUEST_NONCE_TTL_MS)
    {
        let _ = entries.pop_front();
    }
}

fn auth_payload(
    method: &str,
    path: &str,
    session_nonce: &str,
    timestamp: i64,
    request_nonce: &str,
    client: &str,
    body: &[u8],
) -> String {
    let body_hash = Sha256::digest(body);
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        path,
        session_nonce,
        timestamp,
        request_nonce,
        client,
        hex_encode(&body_hash),
    )
}

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn generate_random_hex(byte_len: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; byte_len];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("Failed generating capture bridge secret: {error}"))?;
    Ok(hex_encode(&bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn decode_hex(value: &str) -> Result<Vec<u8>, &'static str> {
    if !value.len().is_multiple_of(2) {
        return Err("invalid signature format");
    }

    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.as_bytes().chunks_exact(2);
    for pair in &mut chars {
        let high = decode_hex_nibble(pair[0])?;
        let low = decode_hex_nibble(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

fn decode_hex_nibble(value: u8) -> Result<u8, &'static str> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(10 + value - b'a'),
        b'A'..=b'F' => Ok(10 + value - b'A'),
        _ => Err("invalid signature format"),
    }
}

async fn read_request(socket: &mut TcpStream, buf: &mut Vec<u8>) -> Result<(), ()> {
    let mut tmp = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        if Instant::now() > deadline {
            return Err(());
        }
        match tokio::time::timeout(Duration::from_secs(5), socket.read(&mut tmp[..])).await {
            Ok(Ok(0)) => return Ok(()),
            Ok(Ok(read)) => {
                buf.extend_from_slice(&tmp[..read]);
                if buf.len() > MAX_BODY_LEN {
                    return Err(());
                }
                if has_complete_http_request(buf) {
                    return Ok(());
                }
            }
            _ => return Err(()),
        }
    }
}

fn has_complete_http_request(buf: &[u8]) -> bool {
    if let Some(header_end) = find_double_crlf(buf) {
        let header_section = std::str::from_utf8(&buf[..header_end + 4]).unwrap_or("");
        let content_length = extract_content_length(header_section).unwrap_or(0);
        let body_start = header_end + 4;
        buf.len() >= body_start + content_length
    } else {
        false
    }
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

fn extract_content_length(headers: &str) -> Option<usize> {
    for line in headers.lines() {
        if line.to_ascii_lowercase().starts_with("content-length:") {
            let (_, value) = line.split_once(':')?;
            return value.trim().parse().ok();
        }
    }
    None
}

fn parse_http(buf: &[u8]) -> Option<HttpRequest<'_>> {
    let header_end = find_double_crlf(buf)?;
    let header_str = std::str::from_utf8(&buf[..header_end + 4]).ok()?;

    let mut lines = header_str.split("\r\n");
    let first_line = lines.next()?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next()?;
    let raw_path = parts.next()?;
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }

    Some(HttpRequest {
        method,
        path,
        headers,
        body: &buf[header_end + 4..],
    })
}

async fn write_response(
    socket: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: {CORS_ALLOW_HEADERS}\r\n\r\n",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.flush().await
}

async fn write_json_response(
    socket: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: {CORS_ALLOW_HEADERS}\r\n\r\n",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.flush().await
}

async fn write_json_response_for_origin(
    socket: &mut TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
    allow_origin: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: {allow_origin}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: {CORS_ALLOW_HEADERS}\r\n\r\n",
        body.len()
    );
    socket.write_all(response.as_bytes()).await?;
    socket.write_all(body).await?;
    socket.flush().await
}

async fn write_cors_preflight(socket: &mut TcpStream) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: {CORS_ALLOW_HEADERS}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    socket.write_all(response.as_bytes()).await?;
    socket.flush().await
}
