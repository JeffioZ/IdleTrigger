//! Self-update for the single-file Windows EXE through GitHub Releases.
//!
//! Checks are panel-driven: the control panel fires one throttled check each
//! time it is shown, and the tray menu plus the settings page offer explicit
//! manual checks. A found update surfaces in the panel's update strip — one
//! confirm downloads, verifies and restarts, with a link to the release page
//! for the notes. Metadata prefers the release's `SHA256SUMS.txt` asset
//! (github.com host, no API quota) with the GitHub API as fallback; the new
//! EXE downloads to a `.part` file, passes the size/MZ/SHA-256 chain, and a
//! PowerShell script performs the atomic replace-and-restart after the
//! process exits. Startup cleanup reclaims the staging folder only after the
//! running version proves the replacement succeeded.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

pub(crate) const APP_REPO: &str = "JeffioZ/IdleTrigger";

// Release asset for the running architecture; release.yml uploads both plus
// SHA256SUMS.txt ("<hex> *<name>", x86 first).
#[cfg(target_arch = "x86_64")]
pub(crate) const ASSET_NAME: &str = "IdleTrigger-x64.exe";
#[cfg(target_arch = "x86")]
pub(crate) const ASSET_NAME: &str = "IdleTrigger-x86.exe";
#[cfg(not(any(target_arch = "x86_64", target_arch = "x86")))]
pub(crate) const ASSET_NAME: &str = "IdleTrigger.exe";

const CHECK_TIMEOUTS: (i32, i32, i32, i32) = (5000, 5000, 5000, 8000);
const DOWNLOAD_TIMEOUTS: (i32, i32, i32, i32) = (10000, 10000, 10000, 30000);
// The release EXE is a few MB; the cap only guards against pathological or
// hostile responses filling the disk.
const MAX_EXE_BYTES: u64 = 128 * 1024 * 1024;
const MIN_EXE_BYTES: u64 = 256 * 1024;
// Panel-show checks stay at most one per minute; the check itself is silent.
const CHECK_COOLDOWN: Duration = Duration::from_secs(60);
const PENDING_APPLY_MARKER: &str = "pending-apply";
const READY_FILE: &str = "ready.json";
const REPLACE_SCRIPT: &str = "update.ps1";
const REPLACE_ERROR_LOG: &str = "replace-error.log";

// ---- UI-facing state (read by the panel) ----------------------------------

/// Lifecycle of the update flow, mirrored onto the panel's header link.
#[derive(Clone, PartialEq)]
pub(crate) enum Phase {
    /// No known update (or the check has not run yet).
    Idle,
    Checking,
    /// A newer release exists; nothing downloaded yet.
    Available(String),
    Downloading {
        version: String,
        percent: u32,
    },
    /// Staged and verified; a restart applies it.
    Ready(String),
}

static PHASE: Mutex<Phase> = Mutex::new(Phase::Idle);
/// Newest version seen by the last successful check (drives the retry caption
/// after a failed apply).
static KNOWN_UPDATE: Mutex<Option<String>> = Mutex::new(None);
/// Failed apply errors waiting to be shown on the UI thread.
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static COOLDOWN_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
static BUSY: AtomicBool = AtomicBool::new(false);
/// An explicit (tray/settings) check is queued but its thread has not yet
/// taken the BUSY gate; panel-show auto checks yield to it.
static MANUAL_REQUESTED: AtomicBool = AtomicBool::new(false);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    crate::runtime::lock(mutex)
}

pub(crate) fn phase() -> Phase {
    lock(&PHASE).clone()
}

fn is_busy() -> bool {
    BUSY.load(Ordering::SeqCst)
}

/// Errors queued for the UI thread (dialog), drained once each.
pub(crate) fn take_error() -> Option<String> {
    lock(&LAST_ERROR).take()
}

/// Set once the replace script is running; the UI thread exits on drain.
pub(crate) fn take_exit_request() -> bool {
    EXIT_REQUESTED.swap(false, Ordering::SeqCst)
}

/// One-shot dialog texts queued for the UI thread: "already up to date"
/// after a manual check, or the devtools preview's end-of-flow notice.
static FEEDBACK: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn take_feedback() -> Option<String> {
    lock(&FEEDBACK).take()
}

/// An explicit check (tray menu or the settings button) is waiting for its
/// answer: when the round that finishes while this is latched finds a newer
/// release, the UI thread opens the one-click confirm dialog automatically
/// instead of leaving the user to click a caption that merely changed.
/// Consumed exactly once — including by an in-flight auto check the click
/// landed on.
static PROMPT_AFTER_CHECK: AtomicBool = AtomicBool::new(false);
static UPDATE_PROMPT_PENDING: AtomicBool = AtomicBool::new(false);

/// Queued request for the UI thread to open the one-click update confirm
/// (the same dialog the panel's update link opens).
pub(crate) fn take_update_prompt() -> bool {
    UPDATE_PROMPT_PENDING.swap(false, Ordering::SeqCst)
}

/// The update link's click payload: the target version (release notes live
/// on the release page, opened from the panel's notes link).
pub(crate) fn pending_update_version() -> Option<String> {
    match &*lock(&PHASE) {
        Phase::Available(version) | Phase::Ready(version) => Some(version.clone()),
        _ => None,
    }
}

// ---- Check entry points (all three funnel into run_check) ------------------

/// The panel was just shown: run one silent check unless the config switch is
/// off, a check/download is already running, an explicit check is in flight,
/// or the cooldown is active. Devtools preview bypasses the gates so the
/// simulated state always presents.
pub(crate) fn on_panel_shown() {
    #[cfg(feature = "devtools")]
    let preview = crate::devtools::UPDATE_PREVIEW.load(Ordering::SeqCst);
    #[cfg(not(feature = "devtools"))]
    let preview = false;
    if !preview
        && (!crate::cfg_map(|c| c.update_check_enabled)
            || is_busy()
            || MANUAL_REQUESTED.load(Ordering::SeqCst))
    {
        return;
    }
    if !preview
        && let Some(until) = lock(&COOLDOWN_UNTIL).as_ref()
        && Instant::now() < *until
    {
        return;
    }
    spawn_check(false);
}

/// Explicit manual check (tray menu / settings button): ignores the config
/// switch and the cooldown; a "no update" outcome answers with an up-to-date
/// dialog. The pending flag makes a panel-show auto check yield to it.
pub(crate) fn manual_check() {
    if is_busy() {
        return;
    }
    MANUAL_REQUESTED.store(true, Ordering::SeqCst);
    spawn_check(true);
}

/// Settings-button check: when the round this starts finds a newer release,
/// the one-click confirm opens by itself — the user's click already said
/// "check", so a caption that merely changed to "Update to vX" forces a
/// second click for no reason. Callers keep ignoring clicks while a check
/// or download is already running.
pub(crate) fn manual_check_prompting() {
    PROMPT_AFTER_CHECK.store(true, Ordering::SeqCst);
    manual_check();
}

/// Tray-menu "Check for updates": the panel is NOT shown. Every outcome
/// arrives as a standalone dialog — up to date / failure via the existing
/// feedback and error boxes, "update available" via the update confirm. If
/// a check is already running, the request still latches and that in-flight
/// round answers it.
pub(crate) fn manual_check_from_tray() {
    PROMPT_AFTER_CHECK.store(true, Ordering::SeqCst);
    if is_busy() {
        return;
    }
    MANUAL_REQUESTED.store(true, Ordering::SeqCst);
    spawn_check(true);
}

/// Single spawn site for checks: run_check owns the BUSY gate, so a losing
/// thread exits immediately without disturbing the winner. A failed spawn
/// clears the manual flag — a stuck flag would disable auto checks forever.
fn spawn_check(manual: bool) {
    let spawned = std::thread::Builder::new()
        .name(if manual {
            "self-update-manual".into()
        } else {
            "self-update".into()
        })
        .spawn(move || {
            crate::runtime::catch_and_log("self-update", || run_check(manual));
        });
    if spawned.is_err() {
        MANUAL_REQUESTED.store(false, Ordering::SeqCst);
        if PROMPT_AFTER_CHECK.swap(false, Ordering::SeqCst) {
            // The explicit-check user is waiting on a dialog answer.
            *lock(&FEEDBACK) = Some(crate::t_pub("update_err_network"));
        }
        crate::log_line("self-update: failed to spawn the check thread");
    }
}

