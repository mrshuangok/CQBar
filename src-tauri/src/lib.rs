use std::os::windows::process::CommandExt;
use std::{
    fs, io,
    net::ToSocketAddrs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{TimeZone, Utc};
use reqwest::header::{
    HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, ORIGIN, REFERER, USER_AGENT,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    menu::{CheckMenuItem, MenuItem, Submenu},
    tray::TrayIconBuilder,
    AppHandle, LogicalSize, Manager, PhysicalPosition, Window, WindowEvent,
};
use windows::{
    core::PCWSTR,
    Win32::{
        System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ},
        UI::Shell::SHRegGetValueW,
    },
};

mod diagnostics;
mod native_ui;

const AUTH_FILE_RELATIVE: &str = ".codex/auth.json";
const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const USAGE_HOST: &str = "chatgpt.com";
const TRAY_ALWAYS_ON_TOP_ID: &str = "tray-always-on-top";
const TRAY_PRESENTATION_ID: &str = "tray-presentation";
const TRAY_SETTINGS_ID: &str = "tray-settings";
const TRAY_CLOSE_ID: &str = "tray-close";
const TRAY_LANGUAGE_ZH_ID: &str = "tray-language-zh";
const TRAY_LANGUAGE_EN_ID: &str = "tray-language-en";
const REQUEST_TIMEOUT_SECONDS: u64 = 12;
const REQUEST_RETRY_ATTEMPTS: usize = 3;
const CHATGPT_ACCOUNT_ID_HEADER: &str = "chatgpt-account-id";
const CHATGPT_ORIGIN: &str = "https://chatgpt.com";
const CHATGPT_REFERER: &str = "https://chatgpt.com/";
const APP_USER_AGENT: &str = "CodexQuotaFloat/0.1";
const WIDGET_CONTENT_WIDTH: f64 = 190.0;
const WIDGET_CONTENT_SQUARE_HEIGHT: f64 = 190.0;
const WIDGET_CONTENT_COMPACT_HEIGHT: f64 = 40.0;
pub(crate) const WINDOW_EFFECT_PADDING: f32 = 10.0;
const SQUARE_WIDTH: f64 = WIDGET_CONTENT_WIDTH + WINDOW_EFFECT_PADDING as f64 * 2.0;
const SQUARE_HEIGHT: f64 = WIDGET_CONTENT_SQUARE_HEIGHT + WINDOW_EFFECT_PADDING as f64 * 2.0;
const COMPACT_HEIGHT: f64 = WIDGET_CONTENT_COMPACT_HEIGHT + WINDOW_EFFECT_PADDING as f64 * 2.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowPosition {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    auto_refresh_enabled: bool,
    auto_refresh_minutes: u32,
    window_position: Option<WindowPosition>,
    always_on_top: bool,
    display_mode: DisplayMode,
    #[serde(default)]
    compact_display: CompactDisplaySettings,
    theme_mode: ThemeMode,
    #[serde(default)]
    shape_mode: ShapeMode,
    #[serde(default)]
    presentation_mode: PresentationMode,
    #[serde(default)]
    language: Option<Language>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Language {
    Zh,
    En,
}

impl Language {
    fn text(self, zh: &'static str, en: &'static str) -> &'static str {
        match self {
            Self::Zh => zh,
            Self::En => en,
        }
    }
}

impl AppSettings {
    fn effective_language(&self) -> Language {
        self.language.unwrap_or_else(system_language)
    }
}

fn language_for_ui_lang_id(lang_id: u16) -> Language {
    if lang_id & 0x03ff == 0x0004 {
        Language::Zh
    } else {
        Language::En
    }
}

