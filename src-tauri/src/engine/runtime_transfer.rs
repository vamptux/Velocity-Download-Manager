use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{
    header::{HeaderMap, HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, RETRY_AFTER},
    StatusCode,
};
use tokio::time::sleep;

use super::file_ops::query_available_space;
use super::http_helpers::{
    apply_request_cookies, apply_request_payload, apply_request_referer,
    parse_content_range_bounds, request_context_supports_segmented_transfer,
};
use super::probe::protocol_label;
use crate::engine::disk_pool::{DiskPool, WriteBlock};
use crate::engine::http_pool::HttpPool;
use crate::model::{
    DownloadRequestField, DownloadRequestMethod, DownloadSegment, DownloadSegmentStatus,
};

#[derive(Clone)]
pub struct TransferWorkerConfig {
    pub url: String,
    pub request_referer: Option<String>,
    pub request_cookies: Option<String>,
    pub request_method: DownloadRequestMethod,
    pub request_form_fields: Vec<DownloadRequestField>,
    pub chunk_buffer_size: usize,
    pub request_timeout_secs: u64,
    pub retry_budget: u32,
    pub backoff_base_ms: u64,
    pub backoff_max_ms: u64,
    pub per_download_limiter: Option<Arc<TokenBucketRateLimiter>>,
    pub per_host_limiter: Option<Arc<TokenBucketRateLimiter>>,
}

impl Default for TransferWorkerConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            request_referer: None,
            request_cookies: None,
            request_method: DownloadRequestMethod::Get,
            request_form_fields: Vec::new(),
            chunk_buffer_size: 2 * 1024 * 1024,
            request_timeout_secs: 30,
            retry_budget: 5,
            backoff_base_ms: 150,
            backoff_max_ms: 2_500,
            per_download_limiter: None,
            per_host_limiter: None,
        }
    }
}

#[derive(Clone)]
pub struct SegmentRuntimeProgress {
    pub segment_id: u32,
    pub downloaded: i64,
    pub status: DownloadSegmentStatus,
    pub throughput_bytes_per_second: u64,
    pub retry_attempts: u32,
    pub terminal_failure_reason: Option<String>,
    pub telemetry: Option<SegmentNetworkTelemetry>,
}

#[derive(Clone)]
pub struct SegmentNetworkTelemetry {
    pub ttfb_ms: Option<u64>,
    pub connection_reused: Option<bool>,
    pub negotiated_protocol: Option<String>,
}

pub struct InitialResponseStream {
    pub response: reqwest::Response,
    pub ttfb_ms: u64,
    pub connection_reused_hint: bool,
    pub negotiated_protocol: Option<String>,
}

pub struct SegmentWorkerStart {
    pub control: SegmentRuntimeControl,
    pub initial_response: Option<InitialResponseStream>,
}

pub struct UnknownSizeStreamOutcome {
    pub downloaded: u64,
    pub reported_content_length: Option<u64>,
}

#[derive(Clone, Default)]
pub struct UnknownSizeStreamOptions {
    pub starting_offset: u64,
    pub space_check_path: Option<PathBuf>,
    pub space_check_interval_bytes: u64,
    pub space_safety_margin_bytes: u64,
}

pub struct UnknownSizeStreamProgress {
    pub downloaded: i64,
    pub throughput_bytes_per_second: u64,
    pub telemetry: Option<SegmentNetworkTelemetry>,
}

#[derive(Clone, Copy)]
struct RetryPolicy {
    budget: u32,
    backoff_base_ms: u64,
    backoff_max_ms: u64,
    jitter_percent: u8,
}

const THROTTLE_EXTRA_RETRIES: u32 = 6;
const THROTTLE_BACKOFF_MAX_MS: u64 = 30_000;
const RETRY_AFTER_MAX_SECONDS: u64 = 120;
const DEFAULT_RETRY_JITTER_PERCENT: u8 = 15;
const ADAPTIVE_CHUNK_BUFFER_FLOOR_BYTES: usize = 256 * 1024;
const ADAPTIVE_CHUNK_BUFFER_CEILING_BYTES: usize = 8 * 1024 * 1024;

struct FetchSegmentOutcome {
    response: reqwest::Response,
    ttfb_ms: u64,
    connection_reused_hint: bool,
    negotiated_protocol: Option<String>,
    retry_attempts: u32,
}

#[derive(Clone, Copy)]
struct FetchSegmentRequest {
    start: u64,
    end: u64,
    connection_reused_hint: bool,
}

impl RetryPolicy {
    fn from_config(config: &TransferWorkerConfig) -> Self {
        let backoff_base_ms = config.backoff_base_ms.max(10);
        Self {
            budget: config.retry_budget.max(1),
            backoff_base_ms,
            backoff_max_ms: config.backoff_max_ms.max(backoff_base_ms),
            jitter_percent: DEFAULT_RETRY_JITTER_PERCENT,
        }
    }