/// The panel link was confirmed: download (if needed), apply and request the
/// exit. Runs the whole tail on a worker; failures surface via `take_error`.
pub(crate) fn begin_update() {
    if is_busy() {
        return;
    }
    let Some(version) = pending_update_version() else {
        return;
    };
    #[cfg(feature = "devtools")]
    if crate::devtools::UPDATE_PREVIEW.load(Ordering::SeqCst) {
        let _ = std::thread::Builder::new()
            .name("self-update-preview".into())
            .spawn(move || {
                crate::runtime::catch_and_log("self-update-preview", || preview_apply(version));
            });
        return;
    }
    let _ = std::thread::Builder::new()
        .name("self-update-apply".into())
        .spawn(move || {
            crate::runtime::catch_and_log("self-update-apply", || apply_flow(version));
        });
}

/// RAII release of the update BUSY flag: whichever flow holds the slot
/// clears it on drop, including through a panic unwind.
struct BusyGuard;
impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.store(false, Ordering::Release);
    }
}

/// One silent check: atom discovery → prerelease filter → semver compare.
/// Never dialogs on failure; the phase just falls back to what is known.
/// `manual` (tray menu) additionally answers "already up to date".
fn run_check(manual: bool) {
    if BUSY.swap(true, Ordering::SeqCst) {
        MANUAL_REQUESTED.store(false, Ordering::SeqCst);
        return;
    }
    MANUAL_REQUESTED.store(false, Ordering::SeqCst);
    let _reset = BusyGuard;

    #[cfg(feature = "devtools")]
    if crate::devtools::UPDATE_PREVIEW.load(Ordering::SeqCst) {
        preview_inject();
        PROMPT_AFTER_CHECK.store(false, Ordering::SeqCst);
        notify_ui(true);
        return;
    }

    *lock(&PHASE) = Phase::Checking;
    notify_ui(true);
    match fetch_latest() {
        Ok(None) => {
            *lock(&KNOWN_UPDATE) = None;
            *lock(&PHASE) = Phase::Idle;
            // A latched tray request gets its dialog answer even when the
            // round itself was an auto check the click landed on.
            let explicit_answer = PROMPT_AFTER_CHECK.swap(false, Ordering::SeqCst);
            if manual || explicit_answer {
                *lock(&FEEDBACK) = Some(crate::t_args("update_up_to_date", &[crate::APP_VERSION]));
            }
        }
        Ok(Some(version)) => {
            *lock(&KNOWN_UPDATE) = Some(version.clone());
            let mut phase = lock(&PHASE);
            if !matches!(&*phase, Phase::Ready(ready) if *ready == version) {
                *phase = Phase::Available(version.clone());
            }
            drop(phase);
            // Explicit check found a newer release: queue the confirm.
            if PROMPT_AFTER_CHECK.swap(false, Ordering::SeqCst) {
                UPDATE_PROMPT_PENDING.store(true, Ordering::SeqCst);
            }
        }
        Err(error) => {
            crate::log_line(&format!("self-update: check failed: {error}"));
            *lock(&PHASE) = fallback_phase();
            let explicit_answer = PROMPT_AFTER_CHECK.swap(false, Ordering::SeqCst);
            if manual || explicit_answer {
                // Timeouts/transport errors/HTTP rejections all land here
                // (WinHTTP bounds each phase: 5s connect, 8s receive), so an
                // explicit check never hangs the button. Failure surfaces as
                // a warning dialog (LAST_ERROR), keeping FEEDBACK for the
                // informational "already up to date" answer.
                *lock(&LAST_ERROR) = Some(error);
            }
        }
    }
    stamp_cooldown();
    notify_ui(true);
}

/// Download (unless already staged), verify and request the exit. One confirm
/// covers the whole tail — no second click before the restart.
fn apply_flow(version: String) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let _reset = BusyGuard;

    let outcome = (|| -> Result<(), String> {
        if lock(&PHASE).clone() != Phase::Ready(version.clone()) {
            *lock(&PHASE) = Phase::Downloading {
                version: version.clone(),
                percent: 0,
            };
            notify_ui(true);
            let meta = fetch_release_meta(&version)?;
            download_staged(&meta, &version)?;
        }
        // Re-verifies the staged file before writing the replace script.
        apply_ready_update()
    })();
    match outcome {
        Ok(()) => {
            EXIT_REQUESTED.store(true, Ordering::SeqCst);
        }
        Err(error) => {
            crate::log_line(&format!("self-update: applying {version} failed: {error}"));
            *lock(&LAST_ERROR) = Some(error);
            *lock(&PHASE) = fallback_phase();
        }
    }
    notify_ui(true);
}

// ---- Devtools update preview (feature-gated; no network, no replace) ------

/// Injects the simulated release ("9.9.9") whenever the phase is idle, so
/// every panel show presents the available-update state.
#[cfg(feature = "devtools")]
fn preview_inject() {
    const PREVIEW_VERSION: &str = "9.9.9";
    *lock(&KNOWN_UPDATE) = Some(PREVIEW_VERSION.to_string());
    if !matches!(&*lock(&PHASE), Phase::Ready(_)) {
        *lock(&PHASE) = Phase::Available(PREVIEW_VERSION.to_string());
    }
}

/// Simulated apply, mirroring the real one-confirm tail: an animated
/// download (percent ticks via the same throttled UI posts) straight into
/// the exit handover — which the preview replaces with an end-of-tour notice
/// (it never exits or replaces anything).
#[cfg(feature = "devtools")]
fn preview_apply(version: String) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    let _reset = BusyGuard;

    if lock(&PHASE).clone() != Phase::Ready(version.clone()) {
        *lock(&PHASE) = Phase::Downloading {
            version: version.clone(),
            percent: 0,
        };
        notify_ui(true);
        let mut percent: u32 = 0;
        while percent < 100 {
            std::thread::sleep(Duration::from_millis(250));
            percent = (percent + 9).min(100);
            if let Phase::Downloading { percent: slot, .. } = &mut *lock(&PHASE) {
                *slot = percent;
            }
            notify_ui(false);
        }
        *lock(&PHASE) = Phase::Ready(version);
        notify_ui(true);
    }
    *lock(&FEEDBACK) = Some(crate::t_pub("update_preview_feedback"));
    notify_ui(true);
}
/// The phase a failed round falls back to: a verified staged update stays
/// ready (offline install), else the known newer version, else idle.
fn fallback_phase() -> Phase {
    if let Some((version, _)) = verified_ready() {
        return Phase::Ready(version);
    }
    match &*lock(&KNOWN_UPDATE) {
        Some(version) => Phase::Available(version.clone()),
        None => Phase::Idle,
    }
}

fn stamp_cooldown() {
    *lock(&COOLDOWN_UNTIL) = Some(Instant::now() + CHECK_COOLDOWN);
}

/// A staged update that verifies against its recorded SHA-256 and differs
/// from the running build (shared by the failure fallback and the startup
/// restore).
fn verified_ready() -> Option<(String, String)> {
    let (version, sha256) = ready_state()?;
    if version == crate::APP_VERSION || verify_staged(&staged_path(), &sha256).is_err() {
        return None;
    }
    Some((version, sha256))
}

/// Restores the Ready phase for a staged update found at startup so the next
/// panel show offers "restart and update" without waiting for a fresh check.
pub(crate) fn restore_ready_phase() {
    if let Some((version, _)) = verified_ready() {
        *lock(&PHASE) = Phase::Ready(version);
    }
}

// ---- Release metadata ------------------------------------------------------

struct ReleaseMeta {
    url: String,
    sha256: String,
}