fn system_language() -> Language {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
    }
    language_for_ui_lang_id(unsafe { GetUserDefaultUILanguage() })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum DisplayMode {
    Remaining,
    Used,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompactDisplaySettings {
    #[serde(default)]
    five_hour_time: bool,
    #[serde(default)]
    weekly_date: bool,
}

impl Default for CompactDisplaySettings {
    fn default() -> Self {
        Self {
            five_hour_time: false,
            weekly_date: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ThemeMode {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum ShapeMode {
    Square,
    Compact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PresentationMode {
    Floating,
    Taskbar,
}

impl Default for PresentationMode {
    fn default() -> Self {
        Self::Floating
    }
}

impl Default for ShapeMode {
    fn default() -> Self {
        Self::Square
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            auto_refresh_enabled: true,
            auto_refresh_minutes: 5,
            window_position: None,
            always_on_top: true,
            display_mode: DisplayMode::Remaining,
            compact_display: CompactDisplaySettings::default(),
            theme_mode: ThemeMode::Dark,
            shape_mode: ShapeMode::Square,
            presentation_mode: PresentationMode::Floating,
            language: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettingsInput {
    auto_refresh_enabled: bool,
    auto_refresh_minutes: u32,
    window_position: Option<WindowPosition>,
    always_on_top: bool,
    display_mode: DisplayMode,
    compact_display: CompactDisplaySettings,
    theme_mode: ThemeMode,
    shape_mode: ShapeMode,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum UsageStatus {
    Ok,
    NotLoggedIn,
    TokenMissing,
    RequestFailed,
    UnsupportedResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageWindowSnapshot {
    used_percent: u8,
    remaining_percent: u8,
    reset_at: Option<String>,
    reset_after_seconds: Option<u64>,
    window_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageSnapshot {
    account_label: Option<String>,
    plan_type: Option<String>,
    fetched_at: String,
    status: UsageStatus,
    error_message: Option<String>,
    five_hour_usage: Option<UsageWindowSnapshot>,
    weekly_usage: Option<UsageWindowSnapshot>,
}

#[derive(Debug, Deserialize)]
struct AuthFile {
    tokens: Option<AuthTokens>,
}

#[derive(Debug, Deserialize)]
struct AuthTokens {
    access_token: Option<String>,
    id_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsageApiResponse {
    email: Option<String>,
    plan_type: Option<String>,
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
struct RateLimit {
    primary_window: Option<UsageWindow>,
    secondary_window: Option<UsageWindow>,
}

#[derive(Debug, Deserialize)]
struct UsageWindow {
    used_percent: Option<u8>,
    limit_window_seconds: Option<u64>,
    reset_after_seconds: Option<u64>,
    reset_at: Option<i64>,
}

struct AppState {
    settings: Mutex<AppSettings>,
    always_on_top_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    presentation_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    settings_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    close_item: Mutex<Option<MenuItem<tauri::Wry>>>,
    language_menu: Mutex<Option<Submenu<tauri::Wry>>>,
    language_zh_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    language_en_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
    snapshot: Mutex<Option<UsageSnapshot>>,
    last_good_snapshot: Mutex<Option<UsageSnapshot>>,
    last_refresh: Mutex<Option<Instant>>,
    refresh_gate: tokio::sync::Mutex<()>,
    refreshing: AtomicBool,
    native_ui: Mutex<Option<native_ui::NativeUiController>>,
}

#[tauri::command]
fn get_app_settings(state: tauri::State<'_, AppState>) -> Result<AppSettings, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|_| "failed to read app settings".to_string())
}

#[tauri::command]
fn update_app_settings(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    input: AppSettingsInput,
) -> Result<AppSettings, String> {
    diagnostics::event("INFO", "settings.update.start", "source=command");
    let mut guard = state
        .settings
        .lock()
        .map_err(|_| "failed to update app settings".to_string())?;

    let updated = sanitize_settings(AppSettings {
        auto_refresh_enabled: input.auto_refresh_enabled,
        auto_refresh_minutes: input.auto_refresh_minutes,
        window_position: input.window_position,
        always_on_top: input.always_on_top,
        display_mode: input.display_mode,
        compact_display: input.compact_display,
        theme_mode: input.theme_mode,
        shape_mode: input.shape_mode,
        presentation_mode: guard.presentation_mode,
        language: guard.language,
    });

    if let Some(window) = app.get_window("main") {
        if updated.presentation_mode == PresentationMode::Floating
            && updated.always_on_top != guard.always_on_top
        {
            window
                .set_always_on_top(updated.always_on_top)
                .map_err(|error| error.to_string())?;
        }
    }
    if let Err(error) = persist_settings(&updated) {
        if let Some(window) = app.get_window("main") {
            if updated.presentation_mode == PresentationMode::Floating
                && updated.always_on_top != guard.always_on_top
            {
                let _ = window.set_always_on_top(guard.always_on_top);
            }
        }
        return Err(error);
    }
    *guard = updated.clone();
    drop(guard);
    sync_tray_always_on_top(&state, updated.always_on_top);
    let _ = write_taskbar_state(&state);
    diagnostics::event(
        "INFO",
        "settings.update.finish",
        &format!(
            "shape={:?} presentation={:?} always_on_top={} auto_refresh={} interval_min={}",
            updated.shape_mode,
            updated.presentation_mode,
            updated.always_on_top,
            updated.auto_refresh_enabled,
            updated.auto_refresh_minutes
        ),
    );
    Ok(updated)
}

#[tauri::command]
async fn get_usage_snapshot(app: AppHandle) -> Result<UsageSnapshot, String> {
    refresh_and_publish(&app).await
}

fn sanitize_settings(mut settings: AppSettings) -> AppSettings {
    settings.auto_refresh_minutes = settings.auto_refresh_minutes.clamp(1, 120);
    settings
}

fn settings_file_path() -> Result<PathBuf, String> {
    let base_dir = dirs::data_local_dir().ok_or("local app data directory not found")?;
    Ok(base_dir.join("codex-quota-float").join("settings.json"))
}

fn load_settings() -> AppSettings {
    let path = match settings_file_path() {
        Ok(path) => path,
        Err(error) => {
            diagnostics::event("WARN", "settings.path_unavailable", &error);
            return AppSettings::default();
        }
    };

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) => {
            diagnostics::event(
                if error.kind() == io::ErrorKind::NotFound {
                    "INFO"
                } else {
                    "WARN"
                },
                "settings.read_failed",
                &format!("kind={:?}", error.kind()),
            );
            return AppSettings::default();
        }
    };

    match serde_json::from_str::<AppSettings>(&content) {
        Ok(settings) => sanitize_settings(settings),
        Err(error) => {
            diagnostics::event(
                "WARN",
                "settings.parse_failed",
                &format!("category={:?}", error.classify()),
            );
            AppSettings::default()
        }
    }
}

fn persist_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let payload = serde_json::to_string_pretty(settings).map_err(|error| error.to_string())?;
    fs::write(path, payload).map_err(|error| error.to_string())
}

fn taskbar_state_file_path() -> Result<PathBuf, String> {
    Ok(settings_file_path()?.with_file_name("taskbar-state.ini"))
}

fn reset_timestamp(window: Option<&UsageWindowSnapshot>) -> i64 {
    window
        .and_then(|usage| usage.reset_at.as_deref())
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp())
        .unwrap_or(0)
}

fn write_taskbar_state(state: &AppState) -> Result<(), String> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| "failed to read taskbar settings".to_string())?
        .clone();
    let snapshot = state
        .snapshot
        .lock()
        .map_err(|_| "failed to read taskbar snapshot".to_string())?
        .clone();
    let last_good = state
        .last_good_snapshot
        .lock()
        .map_err(|_| "failed to read cached taskbar snapshot".to_string())?
        .clone();
    let cached = matches!(
        snapshot.as_ref().map(|item| &item.status),
        Some(UsageStatus::RequestFailed)
    );
    let display = if cached {
        last_good.as_ref()
    } else {
        snapshot.as_ref()
    };
    let five = display.and_then(|item| item.five_hour_usage.as_ref());
    let week = display.and_then(|item| item.weekly_usage.as_ref());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let contents = format!(
        "[quota]\r\npid={}\r\nheartbeat={now}\r\nhasData={}\r\ncached={}\r\nusedMode={}\r\nlanguage={}\r\nfiveUsed={}\r\nfiveRemaining={}\r\nfiveReset={}\r\nweekUsed={}\r\nweekRemaining={}\r\nweekReset={}\r\n",
        std::process::id(),
        u8::from(display.is_some()),
        u8::from(cached && display.is_some()),
        u8::from(matches!(settings.display_mode, DisplayMode::Used)),
        if settings.effective_language() == Language::Zh { 0 } else { 1 },
        five.map_or(-1, |item| i32::from(item.used_percent)),
        five.map_or(-1, |item| i32::from(item.remaining_percent)),
        reset_timestamp(five),
        week.map_or(-1, |item| i32::from(item.used_percent)),
        week.map_or(-1, |item| i32::from(item.remaining_percent)),
        reset_timestamp(week),
    );
    let path = taskbar_state_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, contents).map_err(|error| error.to_string())
}

async fn refresh_and_publish(app: &AppHandle) -> Result<UsageSnapshot, String> {
    let state = app.state::<AppState>();
    let waiting = Instant::now();
    diagnostics::event("INFO", "usage.refresh.wait", "");
    let _gate = state.refresh_gate.lock().await;
    let started = Instant::now();
    diagnostics::event(
        "INFO",
        "usage.refresh.start",
        &format!("wait_ms={}", waiting.elapsed().as_millis()),
    );
    state.refreshing.store(true, Ordering::Relaxed);
    native_ui::request_redraw(app);
    let snapshot = fetch_usage_snapshot().await;
    if matches!(snapshot.status, UsageStatus::Ok) {
        *state
            .last_good_snapshot
            .lock()
            .map_err(|_| "failed to store cached usage".to_string())? = Some(snapshot.clone());
    }
    *state
        .snapshot
        .lock()
        .map_err(|_| "failed to store usage".to_string())? = Some(snapshot.clone());
    *state
        .last_refresh
        .lock()
        .map_err(|_| "failed to store refresh time".to_string())? = Some(Instant::now());
    if let Err(error) = write_taskbar_state(&state) {
        diagnostics::event("ERROR", "taskbar.state.write_failed", &error);
    }
    state.refreshing.store(false, Ordering::Relaxed);
    native_ui::request_redraw(app);
    diagnostics::event(
        if matches!(snapshot.status, UsageStatus::Ok) {
            "INFO"
        } else {
            "WARN"
        },
        "usage.refresh.finish",
        &format!(
            "status={:?} elapsed_ms={}",
            snapshot.status,
            started.elapsed().as_millis()
        ),
    );
    Ok(snapshot)
}

fn resolve_ipv4_usage_addr() -> Option<std::net::SocketAddr> {
    (USAGE_HOST, 443)
        .to_socket_addrs()
        .ok()?
        .find(std::net::SocketAddr::is_ipv4)
}

fn auth_file_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(AUTH_FILE_RELATIVE))
}

fn decode_email_from_id_token(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: Value = serde_json::from_slice(&decoded).ok()?;
    json.get("email")?.as_str().map(ToString::to_string)
}

fn fallback_account_label(tokens: &AuthTokens) -> Option<String> {
    tokens
        .id_token
        .as_deref()
        .and_then(decode_email_from_id_token)
        .or_else(|| tokens.account_id.clone())
}

fn empty_snapshot(
    status: UsageStatus,
    account_label: Option<String>,
    error_message: Option<String>,
) -> UsageSnapshot {
    UsageSnapshot {
        account_label,
        plan_type: None,
        fetched_at: Utc::now().to_rfc3339(),
        status,
        error_message,
        five_hour_usage: None,
        weekly_usage: None,
    }
}

fn map_window(window: Option<UsageWindow>) -> Option<UsageWindowSnapshot> {
    let window = window?;
    let used_percent = window.used_percent?;
    let remaining_percent = 100_u8.saturating_sub(used_percent.min(100));
    let reset_at = window
        .reset_at
        .and_then(|timestamp| Utc.timestamp_opt(timestamp, 0).single())
        .map(|date| date.to_rfc3339());

    Some(UsageWindowSnapshot {
        used_percent: used_percent.min(100),
        remaining_percent,
        reset_at,
        reset_after_seconds: window.reset_after_seconds,
        window_seconds: window.limit_window_seconds,
    })
}

async fn fetch_usage_snapshot() -> UsageSnapshot {
    let auth_path = match auth_file_path() {
        Some(path) => path,
        None => {
            diagnostics::event("WARN", "auth.home_unavailable", "");
            return empty_snapshot(
                UsageStatus::NotLoggedIn,
                None,
                Some("未找到当前系统用户目录，无法读取 .codex/auth.json".to_string()),
            );
        }
    };

    let auth_content = match fs::read_to_string(&auth_path) {
        Ok(content) => content,
        Err(error) => {
            diagnostics::event(
                "WARN",
                "auth.read_failed",
                &format!("kind={:?}", error.kind()),
            );
            return empty_snapshot(
                UsageStatus::NotLoggedIn,
                None,
                Some(format!("未找到登录文件：{}", auth_path.display())),
            );
        }
    };

    let auth: AuthFile = match serde_json::from_str(&auth_content) {
        Ok(auth) => auth,
        Err(error) => {
            diagnostics::event(
                "WARN",
                "auth.parse_failed",
                &format!("category={:?}", error.classify()),
            );
            return empty_snapshot(
                UsageStatus::UnsupportedResponse,
                None,
                Some(format!("auth.json 解析失败：{error}")),
            );
        }
    };

    let tokens = match auth.tokens {
        Some(tokens) => tokens,
        None => {
            diagnostics::event("WARN", "auth.tokens_missing", "");
            return empty_snapshot(
                UsageStatus::TokenMissing,
                None,
                Some("auth.json 中未找到 tokens 字段".to_string()),
            );
        }
    };

    let account_label = fallback_account_label(&tokens);
    let account_id = tokens.account_id.clone();
    let access_token = match tokens.access_token {
        Some(token) if !token.trim().is_empty() => token,
        _ => {
            diagnostics::event("WARN", "auth.access_token_missing", "");
            return empty_snapshot(
                UsageStatus::TokenMissing,
                account_label,
                Some("auth.json 中未找到 access_token".to_string()),
            );
        }
    };

    let mut client_builder =
        reqwest::Client::builder().timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS));
    if let Some(ipv4_addr) = resolve_ipv4_usage_addr() {
        client_builder = client_builder.resolve(USAGE_HOST, ipv4_addr);
    }

    let client = match client_builder.build() {
        Ok(client) => client,
        Err(error) => {
            diagnostics::event(
                "ERROR",
                "usage.client_build_failed",
                &format!("timeout={}", error.is_timeout()),
            );
            return empty_snapshot(
                UsageStatus::RequestFailed,
                account_label,
                Some(format!("额度请求客户端创建失败：{error}")),
            );
        }
    };

    let mut last_request_error = None;
    let mut response = None;
    let account_id_header = HeaderName::from_static(CHATGPT_ACCOUNT_ID_HEADER);

    for attempt in 0..REQUEST_RETRY_ATTEMPTS {
        diagnostics::event(
            "INFO",
            "usage.request.start",
            &format!("attempt={}", attempt + 1),
        );
        let mut request = client
            .get(USAGE_URL)
            .header(AUTHORIZATION, format!("Bearer {access_token}"))
            .header(ACCEPT, "application/json")
            .header(ORIGIN, CHATGPT_ORIGIN)
            .header(REFERER, CHATGPT_REFERER)
            .header(USER_AGENT, APP_USER_AGENT);

        if let Some(account_id) = account_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            if let Ok(value) = HeaderValue::from_str(account_id) {
                request = request.header(account_id_header.clone(), value);
            }
        }

        match request.send().await {
            Ok(next_response) => {
                diagnostics::event(
                    "INFO",
                    "usage.request.response",
                    &format!(
                        "attempt={} http_status={}",
                        attempt + 1,
                        next_response.status().as_u16()
                    ),
                );
                response = Some(next_response);
                break;
            }
            Err(error) => {
                diagnostics::event(
                    "WARN",
                    "usage.request.failed",
                    &format!(
                        "attempt={} timeout={} connect={}",
                        attempt + 1,
                        error.is_timeout(),
                        error.is_connect()
                    ),
                );
                last_request_error = Some(error.to_string());
            }
        }
    }

    let response = match response {
        Some(response) => response,
        None => {
            let detail = last_request_error.unwrap_or_else(|| "unknown error".to_string());
            return empty_snapshot(
                UsageStatus::RequestFailed,
                account_label,
                Some(format!(
                    "额度接口请求失败：{detail}（已重试 {REQUEST_RETRY_ATTEMPTS} 次）"
                )),
            );
        }
    };

    let status_code = response.status();
    if !status_code.is_success() {
        return empty_snapshot(
            UsageStatus::RequestFailed,
            account_label,
            Some(format!("额度接口返回异常状态：{status_code}")),
        );
    }

    let api: UsageApiResponse = match response.json().await {
        Ok(data) => data,
        Err(error) => {
            diagnostics::event(
                "WARN",
                "usage.response_parse_failed",
                &format!("decode={}", error.is_decode()),
            );
            return empty_snapshot(
                UsageStatus::UnsupportedResponse,
                account_label,
                Some(format!("额度接口返回解析失败：{error}")),
            );
        }
    };

    let rate_limit = match api.rate_limit {
        Some(rate_limit) => rate_limit,
        None => {
            diagnostics::event("WARN", "usage.rate_limit_missing", "");
            return empty_snapshot(
                UsageStatus::UnsupportedResponse,
                api.email.or(account_label),
                Some("额度接口未返回 rate_limit 字段".to_string()),
            );
        }
    };

    UsageSnapshot {
        account_label: api.email.or(account_label),
        plan_type: api.plan_type,
        fetched_at: Utc::now().to_rfc3339(),
        status: UsageStatus::Ok,
        error_message: None,
        five_hour_usage: map_window(rate_limit.primary_window),
        weekly_usage: map_window(rate_limit.secondary_window),
    }
}

fn apply_saved_window_position(window: &Window, settings: &AppSettings) {
    if let Some(position) = &settings.window_position {
        let padding = window_effect_padding_physical(window);
        let origin = window_origin_for_content_anchor(
            PhysicalPosition::new(position.x, position.y),
            padding,
        );
        let _ = window.set_position(origin);
    }
}

fn window_effect_padding_physical(window: &Window) -> i32 {
    (WINDOW_EFFECT_PADDING as f64 * window.scale_factor().unwrap_or(1.0)).round() as i32
}

fn window_origin_for_content_anchor(
    position: PhysicalPosition<i32>,
    padding: i32,
) -> PhysicalPosition<i32> {
    PhysicalPosition::new(
        position.x.saturating_sub(padding),
        position.y.saturating_sub(padding),
    )
}

fn content_anchor_for_window_origin(
    position: PhysicalPosition<i32>,
    padding: i32,
) -> PhysicalPosition<i32> {
    PhysicalPosition::new(
        position.x.saturating_add(padding),
        position.y.saturating_add(padding),
    )
}

fn apply_window_size(window: &tauri::Window, width: f64, height: f64) {
    let _ = window.set_min_size(None::<LogicalSize<f64>>);
    let _ = window.set_max_size(None::<LogicalSize<f64>>);
    let _ = window.set_size(LogicalSize::new(width, height));
    let _ = window.set_min_size(Some(LogicalSize::new(width, height)));
    let _ = window.set_max_size(Some(LogicalSize::new(width, height)));
}

fn apply_base_shape_size(window: &Window, shape_mode: &ShapeMode) {
    let height = match shape_mode {
        ShapeMode::Square => SQUARE_HEIGHT,
        ShapeMode::Compact => COMPACT_HEIGHT,
    };

    let _ = window.set_min_size(None::<LogicalSize<f64>>);
    let _ = window.set_max_size(None::<LogicalSize<f64>>);
    let _ = window.set_size(LogicalSize::new(SQUARE_WIDTH, height));
    let _ = window.set_min_size(Some(LogicalSize::new(SQUARE_WIDTH, height)));
    let _ = window.set_max_size(Some(LogicalSize::new(SQUARE_WIDTH, height)));
}

fn apply_runtime_window_shape(
    window: &tauri::Window,
    shape_mode: &ShapeMode,
    expanded_for_panel: bool,
) {
    let height = if matches!(shape_mode, ShapeMode::Compact) && expanded_for_panel {
        SQUARE_HEIGHT
    } else {
        match shape_mode {
            ShapeMode::Square => SQUARE_HEIGHT,
            ShapeMode::Compact => COMPACT_HEIGHT,
        }
    };

    apply_window_size(window, SQUARE_WIDTH, height);
}

fn persist_window_position(window: &tauri::Window, position: PhysicalPosition<i32>) {
    let state = window.state::<AppState>();
    let mut guard = match state.settings.lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    if guard.presentation_mode != PresentationMode::Floating {
        return;
    }

    let padding = window_effect_padding_physical(window);
    let anchor = content_anchor_for_window_origin(position, padding);
    guard.window_position = Some(WindowPosition {
        x: anchor.x,
        y: anchor.y,
    });

    if let Err(error) = persist_settings(&guard.clone()) {
        diagnostics::event("ERROR", "settings.position_save_failed", &error);
    }
}

fn sync_tray_always_on_top(state: &AppState, checked: bool) {
    let item = state
        .always_on_top_item
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned());

    if let Some(item) = item {
        let _ = item.set_checked(checked);
    }
}