    fn delay_ms(self, attempt: u32, throttled: bool, retry_after_ms: Option<u64>) -> u64 {
        let backoff_cap = if throttled {
            THROTTLE_BACKOFF_MAX_MS
        } else {
            self.backoff_max_ms
        };
        let base_delay = exponential_backoff_ms(self.backoff_base_ms, backoff_cap, attempt);
        jittered_retry_delay_ms(
            base_delay.max(retry_after_ms.unwrap_or(0)),
            self.jitter_percent,
        )
    }

    fn stream_recovery_delay_ms(self, attempt: u32) -> u64 {
        jittered_retry_delay_ms(
            exponential_backoff_ms(self.backoff_base_ms, self.backoff_max_ms, attempt),
            self.jitter_percent,
        )
    }
}

fn validate_range_response(
    status: StatusCode,
    headers: &HeaderMap,
    request: FetchSegmentRequest,
) -> Result<(), String> {
    let requested_len = request.end.saturating_sub(request.start).saturating_add(1);
    let content_range = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok());

    match status {
        StatusCode::PARTIAL_CONTENT => {
            let Some((start, end, _)) = parse_content_range_bounds(content_range) else {
                return Err(format!(
                    "Range response for bytes {}-{} omitted a valid Content-Range header.",
                    request.start, request.end
                ));
            };
            if start != request.start || end != request.end {
                return Err(format!(
                    "Range response returned bytes {}-{} for requested {}-{}.",
                    start, end, request.start, request.end
                ));
            }
            Ok(())
        }
        StatusCode::OK => {
            if let Some((start, end, _)) = parse_content_range_bounds(content_range)
                && start == request.start && end == request.end {
                    return Ok(());
                }

            let content_length = header_to_u64(headers.get(CONTENT_LENGTH));
            if request.start == 0 && content_length == Some(requested_len) {
                return Ok(());
            }

            Err(format!(
                "Server ignored the requested byte range {}-{} and returned HTTP 200 without a matching Content-Range.",
                request.start, request.end
            ))
        }
        _ => Ok(()),
    }
}

fn header_to_u64(value: Option<&HeaderValue>) -> Option<u64> {
    value
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BufferedWritePlan {
    bytes_to_write: usize,
    boundary_reached: bool,
}

fn plan_buffered_write(
    current_pos: u64,
    dynamic_end: u64,
    buffered_len: usize,
) -> BufferedWritePlan {
    if buffered_len == 0 {
        return BufferedWritePlan {
            bytes_to_write: 0,
            boundary_reached: false,
        };
    }
    if current_pos > dynamic_end {
        return BufferedWritePlan {
            bytes_to_write: 0,
            boundary_reached: true,
        };
    }

    let writable = dynamic_end
        .saturating_sub(current_pos)
        .saturating_add(1)
        .min(buffered_len as u64);

    BufferedWritePlan {
        bytes_to_write: writable as usize,
        boundary_reached: current_pos.saturating_add(buffered_len as u64) > dynamic_end,
    }
}

fn adaptive_chunk_buffer_target(
    base_chunk_buffer_size: usize,
    observed_throughput_bytes_per_second: u64,
    queue_utilization_percent: usize,
) -> usize {
    let floor = adaptive_chunk_buffer_floor(base_chunk_buffer_size);
    let ceiling = adaptive_chunk_buffer_ceiling(base_chunk_buffer_size);

    if observed_throughput_bytes_per_second == 0 {
        return base_chunk_buffer_size.clamp(floor, ceiling);
    }

    let target_window_ms = match queue_utilization_percent {
        85..=100 => 16_u64,
        70..=84 => 24_u64,
        55..=69 => 40_u64,
        0..=34 => 72_u64,
        _ => 56_u64,
    };
    let target = observed_throughput_bytes_per_second
        .saturating_mul(target_window_ms)
        .div_ceil(1_000);
    usize::try_from(target)
        .unwrap_or(usize::MAX)
        .clamp(floor, ceiling)
}

fn adaptive_chunk_buffer_floor(base_chunk_buffer_size: usize) -> usize {
    base_chunk_buffer_size
        .saturating_div(4)
        .max(ADAPTIVE_CHUNK_BUFFER_FLOOR_BYTES)
}

fn adaptive_chunk_buffer_ceiling(base_chunk_buffer_size: usize) -> usize {
    base_chunk_buffer_size.saturating_mul(2).clamp(
        adaptive_chunk_buffer_floor(base_chunk_buffer_size),
        ADAPTIVE_CHUNK_BUFFER_CEILING_BYTES,
    )
}

#[derive(Clone)]
pub struct SegmentRuntimeControl {
    dynamic_end: Arc<AtomicI64>,
    canceled: Arc<AtomicBool>,
}

impl SegmentRuntimeControl {
    pub fn new(initial_end: i64) -> Self {
        Self {
            dynamic_end: Arc::new(AtomicI64::new(initial_end)),
            canceled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn current_end(&self) -> i64 {
        self.dynamic_end.load(Ordering::Relaxed)
    }

    pub fn set_end(&self, end: i64) {
        self.dynamic_end.store(end, Ordering::Relaxed);
    }

    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Relaxed);
    }

    fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }
}