/// Digest lookup: SHA256SUMS.txt first (github.com host, no API quota), API
/// metadata as the fallback for releases published before the sums file.
fn fetch_release_meta(version: &str) -> Result<ReleaseMeta, String> {
    match fetch_sums_meta(version) {
        Ok(meta) => Ok(meta),
        Err(sidecar_error) => {
            crate::log_line(&format!(
                "self-update: SHA256SUMS unavailable, falling back to the GitHub API ({sidecar_error})"
            ));
            fetch_api_meta(version)
        }
    }
}

fn fetch_sums_meta(version: &str) -> Result<ReleaseMeta, String> {
    let base = format!("https://github.com/{APP_REPO}/releases/download/v{version}");
    let sums_url = format!("{base}/SHA256SUMS.txt");
    let response = http_get(&sums_url, CHECK_TIMEOUTS, 64 * 1024)?;
    if response.status != 200 {
        return Err(t_http_error(response.status));
    }
    let text = String::from_utf8_lossy(&response.body).into_owned();
    let sha256 = parse_sha256sums(&text, ASSET_NAME)
        .ok_or_else(|| crate::t_pub("update_err_digest_mismatch"))?;
    Ok(ReleaseMeta {
        url: format!("{base}/{ASSET_NAME}"),
        sha256,
    })
}

/// Finds this build's digest inside SHA256SUMS.txt (`<hex> *<name>` per line,
/// binary marker and CRLF tolerated; the digest binds to the exact asset
/// name, so a mismatched line is rejected).
fn parse_sha256sums(text: &str, expected_name: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let Some((hex, name)) = line.split_once(' ') else {
            continue;
        };
        let name = name.trim_start_matches([' ', '*']);
        if name == expected_name
            && hex.len() == 64
            && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

enum ApiRejection {
    RateLimit { minutes: u64 },
    RetryAfter { minutes: u64 },
    Status(u16),
}

/// Only 403/429 classify as rejections: the primary rate limit carries a
/// reset countdown, the secondary a retry-after; anything else keeps its
/// status code. `minutes == 0` means the reset is missing or already passed.
fn classify_api_rejection(
    status: u16,
    remaining: Option<&str>,
    reset: Option<&str>,
    retry_after: Option<&str>,
    now_unix: u64,
) -> ApiRejection {
    if matches!(status, 403 | 429) {
        if remaining == Some("0") {
            let minutes = reset
                .and_then(|value| value.parse::<u64>().ok())
                .map(|reset| reset.saturating_sub(now_unix))
                .map(|until| until.div_ceil(60))
                .unwrap_or(0);
            return ApiRejection::RateLimit { minutes };
        }
        if let Some(minutes) = retry_after
            .and_then(|value| value.parse::<u64>().ok())
            .map(|secs| secs.div_ceil(60))
        {
            return ApiRejection::RetryAfter { minutes };
        }
    }
    ApiRejection::Status(status)
}

fn describe_api_rejection(status: u16, rejection: &ApiRejection) -> String {
    match rejection {
        ApiRejection::RateLimit { minutes } if *minutes > 0 => {
            crate::t_pub("update_err_rate_limit").replacen("%d", &minutes.to_string(), 1)
        }
        ApiRejection::RetryAfter { minutes } if *minutes > 0 => {
            crate::t_pub("update_err_secondary_limit").replacen("%d", &minutes.to_string(), 1)
        }
        // The zero-minute guards fall through here too: a reset that already
        // passed reads as a plain rejection with the original status code.
        _ => t_http_error(match rejection {
            ApiRejection::Status(code) => *code,
            _ => status,
        }),
    }
}

fn t_http_error(status: u16) -> String {
    crate::t_pub("update_err_http").replacen("%d", &status.to_string(), 1)
}

fn fetch_api_meta(version: &str) -> Result<ReleaseMeta, String> {
    let url = format!("https://api.github.com/repos/{APP_REPO}/releases/tags/v{version}");
    let response = http_get(&url, CHECK_TIMEOUTS, 1024 * 1024)?;
    if response.status != 200 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        let header = |name: &str| response.header(name);
        let rejection = classify_api_rejection(
            response.status,
            header("x-ratelimit-remaining").as_deref(),
            header("x-ratelimit-reset").as_deref(),
            header("retry-after").as_deref(),
            now,
        );
        return Err(describe_api_rejection(response.status, &rejection));
    }
    let json: Json =
        serde_json::from_slice(&response.body).map_err(|_| crate::t_pub("update_err_read"))?;
    let (asset_url, sha256) = parse_api_asset(&json, version)?;
    Ok(ReleaseMeta {
        url: asset_url,
        sha256,
    })
}

/// Validates the API release payload against the check result: exact tag,
/// published state, exactly one uploaded Windows asset, a deterministic
/// github.com download URL, and a GitHub-computed SHA-256 digest.
fn parse_api_asset(json: &Json, expected_version: &str) -> Result<(String, String), String> {
    let expected_tag = format!("v{expected_version}");
    let matches_str =
        |value: Option<&Json>, expected: &str| value.and_then(Json::as_str) == Some(expected);
    if !matches_str(json.get("tag_name"), &expected_tag)
        || json.get("draft").and_then(Json::as_bool).unwrap_or(true)
        || json
            .get("prerelease")
            .and_then(Json::as_bool)
            .unwrap_or(true)
    {
        return Err(crate::t_pub("update_err_version_mismatch"));
    }
    let matching: Vec<&Json> = json
        .get("assets")
        .and_then(Json::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter(|asset| {
                    asset.get("name").and_then(Json::as_str) == Some(ASSET_NAME)
                        && asset.get("state").and_then(Json::as_str) == Some("uploaded")
                })
                .collect()
        })
        .unwrap_or_default();
    if matching.len() != 1 {
        return Err(crate::t_pub("update_err_asset_missing"));
    }
    let asset = matching[0];
    let asset_url = asset
        .get("browser_download_url")
        .and_then(Json::as_str)
        .ok_or_else(|| crate::t_pub("update_err_unexpected_url"))?;
    // Stripping the scheme+host also removes the leading path slash, so the
    // expected tail starts at the repository segment.
    let expected_path = format!("{APP_REPO}/releases/download/{expected_tag}/{ASSET_NAME}");
    let valid_url = asset_url
        .strip_prefix("https://github.com/")
        .is_some_and(|rest| {
            let (host_path, query_fragment) = rest
                .split_once(['?', '#'])
                .map_or((rest, ""), |(path, tail)| (path, tail));
            host_path == expected_path && query_fragment.is_empty()
        });
    if !valid_url {
        return Err(crate::t_pub("update_err_unexpected_url"));
    }
    let sha256 = asset
        .get("digest")
        .and_then(Json::as_str)
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| crate::t_pub("update_err_digest_invalid"))?;
    Ok((asset_url.to_string(), sha256.to_ascii_lowercase()))
}

// ---- Version discovery -----------------------------------------------------

/// Compares two versions with SemVer 2.0 precedence (prerelease identifiers
/// included, build metadata ignored). Unparseable input degrades to a stable
/// string comparison instead of failing the check.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn normalize(value: &str) -> &str {
        value.trim().trim_start_matches('v')
    }
    let (a, b) = (normalize(a), normalize(b));
    match (parse_semver(a), parse_semver(b)) {
        (Some(a), Some(b)) => cmp_semver(&a, &b),
        _ => a.cmp(b),
    }
}

#[derive(PartialEq, Eq)]
enum PreId {
    Num(u64),
    Alnum(String),
}

struct SemVer {
    core: [u64; 3],
    pre: Vec<PreId>,
}