fn sync_tray_presentation(state: &AppState, mode: PresentationMode, language: Language) {
    if let Some(item) = state
        .presentation_item
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
    {
        let text = if mode == PresentationMode::Floating {
            language.text("显示到任务栏", "Show in taskbar")
        } else {
            language.text("显示悬浮窗", "Show floating window")
        };
        let _ = item.set_text(text);
        let _ = item.set_enabled(true);
    }
    if let Some(item) = state
        .always_on_top_item
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
    {
        let _ = item.set_enabled(mode == PresentationMode::Floating);
    }
}

fn change_language(app: &AppHandle, language: Language) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current = state
        .settings
        .lock()
        .map_err(|_| "failed to read settings".to_string())?
        .clone();
    let mut updated = current.clone();
    updated.language = Some(language);
    persist_settings(&updated)?;
    *state
        .settings
        .lock()
        .map_err(|_| "failed to update settings".to_string())? = updated;
    sync_tray_presentation(&state, current.presentation_mode, language);
    if let Some(item) = state.always_on_top_item.lock().ok().and_then(|v| v.clone()) {
        let _ = item.set_text(language.text("置顶", "Always on top"));
    }
    if let Some(item) = state.settings_item.lock().ok().and_then(|v| v.clone()) {
        let _ = item.set_text(language.text("设置", "Settings"));
    }
    if let Some(item) = state.close_item.lock().ok().and_then(|v| v.clone()) {
        let _ = item.set_text(language.text("退出", "Exit"));
    }
    if let Some(item) = state.language_menu.lock().ok().and_then(|v| v.clone()) {
        let _ = item.set_text(language.text("语言", "Language"));
    }
    if let Some(item) = state.language_zh_item.lock().ok().and_then(|v| v.clone()) {
        let _ = item.set_checked(language == Language::Zh);
    }
    if let Some(item) = state.language_en_item.lock().ok().and_then(|v| v.clone()) {
        let _ = item.set_checked(language == Language::En);
    }
    if let Some(tray) = app.tray_by_id("main-tray") {
        let _ = tray.set_tooltip(Some("CQBar"));
    }
    if let Some(window) = app.get_window("main") {
        let _ = window.set_title("CQBar");
    }
    if let Err(error) = write_taskbar_state(&state) {
        diagnostics::event("ERROR", "taskbar.state.write_failed", &error);
    }
    if current.presentation_mode == PresentationMode::Taskbar {
        let _ = deskband_action("refresh-data");
    }
    native_ui::request_redraw(app);
    diagnostics::event(
        "INFO",
        "settings.language_changed",
        &format!("language={language:?}"),
    );
    Ok(())
}