pub struct TokenBucketRateLimiter {
    rate_bytes_per_second: AtomicU64,
    burst_bytes: AtomicU64,
    state: std::sync::Mutex<TokenBucketState>,
}

struct TokenBucketState {
    available_tokens: f64,
    last_refill: Instant,
}

impl TokenBucketRateLimiter {
    pub fn new(rate_bytes_per_second: u64, burst_bytes: u64) -> Self {
        let burst_bytes = if rate_bytes_per_second == 0 {
            burst_bytes
        } else {
            burst_bytes.max(rate_bytes_per_second)
        };
        Self {
            rate_bytes_per_second: AtomicU64::new(rate_bytes_per_second),
            burst_bytes: AtomicU64::new(burst_bytes),
            state: std::sync::Mutex::new(TokenBucketState {
                available_tokens: burst_bytes as f64,
                last_refill: Instant::now(),
            }),
        }
    }

    pub fn reconfigure(&self, rate_bytes_per_second: u64, burst_bytes: u64) {
        let burst_bytes = if rate_bytes_per_second == 0 {
            burst_bytes
        } else {
            burst_bytes.max(rate_bytes_per_second)
        };
        self.rate_bytes_per_second
            .store(rate_bytes_per_second, Ordering::Relaxed);
        self.burst_bytes.store(burst_bytes, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap();
        state.available_tokens = state.available_tokens.min(burst_bytes as f64);
        state.last_refill = Instant::now();
    }

    pub async fn acquire(&self, bytes: usize) {
        if self.rate_bytes_per_second.load(Ordering::Relaxed) == 0 {
            return;
        }
        let target = bytes.max(1) as f64;
        loop {
            let wait_for = {
                let mut state = self.state.lock().unwrap();
                let rate_bytes_per_second = self.rate_bytes_per_second.load(Ordering::Relaxed);
                if rate_bytes_per_second == 0 {
                    state.last_refill = Instant::now();
                    return;
                }
                let burst_bytes = self.burst_bytes.load(Ordering::Relaxed);
                let now = Instant::now();
                let elapsed = now
                    .saturating_duration_since(state.last_refill)
                    .as_secs_f64();
                if elapsed > 0.0 {
                    let refill = elapsed * rate_bytes_per_second as f64;
                    state.available_tokens =
                        (state.available_tokens + refill).min(burst_bytes as f64);
                    state.last_refill = now;
                }
                if state.available_tokens >= target {
                    state.available_tokens -= target;
                    None
                } else {
                    let deficit = target - state.available_tokens;
                    let wait_secs = deficit / rate_bytes_per_second as f64;
                    Some(Duration::from_secs_f64(wait_secs.max(0.001)))
                }
            };
            if let Some(wait) = wait_for {
                sleep(wait).await;
                continue;
            }
            return;
        }
    }
}

fn build_transfer_request(
    client: &reqwest::Client,
    config: &TransferWorkerConfig,
) -> reqwest::RequestBuilder {
    let request = match config.request_method {
        DownloadRequestMethod::Get => client.get(&config.url),
        DownloadRequestMethod::Post => client.post(&config.url),
    };

    apply_request_cookies(
        apply_request_referer(
            apply_request_payload(request, &config.request_method, &config.request_form_fields),
            config.request_referer.as_deref(),
        ),
        config.request_cookies.as_deref(),
    )
}

pub async fn run_segment_worker<F>(
    config: &TransferWorkerConfig,
    http_pool: &HttpPool,
    disk_pool: &Arc<DiskPool>,
    output_file: &Arc<File>,
    segment: &mut DownloadSegment,
    start: SegmentWorkerStart,
    mut on_progress: F,
) -> Result<(), String>
where
    F: FnMut(SegmentRuntimeProgress) -> Result<(), String>,
{
    if !request_context_supports_segmented_transfer(
        &config.request_method,
        &config.request_form_fields,
    ) {
        return Err(
            "Segmented byte-range workers require a plain GET request context; this download must stay in guarded single-stream mode."
                .to_string(),
        );
    }

    let SegmentWorkerStart {
        control,
        initial_response,
    } = start;
    let client_lease = http_pool
        .get_client(&config.url)
        .ok_or_else(|| "Failed to acquire HTTP client.".to_string())?;
    let client = client_lease.client;
    let pooled_client_reused = client_lease.reused_pool_client;
    let timeout = Duration::from_secs(config.request_timeout_secs.max(1));
    let mut current_pos = segment
        .start
        .saturating_add(segment.downloaded.max(0))
        .max(segment.start) as u64;
    let mut window_start = Instant::now();
    let mut window_bytes: u64 = 0;
    let mut emitted_ttfb_sample = false;
    let mut request_ordinal = 0_u64;
    let mut stream_drops = 0u32;
    let mut observed_throughput_bytes_per_second = 0_u64;
    let retry_policy = RetryPolicy::from_config(config);
    let mut initial_response = initial_response;
    segment.status = DownloadSegmentStatus::Downloading;
    on_progress(SegmentRuntimeProgress {
        segment_id: segment.id,
        downloaded: segment.downloaded,
        status: segment.status.clone(),
        throughput_bytes_per_second: 0,
        retry_attempts: segment.retry_attempts,
        terminal_failure_reason: None,
        telemetry: None,
    })?;

    loop {
        if control.is_canceled() {
            return Err("segment-canceled".to_string());
        }
        let current_end = control.current_end().max(segment.start) as u64;
        segment.end = current_end as i64;
        if current_pos > current_end {
            break;
        }

        let request = FetchSegmentRequest {
            start: current_pos,
            end: current_end,
            connection_reused_hint: pooled_client_reused || request_ordinal > 0,
        };
        let mut fetch_outcome;
        if request_ordinal == 0 {
            if let Some(initial) = initial_response.take() {
                if validate_range_response(
                    initial.response.status(),
                    initial.response.headers(),
                    request,
                )
                .is_ok()
                {
                    fetch_outcome = FetchSegmentOutcome {
                        response: initial.response,
                        ttfb_ms: initial.ttfb_ms,
                        connection_reused_hint: initial.connection_reused_hint,
                        negotiated_protocol: initial.negotiated_protocol,
                        retry_attempts: 0,
                    };
                } else {
                    fetch_outcome = fetch_segment_stream_with_retry(
                        &client,
                        &config.url,
                        request,
                        timeout,
                        retry_policy,
                        config.request_referer.as_deref(),
                        config.request_cookies.as_deref(),
                        &control,
                    )
                    .await?;
                }
            } else {
                fetch_outcome = fetch_segment_stream_with_retry(
                    &client,
                    &config.url,
                    request,
                    timeout,
                    retry_policy,
                    config.request_referer.as_deref(),
                    config.request_cookies.as_deref(),
                    &control,
                )
                .await?;
            }
        } else {
            fetch_outcome = fetch_segment_stream_with_retry(
                &client,
                &config.url,
                request,
                timeout,
                retry_policy,
                config.request_referer.as_deref(),
                config.request_cookies.as_deref(),
                &control,
            )
            .await?;
        }
        request_ordinal = request_ordinal.saturating_add(1);
        if fetch_outcome.retry_attempts > 0 {
            segment.retry_attempts = segment
                .retry_attempts
                .saturating_add(fetch_outcome.retry_attempts);
        }

        let mut local_buffer = Vec::with_capacity(config.chunk_buffer_size);
        let mut stream_failed = false;

        loop {
            if control.is_canceled() {
                return Err("segment-canceled".to_string());
            }

            let chunk_result = fetch_outcome.response.chunk().await;
            let chunk = match chunk_result {
                Ok(c_opt) => {
                    let Some(c) = c_opt else {
                        break; // EOF
                    };
                    c
                }
                Err(_e) => {
                    stream_failed = true;
                    // Stream disconnected mid-flight
                    break;
                }
            };

            let reservation = chunk.len();
            if let Some(limiter) = &config.per_download_limiter {
                limiter.acquire(reservation).await;
            }
            if let Some(limiter) = &config.per_host_limiter {
                limiter.acquire(reservation).await;
            }

            local_buffer.extend_from_slice(&chunk);

            let dynamic_end = control.current_end().max(segment.start) as u64;
            let write_plan = plan_buffered_write(current_pos, dynamic_end, local_buffer.len());
            let flush_target = adaptive_chunk_buffer_target(
                config.chunk_buffer_size,
                observed_throughput_bytes_per_second,
                disk_pool.queue_utilization_percent(),
            );

            if local_buffer.len() >= flush_target || write_plan.boundary_reached {
                segment.end = dynamic_end as i64;
                if write_plan.bytes_to_write == 0 {
                    local_buffer.clear();
                    break;
                }

                let next_buffer_capacity = adaptive_chunk_buffer_target(
                    config.chunk_buffer_size,
                    observed_throughput_bytes_per_second,
                    disk_pool.queue_utilization_percent(),
                );
                let mut write_buffer = std::mem::replace(
                    &mut local_buffer,
                    Vec::with_capacity(next_buffer_capacity),
                );
                if write_plan.bytes_to_write < write_buffer.len() {
                    write_buffer.truncate(write_plan.bytes_to_write);
                }

                let write_block = WriteBlock {
                    file: Arc::clone(output_file),
                    buffer: write_buffer,
                    offset: current_pos,
                };

                disk_pool
                    .enqueue_write(write_block)
                    .await
                    .map_err(|error| format!("Disk write queue failed: {error}"))?;

                let delta = i64::try_from(write_plan.bytes_to_write).unwrap_or(i64::MAX);
                segment.downloaded = segment.downloaded.saturating_add(delta);
                current_pos = current_pos.saturating_add(write_plan.bytes_to_write as u64);
                window_bytes = window_bytes.saturating_add(write_plan.bytes_to_write as u64);

                let elapsed = window_start.elapsed().as_secs_f64();
                let throughput = if elapsed > 0.0 {
                    let value = (window_bytes as f64 / elapsed) as u64;
                    window_start = Instant::now();
                    window_bytes = 0;
                    value
                } else {
                    0
                };
                if throughput > 0 {
                    observed_throughput_bytes_per_second = throughput;
                }

                on_progress(SegmentRuntimeProgress {
                    segment_id: segment.id,
                    downloaded: segment.downloaded,
                    status: segment.status.clone(),
                    throughput_bytes_per_second: throughput,
                    retry_attempts: segment.retry_attempts,
                    terminal_failure_reason: None,
                    telemetry: if !emitted_ttfb_sample {
                        emitted_ttfb_sample = true;
                        Some(SegmentNetworkTelemetry {
                            ttfb_ms: Some(fetch_outcome.ttfb_ms),
                            connection_reused: Some(fetch_outcome.connection_reused_hint),
                            negotiated_protocol: fetch_outcome.negotiated_protocol.clone(),
                        })
                    } else {
                        None
                    },
                })?;

                if write_plan.boundary_reached {
                    break;
                }
            }

            if current_pos >= dynamic_end {
                break; // Target steal boundary reached naturally
            }
        }

        // Handle partial buffer if we broke out before flushing
        if !local_buffer.is_empty() {
            let dynamic_end = control.current_end().max(segment.start) as u64;
            let write_plan = plan_buffered_write(current_pos, dynamic_end, local_buffer.len());
            segment.end = dynamic_end as i64;
            if write_plan.bytes_to_write == 0 {
                local_buffer.clear();
            } else {
                local_buffer.truncate(write_plan.bytes_to_write);
                let to_write = local_buffer.len();
                let write_block = WriteBlock {
                    file: Arc::clone(output_file),
                    buffer: local_buffer,
                    offset: current_pos,
                };
                disk_pool
                    .enqueue_write(write_block)
                    .await
                    .map_err(|error| format!("Disk write queue failed: {error}"))?;

                let delta = i64::try_from(to_write).unwrap_or(i64::MAX);
                segment.downloaded = segment.downloaded.saturating_add(delta);
                current_pos = current_pos.saturating_add(to_write as u64);
            }
        }

        if stream_failed {
            stream_drops = stream_drops.saturating_add(1);
            segment.retry_attempts = segment.retry_attempts.saturating_add(1);
            if stream_drops > config.retry_budget {
                return Err(format!(
                    "Stream dropped {} times, exceeding retry budget.",
                    stream_drops
                ));
            }
            sleep(Duration::from_millis(
                retry_policy.stream_recovery_delay_ms(stream_drops.saturating_sub(1)),
            ))
            .await;
            continue;
        }

        // If we naturally hit the end or EOF without failing, we are done with this segment entirely or reached steal boundary
        // Because of the outer loop, if current_pos >= dynamic_end it breaks anyway.
    }

    segment.status = DownloadSegmentStatus::Finished;
    on_progress(SegmentRuntimeProgress {
        segment_id: segment.id,
        downloaded: segment.downloaded,
        status: segment.status.clone(),
        throughput_bytes_per_second: 0,
        retry_attempts: segment.retry_attempts,
        terminal_failure_reason: None,
        telemetry: None,
    })?;
    Ok(())
}

pub async fn run_unknown_size_stream_worker<F>(
    config: &TransferWorkerConfig,
    options: UnknownSizeStreamOptions,
    http_pool: &HttpPool,
    disk_pool: &Arc<DiskPool>,
    output_file: &Arc<File>,
    initial_response: Option<InitialResponseStream>,
    mut on_progress: F,
) -> Result<UnknownSizeStreamOutcome, String>
where
    F: FnMut(UnknownSizeStreamProgress) -> Result<(), String>,
{
    let client_lease = http_pool
        .get_client(&config.url)
        .ok_or_else(|| "Failed to acquire HTTP client.".to_string())?;
    let client = client_lease.client;
    let pooled_client_reused = client_lease.reused_pool_client;
    let timeout = Duration::from_secs(config.request_timeout_secs.max(1));
    let mut attempt = 0u32;
    let retry_policy = RetryPolicy::from_config(config);
    let starting_offset = options.starting_offset;
    let check_interval = options.space_check_interval_bytes.max(1);
    let mut initial_response = initial_response;

    loop {
        let outcome;
        if let Some(initial) = initial_response.take() {
            outcome = Ok(initial);
        } else {
            let request_started = Instant::now();
            let response = build_transfer_request(client.as_ref(), config)
                .timeout(timeout)
                .send()
                .await;
            outcome = response.map(|response| {
                    let negotiated_protocol = Some(protocol_label(response.version()).to_string());
                    InitialResponseStream {
                        ttfb_ms: request_started.elapsed().as_millis().min(u64::MAX as u128)
                            as u64,
                        negotiated_protocol,
                        response,
                        connection_reused_hint: pooled_client_reused,
                    }
                });
        }
        match outcome {
            Ok(initial) if initial.response.status().is_success() => {
                let mut response = initial.response;
                let mut current_offset = starting_offset;
                let mut local_buffer = Vec::with_capacity(config.chunk_buffer_size);
                let mut window_start = Instant::now();
                let mut window_bytes = 0u64;
                let mut next_space_check_at = current_offset.saturating_add(check_interval);
                let mut emitted_ttfb_sample = false;
                let mut downloaded_any_bytes = false;
                let mut pre_byte_stream_error: Option<String> = None;
                let mut observed_throughput_bytes_per_second = 0_u64;
                let reported_content_length = header_to_u64(response.headers().get(CONTENT_LENGTH));
                let negotiated_protocol = initial
                    .negotiated_protocol
                    .or_else(|| Some(protocol_label(response.version()).to_string()));
                let ttfb_ms = initial.ttfb_ms;
                let connection_reused_hint = initial.connection_reused_hint;

                loop {
                    let chunk = match response.chunk().await {
                        Ok(next) => {
                            let Some(next) = next else {
                                break;
                            };
                            next
                        }
                        Err(error) => {
                            if downloaded_any_bytes {
                                return Err(format!(
                                    "Unknown-size stream dropped after {} downloaded; restart is required because the host did not expose a resumable content length: {error}",
                                    current_offset.saturating_sub(starting_offset)
                                ));
                            }
                            pre_byte_stream_error = Some(error.to_string());
                            break;
                        }
                    };

                    let reservation = chunk.len();
                    if let Some(limiter) = &config.per_download_limiter {
                        limiter.acquire(reservation).await;
                    }
                    if let Some(limiter) = &config.per_host_limiter {
                        limiter.acquire(reservation).await;
                    }

                    local_buffer.extend_from_slice(&chunk);
                    downloaded_any_bytes = true;

                    let flush_target = adaptive_chunk_buffer_target(
                        config.chunk_buffer_size,
                        observed_throughput_bytes_per_second,
                        disk_pool.queue_utilization_percent(),
                    );
                    if local_buffer.len() >= flush_target {
                        let written = local_buffer.len() as u64;
                        let next_buffer_capacity = adaptive_chunk_buffer_target(
                            config.chunk_buffer_size,
                            observed_throughput_bytes_per_second,
                            disk_pool.queue_utilization_percent(),
                        );
                        current_offset = flush_unknown_size_buffer(
                            disk_pool,
                            output_file,
                            &mut local_buffer,
                            next_buffer_capacity,
                            current_offset,
                        )
                        .await?;
                        if let Some(path) = options.space_check_path.as_deref()
                            && current_offset >= next_space_check_at {
                                enforce_unknown_size_free_space(
                                    path,
                                    options.space_safety_margin_bytes,
                                    current_offset.saturating_sub(starting_offset),
                                )?;
                                next_space_check_at = current_offset.saturating_add(check_interval);
                            }
                        window_bytes = window_bytes.saturating_add(written);
                        let throughput =
                            compute_window_throughput(&mut window_start, &mut window_bytes);
                        if throughput > 0 {
                            observed_throughput_bytes_per_second = throughput;
                        }
                        on_progress(UnknownSizeStreamProgress {
                            downloaded: i64::try_from(current_offset).unwrap_or(i64::MAX),
                            throughput_bytes_per_second: throughput,
                            telemetry: if !emitted_ttfb_sample {
                                emitted_ttfb_sample = true;
                                Some(SegmentNetworkTelemetry {
                                    ttfb_ms: Some(ttfb_ms),
                                    connection_reused: Some(connection_reused_hint),
                                    negotiated_protocol: negotiated_protocol.clone(),
                                })
                            } else {
                                None
                            },
                        })?;
                    }
                }

                if let Some(error) = pre_byte_stream_error {
                    if attempt >= config.retry_budget {
                        return Err(format!(
                            "Single-stream transfer failed before any bytes were received: {error}"
                        ));
                    }
                    let delay = retry_policy.delay_ms(attempt, false, None);
                    sleep(Duration::from_millis(delay)).await;
                    attempt = attempt.saturating_add(1);
                    continue;
                }

                if !local_buffer.is_empty() {
                    let written = local_buffer.len() as u64;
                    let next_buffer_capacity = adaptive_chunk_buffer_target(
                        config.chunk_buffer_size,
                        observed_throughput_bytes_per_second,
                        disk_pool.queue_utilization_percent(),
                    );
                    current_offset = flush_unknown_size_buffer(
                        disk_pool,
                        output_file,
                        &mut local_buffer,
                        next_buffer_capacity,
                        current_offset,
                    )
                    .await?;
                    if let Some(path) = options.space_check_path.as_deref() {
                        enforce_unknown_size_free_space(
                            path,
                            options.space_safety_margin_bytes,
                            current_offset.saturating_sub(starting_offset),
                        )?;
                    }
                    window_bytes = window_bytes.saturating_add(written);
                }

                let throughput = compute_window_throughput(&mut window_start, &mut window_bytes);
                on_progress(UnknownSizeStreamProgress {
                    downloaded: i64::try_from(current_offset).unwrap_or(i64::MAX),
                    throughput_bytes_per_second: throughput,
                    telemetry: if !emitted_ttfb_sample {
                        Some(SegmentNetworkTelemetry {
                            ttfb_ms: Some(ttfb_ms),
                            connection_reused: Some(connection_reused_hint),
                            negotiated_protocol,
                        })
                    } else {
                        None
                    },
                })?;

                return Ok(UnknownSizeStreamOutcome {
                    downloaded: current_offset,
                    reported_content_length,
                });
            }
            Ok(initial) => {
                let response = initial.response;
                let status = response.status();
                let throttled = status == StatusCode::TOO_MANY_REQUESTS
                    || status == StatusCode::SERVICE_UNAVAILABLE;
                let retry_limit = if throttled {
                    config.retry_budget.saturating_add(THROTTLE_EXTRA_RETRIES)
                } else {
                    config.retry_budget
                };
                let retry_after_ms = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(parse_retry_after_delay_ms);
                if attempt >= retry_limit {
                    if let Some(delay_ms) = retry_after_ms {
                        return Err(format!(
                            "HTTP {} for single-stream transfer after {} retries (retry-after {}ms).",
                            status, retry_limit, delay_ms
                        ));
                    }
                    return Err(format!(
                        "HTTP {} for single-stream transfer after {} retries.",
                        status, retry_limit
                    ));
                }
                let delay = retry_policy.delay_ms(attempt, throttled, retry_after_ms);
                sleep(Duration::from_millis(delay)).await;
            }
            Err(error) => {
                if attempt >= config.retry_budget {
                    return Err(format!(
                        "Network error before single-stream transfer could stabilize: {error}"
                    ));
                }
                let delay = retry_policy.delay_ms(attempt, false, None);
                sleep(Duration::from_millis(delay)).await;
            }
        }
        attempt = attempt.saturating_add(1);
    }
}

async fn flush_unknown_size_buffer(
    disk_pool: &Arc<DiskPool>,
    output_file: &Arc<File>,
    buffer: &mut Vec<u8>,
    replacement_capacity: usize,
    current_offset: u64,
) -> Result<u64, String> {
    let to_write = buffer.len();
    if to_write == 0 {
        return Ok(current_offset);
    }

    let write_block = WriteBlock {
        file: Arc::clone(output_file),
        buffer: std::mem::replace(buffer, Vec::with_capacity(replacement_capacity)),
        offset: current_offset,
    };
    disk_pool
        .enqueue_write(write_block)
        .await
        .map_err(|error| format!("Disk write queue failed: {error}"))?;

    Ok(current_offset.saturating_add(to_write as u64))
}

fn enforce_unknown_size_free_space(
    path: &std::path::Path,
    safety_margin_bytes: u64,
    streamed_bytes: u64,
) -> Result<(), String> {
    let Some(available_space) = query_available_space(path) else {
        return Ok(());
    };
    if available_space <= safety_margin_bytes {
        return Err(format!(
            "Available disk space dropped to {} bytes while {} bytes were already streamed. Unknown-size transfer stopped before the target volume ran out of space.",
            available_space, streamed_bytes
        ));
    }
    Ok(())
}

fn compute_window_throughput(window_start: &mut Instant, window_bytes: &mut u64) -> u64 {
    let elapsed = window_start.elapsed().as_secs_f64();
    if elapsed <= 0.0 || *window_bytes == 0 {
        return 0;
    }
    let throughput = (*window_bytes as f64 / elapsed) as u64;
    *window_start = Instant::now();
    *window_bytes = 0;
    throughput
}

fn exponential_backoff_ms(base_delay_ms: u64, max_delay_ms: u64, attempt: u32) -> u64 {
    base_delay_ms
        .saturating_mul(2u64.saturating_pow(attempt.min(8)))
        .min(max_delay_ms)
}

fn jittered_retry_delay_ms(base_delay_ms: u64, jitter_percent: u8) -> u64 {
    let entropy = retry_jitter_entropy().unwrap_or(base_delay_ms);
    jittered_retry_delay_with_entropy(base_delay_ms, jitter_percent, entropy)
}

fn jittered_retry_delay_with_entropy(
    base_delay_ms: u64,
    jitter_percent: u8,
    entropy: u64,
) -> u64 {
    if base_delay_ms == 0 || jitter_percent == 0 {
        return base_delay_ms;
    }

    let max_extra_ms = base_delay_ms
        .saturating_mul(u64::from(jitter_percent))
        .saturating_div(100);
    if max_extra_ms == 0 {
        return base_delay_ms;
    }

    let extra_ms = entropy % max_extra_ms.saturating_add(1);
    base_delay_ms.saturating_add(extra_ms)
}

fn retry_jitter_entropy() -> Option<u64> {
    let mut bytes = [0_u8; 8];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes))
}