fn parse_semver(value: &str) -> Option<SemVer> {
    let (main, _build) = value.split_once('+').unwrap_or((value, ""));
    let (core, pre) = main.split_once('-').unwrap_or((main, ""));
    let mut parts = core.split('.');
    let mut numbers = [0u64; 3];
    for slot in numbers.iter_mut() {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    let pre = if pre.is_empty() {
        Vec::new()
    } else {
        pre.split('.')
            .filter(|id| !id.is_empty())
            .map(|id| match id.parse::<u64>() {
                Ok(number) if !id.starts_with('0') || number == 0 && id.len() == 1 => {
                    PreId::Num(number)
                }
                _ => PreId::Alnum(id.to_string()),
            })
            .collect()
    };
    Some(SemVer { core: numbers, pre })
}

fn cmp_semver(a: &SemVer, b: &SemVer) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let core = a.core.cmp(&b.core);
    if core != Ordering::Equal {
        return core;
    }
    match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, true) => Ordering::Equal,
        // A release outranks any prerelease of the same core version.
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (a_id, b_id) in a.pre.iter().zip(b.pre.iter()) {
                let ordering = match (a_id, b_id) {
                    (PreId::Num(a), PreId::Num(b)) => a.cmp(b),
                    // Numeric identifiers always rank below alphanumeric ones.
                    (PreId::Num(_), PreId::Alnum(_)) => Ordering::Less,
                    (PreId::Alnum(_), PreId::Num(_)) => Ordering::Greater,
                    (PreId::Alnum(a), PreId::Alnum(b)) => a.cmp(b),
                };
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            a.pre.len().cmp(&b.pre.len())
        }
    }
}

/// Semver prerelease marker: any hyphen after the core version (rc, preview,
/// beta, alpha, nightly, …). The release workflow enforces strict semver
/// tags, so a hyphen always means prerelease; such releases never surface as
/// updates. GitHub's own prerelease flag is not carried by releases.atom, so
/// the tag itself is the contract — mark previews through the tag.
fn is_prerelease_tag(tag: &str) -> bool {
    let core = tag.trim_start_matches('v');
    let main = core.split_once('+').map_or(core, |(main, _)| main);
    main.contains('-')
}

/// First stable tag in atom order (newest release first); prerelease tags
/// are skipped so an unswitched rc never reports "update available", and
/// tags that do not parse as semver are skipped too — a stray non-release
/// tag ("nightly", "demo") must never pose as an update, since
/// compare_versions would degrade to a string comparison that can rank it
/// above the running version.
fn latest_stable_tag(tags: &[String]) -> Option<String> {
    tags.iter()
        .map(|tag| tag.trim_start_matches('v').to_string())
        .find(|tag| !is_prerelease_tag(tag) && parse_semver(tag).is_some())
}

/// Parses the tag list from a GitHub releases.atom page (newest first). Tags
/// come from each entry's `<link rel="alternate">` href tail; the title can
/// be a custom release name and is not trustworthy.
fn parse_releases_atom(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for entry in xml.split("<entry>").skip(1) {
        let end = entry.find("</entry>").unwrap_or(entry.len());
        let entry = &entry[..end];
        let mut tag = None;
        for link in entry.split("<link").skip(1) {
            let seg = &link[..link.find('>').unwrap_or(link.len())];
            if !seg.contains("releases/tag/") {
                continue;
            }
            let Some(start) = seg.find("href=\"") else {
                continue;
            };
            let rest = &seg[start + 6..];
            let Some(quote) = rest.find('"') else {
                continue;
            };
            let href = &rest[..quote];
            if let Some(last) = href.rsplit('/').next() {
                tag = Some(last.to_string());
                break;
            }
        }
        if let Some(tag) = tag {
            out.push(tag);
        }
    }
    out
}

/// Atom discovery + compare: `Some(version)` for a newer stable release,
/// `Ok(None)` when current, errors for feed/transport problems.
fn fetch_latest() -> Result<Option<String>, String> {
    let url = format!("https://github.com/{APP_REPO}/releases.atom");
    let text = http_get_text(&url, 256 * 1024)?;
    let tags = parse_releases_atom(&text);
    let Some(latest) = latest_stable_tag(&tags) else {
        return Err(crate::t_pub("update_err_no_stable"));
    };
    let update_available =
        compare_versions(&latest, crate::APP_VERSION) == std::cmp::Ordering::Greater;
    Ok(update_available.then_some(latest))
}

// ---- Download and verification ---------------------------------------------

fn update_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("update")
}

fn staged_path() -> PathBuf {
    update_dir().join(ASSET_NAME)
}

fn part_path() -> PathBuf {
    update_dir().join(format!("{ASSET_NAME}.part"))
}

/// ready.json: the staged version + digest, persisted so a staged update
/// survives restarts (the panel prompt then offers to apply it).
fn ready_state() -> Option<(String, String)> {
    let text = std::fs::read_to_string(update_dir().join(READY_FILE)).ok()?;
    let value: Json = serde_json::from_str(&text).ok()?;
    Some((
        value.get("version")?.as_str()?.to_string(),
        value.get("sha256")?.as_str()?.to_string(),
    ))
}

fn write_ready(version: &str, sha256: &str) -> Result<(), String> {
    let document = serde_json::json!({
        "version": version,
        "sha256": sha256,
    });
    std::fs::create_dir_all(update_dir()).map_err(|_| crate::t_pub("update_err_dir"))?;
    // Write-then-rename: a crash mid-write must not leave a truncated
    // ready.json — that would strand the verified staged file as an orphan
    // reporting "no update pending" until the next successful round.
    let target = update_dir().join(READY_FILE);
    let temp = update_dir().join(format!("{READY_FILE}.new"));
    std::fs::write(&temp, serde_json::to_string(&document).unwrap_or_default())
        .map_err(|error| crate::t_args("update_err_write", &[&error.to_string()]))?;
    if let Err(error) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(crate::t_args("update_err_write", &[&error.to_string()]));
    }
    Ok(())
}

/// Existence + size floor + MZ header + SHA-256: the staged file must prove
/// itself again right before every use, covering the window between staging
/// and applying.
fn verify_staged(path: &Path, expected_sha256: &str) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| crate::t_pub("update_err_no_ready"))?;
    if meta.len() < MIN_EXE_BYTES {
        return Err(crate::t_pub("update_err_too_small"));
    }
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut mz = [0u8; 2];
    file.read_exact(&mut mz)
        .map_err(|error| error.to_string())?;
    if &mz != b"MZ" {
        return Err(crate::t_pub("update_err_no_mz"));
    }
    drop(file);
    let actual = sha256_file(path)?;
    if actual != expected_sha256.to_ascii_lowercase() {
        return Err(crate::t_pub("update_err_hash"));
    }
    Ok(())
}

/// Downloads the release EXE to the `.part` file with progress reporting,
/// then verifies and promotes it to the staged name. Failures clean up the
/// half-written file.
fn download_staged(meta: &ReleaseMeta, version: &str) -> Result<(), String> {
    let dir = update_dir();
    std::fs::create_dir_all(&dir).map_err(|_| crate::t_pub("update_err_dir"))?;
    let part = part_path();
    let _ = std::fs::remove_file(&part);
    let mut progress = |done: u64, total: u64| {
        let percent = done
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or(0)
            .min(100) as u32;
        if let Phase::Downloading { percent: slot, .. } = &mut *lock(&PHASE) {
            *slot = percent;
        }
        notify_ui(false);
    };
    // http_download already removed the half-written .part on failure.
    http_download(&meta.url, &part, MAX_EXE_BYTES, &mut progress)?;
    if let Err(error) = verify_staged(&part, &meta.sha256) {
        let _ = std::fs::remove_file(&part);
        return Err(error);
    }
    // rename replaces an existing target on Windows, so no pre-delete:
    // deleting first would open a staged-file-less window a crash could
    // turn into a lost update.
    let staged = dir.join(ASSET_NAME);
    if let Err(error) = std::fs::rename(&part, &staged) {
        let _ = std::fs::remove_file(&part);
        return Err(error.to_string());
    }
    write_ready(version, &meta.sha256)?;
    crate::log_line(&format!(
        "self-update: {version} downloaded and verified, ready to apply"
    ));
    Ok(())
}

// ---- Apply (replace + restart) ---------------------------------------------