fn registered_installation_directory() -> Option<PathBuf> {
    registered_installation_directory_for("CQBar")
        .or_else(|| registered_installation_directory_for("QuotaPeek"))
        .or_else(|| registered_installation_directory_for("Codex Quota Float"))
}

fn registered_installation_directory_for(product_name: &str) -> Option<PathBuf> {
    let key: Vec<u16> =
        format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product_name}\0")
            .encode_utf16()
            .collect();
    let value_name: Vec<u16> = "InstallLocation\0".encode_utf16().collect();
    let flags = RRF_RT_REG_SZ.0 as i32;
    let mut byte_count = 0_u32;
    let status = unsafe {
        SHRegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            flags,
            None,
            None,
            Some(&mut byte_count),
        )
    };
    if status.0 != 0 || byte_count < 2 {
        return None;
    }

    let mut value = vec![0_u16; (byte_count as usize + 1) / 2];
    let mut capacity = byte_count;
    let status = unsafe {
        SHRegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            flags,
            None,
            Some(value.as_mut_ptr().cast()),
            Some(&mut capacity),
        )
    };
    if status.0 != 0 {
        return None;
    }

    let available_units = (capacity as usize / 2).min(value.len());
    let end = value[..available_units]
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(available_units);
    let directory = String::from_utf16_lossy(&value[..end]);
    let directory = directory.trim().trim_matches('"');
    (!directory.is_empty()).then(|| PathBuf::from(directory))
}