#[allow(clippy::too_many_arguments)]
async fn fetch_segment_stream_with_retry(
    client: &reqwest::Client,
    url: &str,
    request: FetchSegmentRequest,
    timeout: Duration,
    retry_policy: RetryPolicy,
    request_referer: Option<&str>,
    request_cookies: Option<&str>,
    control: &SegmentRuntimeControl,
) -> Result<FetchSegmentOutcome, String> {
    let mut attempt = 0u32;
    loop {
        if control.is_canceled() {
            return Err("segment-canceled".to_string());
        }
        let start = request.start;
        let end = request.end;
        let range_header = format!("bytes={start}-{end}");
        let request_started = Instant::now();
        let outcome = apply_request_cookies(
            apply_request_referer(
                client.get(url).header("Range", &range_header),
                request_referer,
            ),
            request_cookies,
        )
        .timeout(timeout)
        .send()
        .await;
        match outcome {
            Ok(response) if response.status().is_success() => {
                if let Err(error) =
                    validate_range_response(response.status(), response.headers(), request)
                {
                    if attempt >= retry_policy.budget {
                        return Err(error);
                    }
                    if control.is_canceled() {
                        return Err("segment-canceled".to_string());
                    }
                    let delay = retry_policy.delay_ms(attempt, false, None);
                    sleep(Duration::from_millis(delay)).await;
                    attempt = attempt.saturating_add(1);
                    continue;
                }
                let negotiated_protocol = Some(protocol_label(response.version()).to_string());
                let ttfb_ms = request_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                return Ok(FetchSegmentOutcome {
                    response,
                    ttfb_ms,
                    connection_reused_hint: request.connection_reused_hint,
                    negotiated_protocol,
                    retry_attempts: attempt,
                });
            }
            Ok(response) => {
                let status = response.status();
                let throttled = status == StatusCode::TOO_MANY_REQUESTS
                    || status == StatusCode::SERVICE_UNAVAILABLE;
                let retry_limit = if throttled {
                    retry_policy.budget.saturating_add(THROTTLE_EXTRA_RETRIES)
                } else {
                    retry_policy.budget
                };
                let retry_after_ms = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(parse_retry_after_delay_ms);
                if attempt >= retry_limit {
                    if let Some(delay_ms) = retry_after_ms {
                        return Err(format!(
                            "HTTP {} for range {}-{} after {} retries (retry-after {}ms).",
                            status, start, end, retry_limit, delay_ms
                        ));
                    }
                    return Err(format!(
                        "HTTP {} for range {}-{} after {} retries.",
                        status, start, end, retry_limit
                    ));
                }
                if control.is_canceled() {
                    return Err("segment-canceled".to_string());
                }
                let delay = retry_policy.delay_ms(attempt, throttled, retry_after_ms);
                sleep(Duration::from_millis(delay)).await;
                attempt = attempt.saturating_add(1);
                continue;
            }
            Err(error) => {
                if attempt >= retry_policy.budget {
                    return Err(format!(
                        "Network error for range {}-{} after {} retries: {}",
                        start, end, retry_policy.budget, error
                    ));
                }
            }
        }
        if control.is_canceled() {
            return Err("segment-canceled".to_string());
        }
        let delay = retry_policy.delay_ms(attempt, false, None);
        sleep(Duration::from_millis(delay)).await;
        attempt = attempt.saturating_add(1);
    }
}

fn parse_retry_after_delay_ms(value: &reqwest::header::HeaderValue) -> Option<u64> {
    let raw = value.to_str().ok()?.trim();
    let seconds = raw.parse::<u64>().ok()?.min(RETRY_AFTER_MAX_SECONDS);
    Some(seconds.saturating_mul(1_000))
}