/// Applies the staged update: re-verifies, probes the install folder for
/// writability, writes the replacement script and starts it detached. The
/// caller then exits the process; the script replaces the EXE and restarts.
fn apply_ready_update() -> Result<(), String> {
    let Some((version, sha256)) = ready_state() else {
        return Err(crate::t_pub("update_err_no_ready"));
    };
    let dir = update_dir();
    let staged = dir.join(ASSET_NAME);
    // Second verification right before the swap covers "file replaced after
    // download completed".
    verify_staged(&staged, &sha256)?;

    // A restricted folder (Program Files) would doom the script after the
    // process is gone; bail out while the app is still alive instead.
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let probe = exe.with_file_name(".idletrigger-update-probe");
    if let Err(error) = std::fs::write(&probe, b"").and_then(|_| std::fs::remove_file(&probe)) {
        crate::log_line(&format!(
            "self-update: install folder not writable: {error}"
        ));
        return Err(crate::t_pub("update_err_not_writable"));
    }

    // Written with a BOM: Windows PowerShell 5.1 decodes BOM-less .ps1 as
    // ANSI, which mangles non-ASCII paths inside the script.
    let script = dir.join(REPLACE_SCRIPT);
    std::fs::write(
        &script,
        format!("\u{FEFF}{}", replace_script(&staged, &exe, &sha256)),
    )
    .map_err(|error| {
        crate::log_line(&format!(
            "self-update: writing the replace script failed: {error}"
        ));
        crate::t_pub("update_err_script")
    })?;

    // Resolve PowerShell from System32 explicitly: CreateProcess search order
    // prefers the app directory, closing a same-name-exe hijack surface.
    let powershell = std::env::var_os("SYSTEMROOT")
        .map(|root| PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .unwrap_or_else(|| PathBuf::from("powershell"));
    let mut command = std::process::Command::new(powershell);
    command
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script);
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
    if command.spawn().is_err() {
        return Err(crate::t_pub("update_err_spawn"));
    }

    // The next start reads this to decide whether the previous round
    // succeeded. A write failure never blocks the update; leftovers are
    // simply reclaimed after the next successful round.
    if let Err(error) = std::fs::write(dir.join(PENDING_APPLY_MARKER), &version) {
        crate::log_line(&format!(
            "self-update: writing the apply marker failed: {error}"
        ));
    }
    crate::log_line(&format!(
        "self-update: applying {version}, exiting so the replace script can restart the app ({exe:?})"
    ));
    Ok(())
}

/// Stages the new EXE beside the running one and atomically replaces via
/// `File.Replace`, re-verifying the digest inside the script before the swap.
/// `File.Replace` fails while the running EXE locks the target, hence the
/// bounded retry loop instead of an unconditional rename.
fn replace_script(source: &Path, destination: &Path, expected_sha256: &str) -> String {
    let ps_quote = |path: &Path| path.to_string_lossy().replace('\'', "''");
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         Start-Sleep -Seconds 2\n\
         $src = '{}'\n\
         $dst = '{}'\n\
         $new = $dst + '.new'\n\
         $old = $dst + '.old'\n\
         $expected = '{}'\n\
         $replaced = $false\n\
         try {{\n\
           if ((-not (Test-Path -LiteralPath $dst)) -and (Test-Path -LiteralPath $old)) {{ Move-Item -LiteralPath $old -Destination $dst -Force }}\n\
           Copy-Item -LiteralPath $src -Destination $new -Force\n\
           $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $new).Hash.ToLowerInvariant()\n\
           if ($actual -ne $expected) {{ throw 'staged executable digest mismatch' }}\n\
           $i = 0\n\
           while ($i -lt 60 -and -not $replaced) {{\n\
             try {{\n\
               if (Test-Path -LiteralPath $old) {{ Remove-Item -LiteralPath $old -Force }}\n\
               if (Test-Path -LiteralPath $dst) {{ [System.IO.File]::Replace($new, $dst, $old, $true) }} else {{ Move-Item -LiteralPath $new -Destination $dst -Force }}\n\
               $replaced = $true\n\
             }} catch {{ Start-Sleep -Milliseconds 500; $i++ }}\n\
           }}\n\
           if (-not $replaced) {{ throw 'unable to replace executable' }}\n\
           $process = Start-Process -FilePath $dst -WorkingDirectory (Split-Path -Parent $dst) -PassThru\n\
           Start-Sleep -Seconds 3\n\
           if (-not $process.HasExited -and (Test-Path -LiteralPath $old)) {{ Remove-Item -LiteralPath $old -Force }}\n\
         }} catch {{\n\
           if ((-not (Test-Path -LiteralPath $dst)) -and (Test-Path -LiteralPath $old)) {{ Copy-Item -LiteralPath $old -Destination $dst -Force }}\n\
           # Failure trail: the next start logs and deletes it (drain_replace_error).\n\
           $_ | Out-File -LiteralPath (Join-Path (Split-Path -Parent $src) '{REPLACE_ERROR_LOG}') -Encoding utf8\n\
           exit 1\n\
         }} finally {{\n\
           if (Test-Path -LiteralPath $new) {{ Remove-Item -LiteralPath $new -Force }}\n\
         }}\n",
        ps_quote(source),
        ps_quote(destination),
        expected_sha256.to_ascii_lowercase(),
    )
}

/// Reads and deletes the previous round's replace-error trail (UTF-8 with
/// BOM, as written by PowerShell's Out-File). None when absent.
fn drain_replace_error(dir: &Path) -> Option<String> {
    let content = std::fs::read_to_string(dir.join(REPLACE_ERROR_LOG))
        .ok()?
        .trim_start_matches('\u{FEFF}')
        .trim()
        .to_string();
    let _ = std::fs::remove_file(dir.join(REPLACE_ERROR_LOG));
    Some(content)
}

/// Startup reclamation of the previous round's leftovers: the staging folder
/// goes once the running version matches the apply marker; the `.old` backup
/// only then. Any mismatch (replacement failed, rollback pending) keeps
/// everything for the retry. Errors log and never block.
pub(crate) fn cleanup_on_startup() {
    let dir = update_dir();
    // Log the failure trail before any cleanup decision: with a failed
    // replacement the marker never matches, so the marker branch alone would
    // never surface this error.
    if let Some(error) = drain_replace_error(&dir) {
        crate::log_line(&format!(
            "self-update: the previous replace script failed (old version restored or kept for retry):\n{error}"
        ));
    }
    let part = part_path();
    if part.exists() {
        // Downloads never resume across runs; a leftover .part is junk.
        let _ = std::fs::remove_file(&part);
    }
    // A crash between the writability probe's write and remove leaves this
    // marker file beside the EXE; reclaim it with the other leftovers.
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_file_name(".idletrigger-update-probe"));
    }
    let exe_old = std::env::current_exe().ok().map(|exe| {
        let mut old = exe.into_os_string();
        old.push(".old");
        PathBuf::from(old)
    });
    if cleanup_applied_update_in(&dir, crate::APP_VERSION, exe_old.as_deref()) {
        crate::log_line(
            "self-update: previous update confirmed, staging folder and old backup reclaimed",
        );
    }
}

/// Marker-driven cleanup decision (pure for tests): true when cleanup ran.
fn cleanup_applied_update_in(dir: &Path, running_version: &str, exe_old: Option<&Path>) -> bool {
    let marker = dir.join(PENDING_APPLY_MARKER);
    let Ok(target) = std::fs::read_to_string(&marker) else {
        return false; // No marker: the last round never entered the replace flow.
    };
    let target = target.trim();
    if target.is_empty() || target != running_version {
        // The replacement did not finish (or another channel overwrote the
        // version): keep staging, backup and marker for rollback or the
        // still-running retry window; the next round rewrites the marker.
        return false;
    }
    // Conservative cleanup: delete ONLY the files this updater itself wrote,
    // then drop the folder when that leaves it empty. A user's own "update"
    // folder beside the EXE keeps every unrelated file — std::fs::remove_dir
    // refuses non-empty directories, which is exactly the guard we want.
    for name in [
        ASSET_NAME,
        &format!("{ASSET_NAME}.part"),
        READY_FILE,
        &format!("{READY_FILE}.new"),
        REPLACE_SCRIPT,
        REPLACE_ERROR_LOG,
    ] {
        let _ = std::fs::remove_file(dir.join(name));
    }
    // The marker goes last; an unknown leftover file simply keeps the folder.
    let _ = std::fs::remove_file(&marker);
    if dir.exists()
        && let Err(_error) = std::fs::remove_dir(dir)
    {
        // Non-empty (files we did not create) or a busy handle: keeping the
        // empty-ish folder behind is harmless and never worth a scary log.
        crate::log_line("self-update: staging folder kept; it holds files we did not create");
    }
    if let Some(old) = exe_old
        && old.exists()
        && let Err(error) = std::fs::remove_file(old)
    {
        crate::log_line(&format!(
            "self-update: removing the old backup failed: {error}"
        ));
    }
    true
}