fn find_deskband_controller(
    executable: &Path,
    installed_directory: Option<&Path>,
) -> Option<PathBuf> {
    let mut candidates = Vec::with_capacity(4);
    if let Some(directory) = executable.parent() {
        candidates.push(directory.join("deskbandctl.exe"));
        candidates.push(directory.join("resources").join("deskbandctl.exe"));
    }
    if let Some(directory) = installed_directory {
        candidates.push(directory.join("deskbandctl.exe"));
        candidates.push(directory.join("resources").join("deskbandctl.exe"));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn deskband_controller_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    if let Some(controller) = find_deskband_controller(&exe, None) {
        return Ok(controller);
    }
    if let Some(directory) = registered_installation_directory() {
        if let Some(controller) = find_deskband_controller(&exe, Some(&directory)) {
            return Ok(controller);
        }
    }
    Err("任务栏组件未安装，请使用安装包安装".to_string())
}

fn deskband_action(action: &str) -> Result<bool, String> {
    let started = Instant::now();
    diagnostics::event("INFO", "taskbar.action.start", &format!("action={action}"));
    let controller = deskband_controller_path().map_err(|error| {
        diagnostics::event("ERROR", "taskbar.controller_missing", &error);
        error
    })?;
    let output = Command::new(controller)
        .arg(action)
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|error| {
            diagnostics::event(
                "ERROR",
                "taskbar.controller_launch_failed",
                &error.to_string(),
            );
            error.to_string()
        })?;
    diagnostics::event(
        if output.status.success() {
            "INFO"
        } else {
            "WARN"
        },
        "taskbar.action.finish",
        &format!(
            "action={action} exit_code={:?} elapsed_ms={} stderr={}",
            output.status.code(),
            started.elapsed().as_millis(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    );
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(5) => Err("任务栏内嵌功能仅支持 Windows 10 x64".to_string()),
        _ => Err(format!(
            "任务栏组件操作失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
    }
}

fn show_taskbar() -> Result<bool, String> {
    deskband_action("show")
}

fn notify_native_error(message: &str) {
    diagnostics::event("ERROR", "ui.error_dialog", "shown=true");
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(window: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let caption: Vec<u16> = "CQBar".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(0, text.as_ptr(), caption.as_ptr(), 0x10);
    }
}

fn switch_presentation(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current = state
        .settings
        .lock()
        .map_err(|_| "failed to read presentation mode".to_string())?
        .clone();
    let window = app.get_window("main").ok_or("main window not available")?;
    let target = if current.presentation_mode == PresentationMode::Floating {
        PresentationMode::Taskbar
    } else {
        PresentationMode::Floating
    };
    diagnostics::event(
        "INFO",
        "presentation.switch.start",
        &format!("from={:?} to={:?}", current.presentation_mode, target),
    );

    if target == PresentationMode::Taskbar {
        if !deskband_action("supported")? {
            return Err("任务栏内嵌功能仅支持 Windows 10 x64".to_string());
        }
        if !show_taskbar()? {
            return Err("Windows 未能显示任务栏工具栏".to_string());
        }
        if let Err(error) = window.hide() {
            let _ = deskband_action("hide");
            return Err(error.to_string());
        }
    } else {
        window.show().map_err(|error| error.to_string())?;
        if let Err(error) = window.set_skip_taskbar(true) {
            let _ = window.hide();
            return Err(error.to_string());
        }
        if let Err(error) = window.set_always_on_top(current.always_on_top) {
            let _ = window.hide();
            return Err(error.to_string());
        }
        if let Err(error) = native_ui::restore_window_style(&window) {
            let _ = window.hide();
            return Err(error);
        }
        native_ui::request_redraw(app);
        if !deskband_action("hide")? {
            let _ = window.hide();
            return Err("Windows 未能隐藏任务栏工具栏".to_string());
        }
    }

    let mut updated = current.clone();
    updated.presentation_mode = target;
    if let Err(error) = persist_settings(&updated) {
        if target == PresentationMode::Taskbar {
            let _ = window.show();
            let _ = deskband_action("hide");
        } else {
            let _ = deskband_action("show");
            let _ = window.hide();
        }
        return Err(error);
    }
    *state
        .settings
        .lock()
        .map_err(|_| "failed to save presentation mode".to_string())? = updated;
    sync_tray_presentation(&state, target, current.effective_language());
    diagnostics::event(
        "INFO",
        "presentation.switch.finish",
        &format!("mode={target:?}"),
    );
    Ok(())
}

fn restore_floating_after_taskbar_failure(app: &AppHandle, reason: &str) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current = state
        .settings
        .lock()
        .map_err(|_| "failed to read presentation mode".to_string())?
        .clone();
    if current.presentation_mode != PresentationMode::Taskbar {
        return Ok(());
    }
    let window = app.get_window("main").ok_or("main window not available")?;
    window.show().map_err(|error| error.to_string())?;
    window
        .set_skip_taskbar(true)
        .map_err(|error| error.to_string())?;
    window
        .set_always_on_top(current.always_on_top)
        .map_err(|error| error.to_string())?;
    native_ui::restore_window_style(&window)?;
    native_ui::request_redraw(app);
    let _ = deskband_action("hide");
    let mut updated = current;
    updated.presentation_mode = PresentationMode::Floating;
    persist_settings(&updated)?;
    let language = updated.effective_language();
    *state
        .settings
        .lock()
        .map_err(|_| "failed to restore floating mode".to_string())? = updated;
    sync_tray_presentation(&state, PresentationMode::Floating, language);
    diagnostics::event("ERROR", "taskbar.fallback_reason", reason);
    notify_native_error(language.text(
        "任务栏工具栏不可用，已恢复悬浮窗。",
        "Taskbar toolbar unavailable; the floating window was restored.",
    ));
    Ok(())
}

async fn background_refresh_loop(app: AppHandle) {
    diagnostics::event("INFO", "background.loop.start", "");
    tokio::time::sleep(Duration::from_secs(15)).await;
    let mut last_heartbeat_log = Instant::now();
    loop {
        if last_heartbeat_log.elapsed() >= Duration::from_secs(300) {
            diagnostics::event("INFO", "background.loop.heartbeat", "");
            last_heartbeat_log = Instant::now();
        }
        let state = app.state::<AppState>();
        let settings = state.settings.lock().ok().map(|guard| guard.clone());
        let last_refresh = state.last_refresh.lock().ok().and_then(|guard| *guard);
        if let Some(settings) = settings {
            let due = last_refresh.map_or(true, |instant| {
                instant.elapsed()
                    >= Duration::from_secs(u64::from(settings.auto_refresh_minutes) * 60)
            });
            if settings.auto_refresh_enabled && due {
                if let Err(error) = refresh_and_publish(&app).await {
                    diagnostics::event("ERROR", "usage.refresh.publish_failed", &error);
                }
            } else if let Err(error) = write_taskbar_state(&state) {
                diagnostics::event("ERROR", "taskbar.state.write_failed", &error);
            }
            if settings.presentation_mode == PresentationMode::Taskbar {
                if !matches!(deskband_action("is-shown"), Ok(true)) {
                    match show_taskbar() {
                        Ok(true) => {}
                        other => {
                            let reason = match other {
                                Ok(false) => "Windows 未能显示任务栏工具栏".to_string(),
                                Err(error) => error,
                                Ok(true) => unreachable!(),
                            };
                            diagnostics::event("ERROR", "taskbar.restore_failed", &reason);
                            if let Err(error) =
                                restore_floating_after_taskbar_failure(&app, &reason)
                            {
                                diagnostics::event(
                                    "ERROR",
                                    "presentation.floating_restore_failed",
                                    &error,
                                );
                            }
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}

fn apply_tray_always_on_top(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let item = state
        .always_on_top_item
        .lock()
        .map_err(|_| "failed to read tray state".to_string())?
        .as_ref()
        .cloned()
        .ok_or_else(|| "always-on-top tray item not available".to_string())?;
    let checked = item.is_checked().map_err(|error| error.to_string())?;
    {
        let mut settings = state
            .settings
            .lock()
            .map_err(|_| "failed to update app settings".to_string())?;
        if settings.presentation_mode != PresentationMode::Floating {
            let _ = item.set_checked(settings.always_on_top);
            return Ok(());
        }
        let previous = settings.always_on_top;
        let mut updated = settings.clone();
        updated.always_on_top = checked;
        let window = app.get_window("main").ok_or("main window not available")?;
        if let Err(error) = window.set_always_on_top(checked) {
            let _ = item.set_checked(previous);
            return Err(error.to_string());
        }
        if let Err(error) = persist_settings(&updated) {
            let _ = window.set_always_on_top(previous);
            let _ = item.set_checked(previous);
            return Err(error);
        }
        *settings = updated;
    }
    native_ui::request_redraw(app);
    Ok(())
}

fn open_tray_settings(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let settings = state
        .settings
        .lock()
        .map_err(|_| "failed to read settings".to_string())?
        .clone();
    let window = app
        .get_window("main")
        .ok_or_else(|| "main window not available".to_string())?;
    let return_to_taskbar = settings.presentation_mode == PresentationMode::Taskbar;

    apply_runtime_window_shape(&window, &settings.shape_mode, true);
    if let Err(error) = window.show() {
        apply_runtime_window_shape(&window, &settings.shape_mode, false);
        return Err(error.to_string());
    }
    if let Err(error) = window.set_focus() {
        apply_runtime_window_shape(&window, &settings.shape_mode, false);
        if return_to_taskbar {
            let _ = window.hide();
        }
        return Err(error.to_string());
    }
    native_ui::restore_window_style(&window)?;

    let controller = match state.native_ui.lock() {
        Ok(controller) => controller.clone(),
        Err(_) => None,
    };
    let Some(controller) = controller else {
        apply_runtime_window_shape(&window, &settings.shape_mode, false);
        if return_to_taskbar {
            let _ = window.hide();
        }
        return Err("native window controller not available".to_string());
    };
    if let Err(error) = controller.open_settings_panel(return_to_taskbar) {
        apply_runtime_window_shape(&window, &settings.shape_mode, false);
        if return_to_taskbar {
            let _ = window.hide();
        }
        return Err(error);
    }

    Ok(())
}

fn build_tray(
    app: &AppHandle,
    always_on_top: bool,
    presentation_mode: PresentationMode,
    language: Language,
) -> Result<
    (
        CheckMenuItem<tauri::Wry>,
        MenuItem<tauri::Wry>,
        MenuItem<tauri::Wry>,
        MenuItem<tauri::Wry>,
        Submenu<tauri::Wry>,
        CheckMenuItem<tauri::Wry>,
        CheckMenuItem<tauri::Wry>,
    ),
    String,
> {
    let always_on_top_item = CheckMenuItem::with_id(
        app,
        TRAY_ALWAYS_ON_TOP_ID,
        language.text("置顶", "Always on top"),
        true,
        always_on_top,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let presentation_item = MenuItem::with_id(
        app,
        TRAY_PRESENTATION_ID,
        if presentation_mode == PresentationMode::Floating {
            language.text("显示到任务栏", "Show in taskbar")
        } else {
            language.text("显示悬浮窗", "Show floating window")
        },
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    always_on_top_item
        .set_enabled(presentation_mode == PresentationMode::Floating)
        .map_err(|error| error.to_string())?;
    let settings_item = MenuItem::with_id(
        app,
        TRAY_SETTINGS_ID,
        language.text("设置", "Settings"),
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let close_item = MenuItem::with_id(
        app,
        TRAY_CLOSE_ID,
        language.text("退出", "Exit"),
        true,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let language_zh_item = CheckMenuItem::with_id(
        app,
        TRAY_LANGUAGE_ZH_ID,
        "中文",
        true,
        language == Language::Zh,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let language_en_item = CheckMenuItem::with_id(
        app,
        TRAY_LANGUAGE_EN_ID,
        "English",
        true,
        language == Language::En,
        None::<&str>,
    )
    .map_err(|error| error.to_string())?;
    let language_menu = Submenu::with_items(
        app,
        language.text("语言", "Language"),
        true,
        &[&language_zh_item, &language_en_item],
    )
    .map_err(|error| error.to_string())?;
    let menu = tauri::menu::Menu::with_items(
        app,
        &[
            &settings_item,
            &language_menu,
            &presentation_item,
            &always_on_top_item,
            &close_item,
        ],
    )
    .map_err(|error| error.to_string())?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "default window icon not available".to_string())?;

    TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .icon(icon)
        .tooltip("CQBar")
        .build(app)
        .map_err(|error| error.to_string())?;

    Ok((
        always_on_top_item,
        presentation_item,
        settings_item,
        close_item,
        language_menu,
        language_zh_item,
        language_en_item,
    ))
}

pub fn run() {
    diagnostics::init();
    let initial_settings = load_settings();
    diagnostics::event(
        "INFO",
        "settings.loaded",
        &format!(
            "shape={:?} presentation={:?} auto_refresh={} interval_min={}",
            initial_settings.shape_mode,
            initial_settings.presentation_mode,
            initial_settings.auto_refresh_enabled,
            initial_settings.auto_refresh_minutes
        ),
    );

    tauri::Builder::default()
        .manage(AppState {
            settings: Mutex::new(initial_settings.clone()),
            always_on_top_item: Mutex::new(None),
            presentation_item: Mutex::new(None),
            settings_item: Mutex::new(None),
            close_item: Mutex::new(None),
            language_menu: Mutex::new(None),
            language_zh_item: Mutex::new(None),
            language_en_item: Mutex::new(None),
            snapshot: Mutex::new(None),
            last_good_snapshot: Mutex::new(None),
            last_refresh: Mutex::new(None),
            refresh_gate: tokio::sync::Mutex::new(()),
            refreshing: AtomicBool::new(false),
            native_ui: Mutex::new(None),
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_ALWAYS_ON_TOP_ID => {
                diagnostics::event("INFO", "tray.always_on_top", "");
                if let Err(error) = apply_tray_always_on_top(app) {
                    diagnostics::event("ERROR", "tray.always_on_top_failed", &error);
                }
            }
            TRAY_PRESENTATION_ID => {
                diagnostics::event("INFO", "tray.presentation", "");
                if let Err(error) = switch_presentation(app) {
                    diagnostics::event("ERROR", "presentation.switch_failed", &error);
                    let language = app
                        .state::<AppState>()
                        .settings
                        .lock()
                        .map(|s| s.effective_language())
                        .unwrap_or(Language::En);
                    notify_native_error(language.text(
                        "切换显示方式失败，请查看应用日志。",
                        "Could not switch display mode. See the application log.",
                    ));
                }
            }
            TRAY_SETTINGS_ID => {
                diagnostics::event("INFO", "tray.settings", "");
                if let Err(error) = open_tray_settings(app) {
                    diagnostics::event("ERROR", "tray.settings_failed", &error);
                    let language = app
                        .state::<AppState>()
                        .settings
                        .lock()
                        .map(|s| s.effective_language())
                        .unwrap_or(Language::En);
                    notify_native_error(language.text(
                        "无法打开设置，请查看应用日志。",
                        "Could not open settings. See the application log.",
                    ));
                }
            }
            TRAY_LANGUAGE_ZH_ID | TRAY_LANGUAGE_EN_ID => {
                let language = if event.id().as_ref() == TRAY_LANGUAGE_ZH_ID {
                    Language::Zh
                } else {
                    Language::En
                };
                if let Err(error) = change_language(app, language) {
                    diagnostics::event("ERROR", "settings.language_change_failed", &error);
                    notify_native_error(language.text(
                        "无法保存语言设置，请查看应用日志。",
                        "Could not save the language setting. See the application log.",
                    ));
                }
            }
            TRAY_CLOSE_ID => {
                diagnostics::event("INFO", "app.exit", "source=tray");
                let _ = deskband_action("hide");
                app.exit(0);
            }
            _ => {}
        })
        .setup(move |app| {
            diagnostics::event("INFO", "app.setup.start", "");
            let (
                always_on_top_item,
                presentation_item,
                settings_item,
                close_item,
                language_menu,
                language_zh_item,
                language_en_item,
            ) = build_tray(
                app.handle(),
                initial_settings.always_on_top,
                initial_settings.presentation_mode,
                initial_settings.effective_language(),
            )
            .map_err(io::Error::other)?;
            let state = app.state::<AppState>();
            state
                .always_on_top_item
                .lock()
                .map_err(|_| io::Error::other("failed to initialize tray state"))?
                .replace(always_on_top_item);
            state
                .presentation_item
                .lock()
                .map_err(|_| io::Error::other("failed to initialize presentation tray item"))?
                .replace(presentation_item);
            state
                .settings_item
                .lock()
                .map_err(|_| io::Error::other("failed to initialize settings tray item"))?
                .replace(settings_item);
            state
                .close_item
                .lock()
                .map_err(|_| io::Error::other("failed to initialize exit tray item"))?
                .replace(close_item);
            state
                .language_menu
                .lock()
                .map_err(|_| io::Error::other("failed to initialize language menu"))?
                .replace(language_menu);
            state
                .language_zh_item
                .lock()
                .map_err(|_| io::Error::other("failed to initialize Chinese menu item"))?
                .replace(language_zh_item);
            state
                .language_en_item
                .lock()
                .map_err(|_| io::Error::other("failed to initialize English menu item"))?
                .replace(language_en_item);

            let window = tauri::window::WindowBuilder::new(app, "main")
                .title(initial_settings.effective_language().text("CQBar", "CQBar"))
                .inner_size(SQUARE_WIDTH, SQUARE_HEIGHT)
                .center()
                .decorations(false)
                .shadow(false)
                .resizable(false)
                .visible(false)
                .skip_taskbar(true)
                .always_on_top(false)
                .build()
                .map_err(io::Error::other)?;
            apply_saved_window_position(&window, &initial_settings);
            apply_base_shape_size(&window, &initial_settings.shape_mode);
            window
                .set_always_on_top(initial_settings.always_on_top)
                .map_err(io::Error::other)?;
            let controller = native_ui::install(app.handle(), &window).map_err(io::Error::other)?;
            diagnostics::event("INFO", "ui.native.installed", "");
            state
                .native_ui
                .lock()
                .map_err(|_| io::Error::other("failed to initialize native UI"))?
                .replace(controller);

            if initial_settings.presentation_mode == PresentationMode::Taskbar {
                match show_taskbar() {
                    Ok(true) => {
                        if let Err(error) = window.hide() {
                            diagnostics::event(
                                "ERROR",
                                "presentation.hide_failed",
                                &error.to_string(),
                            );
                            let _ = deskband_action("hide");
                            if let Ok(mut settings) = state.settings.lock() {
                                settings.presentation_mode = PresentationMode::Floating;
                                let _ = persist_settings(&settings);
                            }
                            sync_tray_presentation(
                                &state,
                                PresentationMode::Floating,
                                initial_settings.effective_language(),
                            );
                            window.show().map_err(io::Error::other)?;
                            notify_native_error(
                                initial_settings.effective_language().text(
                                    "无法隐藏悬浮窗。",
                                    "Could not hide the floating window.",
                                ),
                            );
                        }
                    }
                    other => {
                        let reason = match other {
                            Ok(false) => "Windows 未能显示任务栏工具栏".to_string(),
                            Err(error) => error,
                            Ok(true) => unreachable!(),
                        };
                        diagnostics::event("ERROR", "presentation.taskbar_restore_failed", &reason);
                        let mut settings = state
                            .settings
                            .lock()
                            .map_err(|_| io::Error::other("failed to restore floating mode"))?;
                        settings.presentation_mode = PresentationMode::Floating;
                        let _ = persist_settings(&settings);
                        sync_tray_presentation(
                            &state,
                            PresentationMode::Floating,
                            initial_settings.effective_language(),
                        );
                        window.show().map_err(io::Error::other)?;
                        notify_native_error(initial_settings.effective_language().text(
                            "任务栏工具栏不可用，已恢复悬浮窗。",
                            "Taskbar toolbar unavailable; the floating window was restored.",
                        ));
                    }
                }
            } else {
                window.show().map_err(io::Error::other)?;
            }
            if state
                .settings
                .lock()
                .map(|settings| settings.presentation_mode == PresentationMode::Floating)
                .unwrap_or(false)
            {
                native_ui::restore_window_style(&window).map_err(io::Error::other)?;
                native_ui::request_redraw(app.handle());
            }
            if let Err(error) = write_taskbar_state(&state) {
                diagnostics::event("ERROR", "taskbar.state.write_failed", &error);
            }
            tauri::async_runtime::spawn(background_refresh_loop(app.handle().clone()));
            let actual_presentation = state
                .settings
                .lock()
                .map(|settings| settings.presentation_mode)
                .unwrap_or(initial_settings.presentation_mode);
            diagnostics::event(
                "INFO",
                "app.setup.finish",
                &format!("presentation={actual_presentation:?}"),
            );

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Moved(position) => persist_window_position(window, *position),
            WindowEvent::Focused(true) => {
                let state = window.state::<AppState>();
                if let Ok(settings) = state.settings.lock() {
                    if settings.presentation_mode == PresentationMode::Floating
                        && settings.always_on_top
                    {
                        let _ = window.set_always_on_top(true);
                        if let Err(error) = native_ui::restore_window_style(window) {
                            diagnostics::event("ERROR", "ui.window_style_restore_failed", &error);
                        }
                    }
                };
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_usage_snapshot,
            get_app_settings,
            update_app_settings
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|error| {
            diagnostics::event("ERROR", "app.run_failed", &error.to_string());
            panic!("error while running tauri application");
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effect_padding_expands_native_window_without_changing_content_size() {
        assert_eq!(WIDGET_CONTENT_WIDTH, 190.0);
        assert_eq!(WIDGET_CONTENT_SQUARE_HEIGHT, 190.0);
        assert_eq!(WIDGET_CONTENT_COMPACT_HEIGHT, 40.0);
        assert_eq!(SQUARE_WIDTH, 210.0);
        assert_eq!(SQUARE_HEIGHT, 210.0);
        assert_eq!(COMPACT_HEIGHT, 60.0);
    }

    #[test]
    fn effect_padding_preserves_the_saved_content_anchor() {
        let content_anchor = PhysicalPosition::new(120, 80);
        let native_origin = window_origin_for_content_anchor(content_anchor, 10);
        assert_eq!(native_origin, PhysicalPosition::new(110, 70));
        assert_eq!(
            content_anchor_for_window_origin(native_origin, 10),
            content_anchor
        );
    }

    #[test]
    fn old_settings_default_compact_display_flags() {
        let settings: AppSettings = serde_json::from_str(
            r#"{
                "autoRefreshEnabled": true,
                "autoRefreshMinutes": 5,
                "windowPosition": null,
                "alwaysOnTop": true,
                "displayMode": "remaining",
                "compactDisplayMode": "time",
                "themeMode": "dark",
                "shapeMode": "square"
            }"#,
        )
        .expect("old settings should remain readable");

        assert!(!settings.compact_display.five_hour_time);
        assert!(!settings.compact_display.weekly_date);
        assert_eq!(settings.presentation_mode, PresentationMode::Floating);
        assert_eq!(settings.language, None);
    }

    #[test]
    fn windows_ui_language_defaults_to_chinese_or_english() {
        assert_eq!(language_for_ui_lang_id(0x0804), Language::Zh);
        assert_eq!(language_for_ui_lang_id(0x0404), Language::Zh);
        assert_eq!(language_for_ui_lang_id(0x0409), Language::En);
        assert_eq!(language_for_ui_lang_id(0x040c), Language::En);
        let mut settings = AppSettings::default();
        settings.language = Some(Language::En);
        assert_eq!(settings.effective_language(), Language::En);
        let saved = serde_json::to_string(&settings).unwrap();
        assert_eq!(
            serde_json::from_str::<AppSettings>(&saved)
                .unwrap()
                .language,
            Some(Language::En)
        );
    }

    #[test]
    fn taskbar_controller_uses_installed_copy_for_an_executable_started_elsewhere() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "codex-quota-float-controller-path-{}-{unique}",
            std::process::id()
        ));
        let running_directory = root.join("manual-launch");
        let installed_directory = root.join("installed");
        fs::create_dir_all(&running_directory).expect("running directory should be created");
        fs::create_dir_all(&installed_directory).expect("installed directory should be created");

        let executable = running_directory.join("CQBar.exe");
        let installed_controller = installed_directory.join("deskbandctl.exe");
        fs::write(&installed_controller, b"test controller")
            .expect("installed controller fixture should be created");
        assert_eq!(
            find_deskband_controller(&executable, Some(&installed_directory)),
            Some(installed_controller.clone())
        );

        let local_controller = running_directory.join("deskbandctl.exe");
        fs::write(&local_controller, b"local controller")
            .expect("local controller fixture should be created");
        assert_eq!(
            find_deskband_controller(&executable, Some(&installed_directory)),
            Some(local_controller)
        );

        fs::remove_dir_all(root).expect("temporary controller fixtures should be removed");
    }
}