// ---- UI plumbing -----------------------------------------------------------

/// Posts WM_REFRESH_UI so the panel refreshes the update link. Throttled
/// during download chunks; force=true on phase transitions.
fn notify_ui(force: bool) {
    static LAST_POST: Mutex<Option<Instant>> = Mutex::new(None);
    if !force {
        let mut last = lock(&LAST_POST);
        if let Some(at) = last.as_ref()
            && at.elapsed() < Duration::from_millis(200)
        {
            return;
        }
        *last = Some(Instant::now());
    }
    unsafe {
        let _ = PostMessageW(
            Some(crate::hwnd(&crate::HIDDEN)),
            crate::WM_REFRESH_UI,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

// ---- WinHTTP plumbing ------------------------------------------------------

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, lowercase_name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(name, _)| name == lowercase_name)
            .map(|(_, value)| value.clone())
    }
}

struct HandleGuard(*mut core::ffi::c_void);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Networking::WinHttp::WinHttpCloseHandle(self.0);
        }
    }
}

/// One connected GET request with its handle chain: session → URL crack →
/// connect → open → send → receive → status query. Shared by the buffered
/// GET and the streamed download, whose WinHTTP setup is identical; the
/// guards keep session and connection alive for as long as the request.
struct OpenRequest {
    _session: HandleGuard,
    _connect: HandleGuard,
    request: HandleGuard,
    status: u16,
}

fn open_get_request(
    url: &str,
    timeouts: (i32, i32, i32, i32),
    network_error: &dyn Fn(String) -> String,
) -> Result<OpenRequest, String> {
    use windows::Win32::Networking::WinHttp::{
        URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE,
        WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReceiveResponse, WinHttpSendRequest,
        WinHttpSetTimeouts,
    };
    use windows::core::PCWSTR;

    unsafe {
        let agent = format!("IdleTrigger/{}", crate::APP_VERSION);
        let session = HandleGuard(WinHttpOpen(
            PCWSTR(crate::wide(&agent).as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(network_error("WinHttpOpen failed".into()));
        }
        WinHttpSetTimeouts(session.0, timeouts.0, timeouts.1, timeouts.2, timeouts.3)
            .map_err(|error| network_error(error.to_string()))?;

        let url_wide: Vec<u16> = url.encode_utf16().collect();
        let mut parts = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwHostNameLength: u32::MAX,
            dwUrlPathLength: u32::MAX,
            dwExtraInfoLength: u32::MAX,
            ..Default::default()
        };
        WinHttpCrackUrl(&url_wide, 0, &mut parts)
            .map_err(|error| network_error(error.to_string()))?;
        if parts.lpszHostName.is_null() || parts.dwHostNameLength == 0 {
            return Err(network_error("invalid URL".into()));
        }
        let host = String::from_utf16_lossy(std::slice::from_raw_parts(
            parts.lpszHostName.0,
            parts.dwHostNameLength as usize,
        ));
        let mut path = String::new();
        if !parts.lpszUrlPath.is_null() && parts.dwUrlPathLength > 0 {
            path.push_str(&String::from_utf16_lossy(std::slice::from_raw_parts(
                parts.lpszUrlPath.0,
                parts.dwUrlPathLength as usize,
            )));
        }
        if !parts.lpszExtraInfo.is_null() && parts.dwExtraInfoLength > 0 {
            path.push_str(&String::from_utf16_lossy(std::slice::from_raw_parts(
                parts.lpszExtraInfo.0,
                parts.dwExtraInfoLength as usize,
            )));
        }
        if path.is_empty() {
            path.push('/');
        }
        let connect = HandleGuard(WinHttpConnect(
            session.0,
            PCWSTR(crate::wide(&host).as_ptr()),
            parts.nPort,
            0,
        ));
        if connect.0.is_null() {
            return Err(network_error("WinHttpConnect failed".into()));
        }
        let flags = if parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS {
            WINHTTP_FLAG_SECURE
        } else {
            WINHTTP_OPEN_REQUEST_FLAGS(0)
        };
        let request = HandleGuard(WinHttpOpenRequest(
            connect.0,
            PCWSTR(crate::wide("GET").as_ptr()),
            PCWSTR(crate::wide(&path).as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ));
        if request.0.is_null() {
            return Err(network_error("WinHttpOpenRequest failed".into()));
        }
        WinHttpSendRequest(request.0, None, None, 0, 0, 0)
            .map_err(|error| network_error(error.to_string()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|error| network_error(error.to_string()))?;

        let mut status: u32 = 0;
        let mut status_size = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            std::ptr::null_mut(),
        )
        .map_err(|error| network_error(error.to_string()))?;
        Ok(OpenRequest {
            _session: session,
            _connect: connect,
            request,
            status: status as u16,
        })
    }
}

/// One-shot GET that buffers the body (size-capped) and captures the response
/// headers as lowercase name/value pairs. Redirects are followed by default,
/// which releases/download needs to reach the CDN.
fn http_get(
    url: &str,
    timeouts: (i32, i32, i32, i32),
    max_bytes: usize,
) -> Result<HttpResponse, String> {
    use windows::Win32::Networking::WinHttp::{WinHttpQueryDataAvailable, WinHttpReadData};

    let network_error = |error: String| format!("{}: {error}", crate::t_pub("update_err_network"));
    let opened = open_get_request(url, timeouts, &network_error)?;
    let request = opened.request.0;
    unsafe {
        let headers = query_headers_block(request, &network_error)?;
        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available).is_err() {
                return Err(network_error("WinHttpQueryDataAvailable failed".into()));
            }
            if available == 0 {
                break;
            }
            if body.len() + available as usize > max_bytes.saturating_add(1) {
                return Err(crate::t_pub("update_err_oversize"));
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(request, chunk.as_mut_ptr().cast(), available, &mut read).is_err()
                || read == 0
            {
                return Err(network_error("WinHttpReadData failed".into()));
            }
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
        }
        if body.len() > max_bytes {
            return Err(crate::t_pub("update_err_oversize"));
        }
        Ok(HttpResponse {
            status: opened.status,
            headers,
            body,
        })
    }
}

/// Reads the whole raw header block once and splits it into lowercase pairs.
fn query_headers_block(
    request: *mut core::ffi::c_void,
    network_error: &dyn Fn(String) -> String,
) -> Result<Vec<(String, String)>, String> {
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_QUERY_RAW_HEADERS_CRLF, WinHttpQueryHeaders,
    };
    use windows::core::PCWSTR;
    unsafe {
        let mut buffer = vec![0u16; 16 * 1024];
        let mut size = (buffer.len() * std::mem::size_of::<u16>()) as u32;
        WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_RAW_HEADERS_CRLF,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|error| network_error(error.to_string()))?;
        let text = String::from_utf16_lossy(&buffer);
        let mut headers = Vec::new();
        for line in text.split("\r\n") {
            let Some((name, value)) = line.split_once(':') else {
                continue; // Status line and malformed lines.
            };
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
        Ok(headers)
    }
}

fn http_get_text(url: &str, max_bytes: usize) -> Result<String, String> {
    let response = http_get(url, CHECK_TIMEOUTS, max_bytes)?;
    if response.status != 200 {
        return Err(t_http_error(response.status));
    }
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

/// Streams a large response to `path`, aborting past `max_bytes` and
/// reporting (done, total) progress. The destination is removed on failure.
fn http_download(
    url: &str,
    path: &Path,
    max_bytes: u64,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), String> {
    use windows::Win32::Networking::WinHttp::{
        WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WinHttpQueryDataAvailable,
        WinHttpQueryHeaders, WinHttpReadData,
    };
    use windows::core::PCWSTR;

    let network_error = |error: String| format!("{}: {error}", crate::t_pub("update_err_network"));
    let opened = open_get_request(url, DOWNLOAD_TIMEOUTS, &network_error)?;
    let request = opened.request.0;
    if opened.status != 200 {
        return Err(t_http_error(opened.status));
    }
    let result = unsafe {
        let mut total: u64 = 0;
        let mut total_size: u32 = 0;
        let mut total_size_bytes = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut total_size as *mut u32).cast()),
            &mut total_size_bytes,
            std::ptr::null_mut(),
        )
        .is_ok()
        {
            total = u64::from(total_size);
            if total > max_bytes {
                return Err(crate::t_pub("update_err_download_size"));
            }
        }

        let mut file = std::fs::File::create(path)
            .map_err(|error| crate::t_args("update_err_write", &[&error.to_string()]))?;
        let mut done: u64 = 0;
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request, &mut available).is_err() {
                return Err(network_error("WinHttpQueryDataAvailable failed".into()));
            }
            if available == 0 {
                break;
            }
            if done + u64::from(available) > max_bytes {
                return Err(crate::t_pub("update_err_download_size"));
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if WinHttpReadData(request, chunk.as_mut_ptr().cast(), available, &mut read).is_err()
                || read == 0
            {
                return Err(network_error("WinHttpReadData failed".into()));
            }
            chunk.truncate(read as usize);
            file.write_all(&chunk)
                .map_err(|error| crate::t_args("update_err_write", &[&error.to_string()]))?;
            done += read as u64;
            progress(done, total);
        }
        file.flush()
            .map_err(|error| crate::t_args("update_err_write", &[&error.to_string()]))?;
        Ok(())
    };
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

// BCrypt SHA-256 (bcrypt.dll) keeps the runtime dependency set at "Windows
// system DLLs" — no hash crate.
fn sha256_file(path: &Path) -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{
        BCRYPT_ALG_HANDLE, BCRYPT_HASH_HANDLE, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS,
        BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
        BCryptHashData, BCryptOpenAlgorithmProvider,
    };

    struct HashGuard(BCRYPT_HASH_HANDLE);
    impl Drop for HashGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = BCryptDestroyHash(self.0);
            }
        }
    }

    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut algorithm = BCRYPT_ALG_HANDLE::default();
    unsafe {
        BCryptOpenAlgorithmProvider(
            &mut algorithm,
            windows::core::w!("SHA256"),
            windows::core::PCWSTR::null(),
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
        .ok()
        .map_err(|error| error.to_string())?;
    }
    let result = (|| -> Result<String, String> {
        let mut handle = BCRYPT_HASH_HANDLE::default();
        unsafe {
            BCryptCreateHash(algorithm, &mut handle, None, None, 0)
                .ok()
                .map_err(|error| error.to_string())?;
        }
        let _hash = HashGuard(handle);
        let mut digest = [0u8; 32];
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            unsafe {
                BCryptHashData(_hash.0, &buffer[..read], 0)
                    .ok()
                    .map_err(|error| error.to_string())?;
            }
        }
        unsafe {
            BCryptFinishHash(_hash.0, &mut digest, 0)
                .ok()
                .map_err(|error| error.to_string())?;
        }
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    })();
    unsafe {
        let _ = BCryptCloseAlgorithmProvider(algorithm, 0);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_numeric_and_prerelease() {
        use std::cmp::Ordering;
        assert_eq!(compare_versions("2.3.0", "2.3.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.10.0", "0.9.0"), Ordering::Greater);
        assert_eq!(compare_versions("v2.3.1", "2.3.0"), Ordering::Greater);
        // Prerelease identifiers follow semver precedence.
        assert_eq!(
            compare_versions("1.0.0-rc.2", "1.0.0-rc.1"),
            Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.0.0-rc.9", "1.0.0-rc.10"),
            Ordering::Less
        );
        assert_eq!(compare_versions("1.0.0", "1.0.0-rc.1"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0-rc.1", "1.0.0"), Ordering::Less);
        assert_eq!(
            compare_versions("1.0.0-alpha.9", "1.0.0-beta.1"),
            Ordering::Less
        );
        // Build metadata never changes precedence.
        assert_eq!(compare_versions("1.0.0+a", "1.0.0+b"), Ordering::Equal);
        // Unparseable input degrades to a stable string comparison: letters
        // sort after digits, so "ci" reads as newer than any release.
        assert_eq!(compare_versions("ci", "2.0.0"), Ordering::Greater);
    }

    #[test]
    fn releases_atom_parses_tags_in_order() {
        let xml = r#"<?xml version="1.0"?>
<feed><entry>
  <link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v0.2.0"/>
  <title>v0.2.0</title>
</entry><entry>
  <link rel="alternate" type="text/html" href="https://github.com/o/r/releases/tag/v0.1.0"/>
</entry></feed>"#;
        assert_eq!(parse_releases_atom(xml), vec!["v0.2.0", "v0.1.0"]);
        let self_link = "<entry><link rel=\"alternate\" href=\"https://github.com/o/r/releases/tag/v1.0.0\"/><link rel=\"self\" href=\"https://x/atom\"/></entry>";
        assert_eq!(parse_releases_atom(self_link), vec!["v1.0.0"]);
    }

    #[test]
    fn latest_stable_tag_skips_prereleases_and_takes_first_stable() {
        let tags = vec![
            "v2.0.0-rc.1".to_string(),
            "v1.9.1-dev.3".to_string(),
            "v1.9.0".to_string(),
            "v1.8.0-beta.2".to_string(),
        ];
        assert_eq!(latest_stable_tag(&tags).as_deref(), Some("1.9.0"));
        // Every hyphenated tag is a prerelease, whatever the suffix.
        let all_pre = vec![
            "v2.0.0-rc.1".to_string(),
            "v1.9.0-nightly".to_string(),
            "v1.9.0-preview".to_string(),
        ];
        assert_eq!(latest_stable_tag(&all_pre), None);
        // Build metadata is not a prerelease marker (kept verbatim; the
        // comparison ignores it).
        let build = vec!["v2.0.0+build.1".to_string()];
        assert_eq!(latest_stable_tag(&build).as_deref(), Some("2.0.0+build.1"));
    }

    #[test]
    fn sha256sums_binds_the_digest_to_the_exact_asset() {
        const NAME: &str = "IdleTrigger-x64.exe";
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        // BSD-style marker plus CRLF, uppercase normalized to lowercase.
        let sums = format!(
            "{hex} *IdleTrigger-x86.exe\r\n{} *{NAME}\r\n",
            hex.to_ascii_uppercase()
        );
        assert_eq!(parse_sha256sums(&sums, NAME).as_deref(), Some(hex));
        // Plain double-space form also parses.
        assert_eq!(
            parse_sha256sums(&format!("{hex}  {NAME}\n"), NAME).as_deref(),
            Some(hex)
        );
        // Wrong asset, malformed hex, no space: no match.
        assert_eq!(parse_sha256sums(&format!("{hex} *other.exe"), NAME), None);
        assert_eq!(parse_sha256sums(&format!("01 *{NAME}"), NAME), None);
        assert_eq!(parse_sha256sums(&format!("{hex}*{NAME}"), NAME), None);
        assert_eq!(parse_sha256sums("", NAME), None);
    }

    #[test]
    fn api_rejection_classifies_rate_limit_tiers() {
        assert!(matches!(
            classify_api_rejection(403, Some("0"), Some("1000120"), None, 1_000_000),
            ApiRejection::RateLimit { minutes: 2 }
        ));
        assert!(matches!(
            classify_api_rejection(403, Some("0"), Some("999999"), None, 1_000_000),
            ApiRejection::RateLimit { minutes: 0 }
        ));
        assert!(matches!(
            classify_api_rejection(429, Some("0"), None, None, 0),
            ApiRejection::RateLimit { minutes: 0 }
        ));
        assert!(matches!(
            classify_api_rejection(403, Some("42"), None, Some("60"), 0),
            ApiRejection::RetryAfter { minutes: 1 }
        ));
        assert!(matches!(
            classify_api_rejection(404, Some("42"), Some("1000120"), None, 0),
            ApiRejection::Status(404)
        ));
        assert!(matches!(
            classify_api_rejection(500, Some("0"), Some("1000120"), Some("60"), 0),
            ApiRejection::Status(500)
        ));
    }

    fn api_asset_json(digest: Option<String>, url: &str) -> Json {
        let mut asset = serde_json::json!({
            "name": ASSET_NAME,
            "state": "uploaded",
            "browser_download_url": url,
        });
        if let Some(digest) = digest {
            asset["digest"] = serde_json::json!(digest);
        }
        serde_json::json!({
            "tag_name": "v0.2.0",
            "draft": false,
            "prerelease": false,
            "assets": [asset],
        })
    }

    #[test]
    fn api_asset_requires_exact_tag_url_and_digest() {
        let url = format!("https://github.com/{APP_REPO}/releases/download/v0.2.0/{ASSET_NAME}");
        let (asset_url, digest) = parse_api_asset(
            &api_asset_json(Some(format!("sha256:{}", "ab".repeat(32))), &url),
            "0.2.0",
        )
        .unwrap();
        assert_eq!(asset_url, url);
        assert_eq!(digest, "ab".repeat(32));
        // Missing digest, version drift, draft state: rejected.
        assert!(parse_api_asset(&api_asset_json(None, &url), "0.2.0").is_err());
        assert!(
            parse_api_asset(
                &api_asset_json(Some(format!("sha256:{}", "ab".repeat(32))), &url),
                "0.3.0"
            )
            .is_err()
        );
        let mut draft = api_asset_json(Some(format!("sha256:{}", "ab".repeat(32))), &url);
        draft["draft"] = serde_json::json!(true);
        assert!(parse_api_asset(&draft, "0.2.0").is_err());
        // Unexpected host/path/query: rejected.
        assert!(
            parse_api_asset(
                &api_asset_json(
                    Some(format!("sha256:{}", "ab".repeat(32))),
                    &format!("https://objects.githubusercontent.com/x/{ASSET_NAME}")
                ),
                "0.2.0"
            )
            .is_err()
        );
        assert!(
            parse_api_asset(
                &api_asset_json(
                    Some(format!("sha256:{}", "ab".repeat(32))),
                    &format!(
                        "https://github.com/{APP_REPO}/releases/download/v0.2.0/{ASSET_NAME}?x=1"
                    )
                ),
                "0.2.0"
            )
            .is_err()
        );
    }

    #[test]
    fn api_asset_rejects_duplicate_windows_assets() {
        let url = format!("https://github.com/{APP_REPO}/releases/download/v0.2.0/{ASSET_NAME}");
        let mut json = api_asset_json(Some(format!("sha256:{}", "ab".repeat(32))), &url);
        let asset = json["assets"][0].clone();
        json["assets"].as_array_mut().unwrap().push(asset);
        assert!(parse_api_asset(&json, "0.2.0").is_err());
    }

    #[test]
    fn replacement_script_stages_and_atomically_replaces() {
        let script = replace_script(
            Path::new(r"C:\cache\IdleTrigger-x64.exe"),
            Path::new(r"C:\app\IdleTrigger.exe"),
            &"a".repeat(64),
        );
        assert!(script.contains("Copy-Item -LiteralPath $src -Destination $new"));
        assert!(script.contains("[System.IO.File]::Replace($new, $dst, $old, $true)"));
        assert!(script.contains("Get-FileHash -Algorithm SHA256"));
        assert!(script.contains("Start-Process -FilePath $dst"));
        assert!(script.contains(REPLACE_ERROR_LOG));
        assert!(!script.contains("Move-Item -LiteralPath $dst -Destination $old"));
    }

    #[test]
    fn cleanup_only_runs_with_a_matching_success_marker() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp = |tag: &str| {
            let dir = std::env::temp_dir().join(format!(
                "idletrigger-selfupdate-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };

        // No marker: the previous round never started, nothing is touched.
        let dir = temp("nomarker");
        let staged = dir.join(ASSET_NAME);
        std::fs::write(&staged, b"new").unwrap();
        assert!(!cleanup_applied_update_in(&dir, "1.2.3", None));
        assert!(staged.exists());
        let nomarker_dir = dir;

        // Marker mismatch (replacement failed or rolled back): keep staging,
        // backup and marker for the retry window.
        let dir = temp("mismatch");
        std::fs::write(dir.join(ASSET_NAME), b"new").unwrap();
        std::fs::write(dir.join(PENDING_APPLY_MARKER), "1.9.9").unwrap();
        let old = temp("mismatch-old").join("IdleTrigger.exe.old");
        std::fs::write(&old, b"old").unwrap();
        assert!(!cleanup_applied_update_in(&dir, "1.2.3", Some(&old)));
        assert!(dir.join(ASSET_NAME).exists());
        assert!(dir.join(PENDING_APPLY_MARKER).exists());
        assert!(old.exists());
        let mismatch_dir = dir;
        let mismatch_old = old.parent().unwrap().to_path_buf();

        // Matching marker (running == target): reclaim everything, idempotent.
        let dir = temp("match");
        std::fs::write(dir.join(ASSET_NAME), b"new").unwrap();
        std::fs::write(dir.join(REPLACE_SCRIPT), b"script").unwrap();
        std::fs::write(dir.join(PENDING_APPLY_MARKER), "1.2.3").unwrap();
        let old = temp("match-old").join("IdleTrigger.exe.old");
        std::fs::write(&old, b"old").unwrap();
        assert!(cleanup_applied_update_in(&dir, "1.2.3", Some(&old)));
        assert!(!dir.exists());
        assert!(!old.exists());
        assert!(!cleanup_applied_update_in(&dir, "1.2.3", Some(&old)));
        let match_old = old.parent().unwrap().to_path_buf();

        // A user's own file inside a folder named "update" must survive the
        // cleanup: only updater-written files are removed, and the folder is
        // dropped solely when that leaves it empty.
        let dir = temp("userfile");
        std::fs::write(dir.join(ASSET_NAME), b"new").unwrap();
        std::fs::write(dir.join(PENDING_APPLY_MARKER), "1.2.3").unwrap();
        let user_file = dir.join("my-notes.txt");
        std::fs::write(&user_file, b"keep me").unwrap();
        assert!(cleanup_applied_update_in(&dir, "1.2.3", None));
        assert!(!dir.join(ASSET_NAME).exists());
        assert!(!dir.join(PENDING_APPLY_MARKER).exists());
        assert!(user_file.exists());
        assert!(dir.exists());

        let _ = std::fs::remove_dir_all(&mismatch_dir);
        let _ = std::fs::remove_dir_all(&mismatch_old);
        let _ = std::fs::remove_dir_all(&match_old);
        let _ = std::fs::remove_dir_all(&nomarker_dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_error_log_is_drained_once() {
        let dir = std::env::temp_dir().join(format!(
            "idletrigger-selfupdate-error-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(REPLACE_ERROR_LOG),
            "\u{FEFF}replacement failed boom\r\n",
        )
        .unwrap();
        assert_eq!(
            drain_replace_error(&dir).as_deref(),
            Some("replacement failed boom")
        );
        assert_eq!(drain_replace_error(&dir), None);
        assert!(!dir.join(REPLACE_ERROR_LOG).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
