use std::{
    ptr::null_mut,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Local};
use tauri::{AppHandle, Manager, Window};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
        Graphics::{
            Gdi::{
                CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject,
                AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
                DIB_RGB_COLORS, HGDIOBJ,
            },
            GdiPlus::{
                CombineModeReplace, CompositingModeSourceOver, FillModeAlternate, FontStyleBold,
                FontStyleRegular, GdipAddPathArc, GdipClosePathFigure, GdipCreateBitmapFromScan0,
                GdipCreateFont, GdipCreateFontFamilyFromName, GdipCreateLineBrushFromRect,
                GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipCreateStringFormat,
                GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily, GdipDeleteGraphics,
                GdipDeletePath, GdipDeletePen, GdipDeleteStringFormat, GdipDisposeImage,
                GdipDrawArc, GdipDrawLine, GdipDrawPath, GdipDrawString, GdipFillPath,
                GdipGetImageGraphicsContext, GdipGraphicsClear, GdipResetClip,
                GdipScaleWorldTransform, GdipSetClipRect, GdipSetCompositingMode,
                GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdipSetStringFormatAlign,
                GdipSetStringFormatFlags, GdipSetStringFormatLineAlign,
                GdipSetStringFormatTrimming, GdipSetTextRenderingHint, GdiplusStartup,
                GdiplusStartupInput, GpBitmap, GpFont, GpFontFamily, GpGraphics, GpLineGradient,
                GpPath, GpPen, GpStringFormat, LinearGradientModeVertical, MatrixOrderPrepend,
                PixelOffsetModeHalf, RectF, SmoothingModeAntiAlias, Status, StringAlignment,
                StringAlignmentCenter, StringAlignmentFar, StringAlignmentNear,
                StringTrimmingEllipsisCharacter, TextRenderingHintAntiAliasGridFit, UnitPixel,
                WrapModeTile,
            },
        },
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT},
            Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, SUBCLASSPROC},
            WindowsAndMessaging::{
                GetClientRect, GetWindowLongPtrW, GetWindowRect, KillTimer, LoadCursorW,
                PostMessageW, SetCursor, SetTimer, SetWindowLongPtrW, SetWindowPos,
                UpdateLayeredWindow, GWL_EXSTYLE, GWL_STYLE, IDC_ARROW, IDC_HAND, STYLESTRUCT,
                SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, ULW_ALPHA, WM_DPICHANGED,
                WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
                WM_MOUSEMOVE, WM_NCDESTROY, WM_NCHITTEST, WM_RBUTTONUP, WM_SETCURSOR,
                WM_STYLECHANGING, WM_TIMER, WS_CAPTION, WS_EX_LAYERED, WS_EX_WINDOWEDGE,
                WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
            },
        },
    },
};

use super::{
    apply_runtime_window_shape, diagnostics, persist_settings, write_taskbar_state, AppSettings,
    AppState, DisplayMode, Language, PresentationMode, ShapeMode, ThemeMode, UsageSnapshot,
    UsageStatus, WINDOW_EFFECT_PADDING,
};

#[cfg(test)]
use windows::Win32::Graphics::GdiPlus::GdipMeasureString;

const SUBCLASS_ID: usize = 0x4351_464C;
const DRAG_TIMER_ID: usize = 0x4351_464D;
const DRAG_DELAY: Duration = Duration::from_millis(220);
const WM_NATIVE_REDRAW: u32 = 0x8000 + 42;
const WM_NATIVE_OPEN_SETTINGS: u32 = 0x8000 + 43;
const WM_MOUSELEAVE: u32 = 0x02A3;
const HTCLIENT: u16 = 1;
const HTTRANSPARENT: isize = -1;
const PIXEL_FORMAT_32BPP_PARGB: i32 = 0x000E_200B;
const MASK_SUPERSAMPLE: u32 = 8;
const STRING_FORMAT_FLAGS_NO_WRAP: i32 = 0x0000_1000;
const COMPACT_VALUE_INSET: f32 = 4.0;
const COMPACT_VALUE_WIDTH: f32 = 37.0;
const COMPACT_PERCENT_INSET: f32 = 41.0;
const COMPACT_PERCENT_WIDTH: f32 = 35.0;
const SETTINGS_PANEL_CARD_INSET: f32 = 5.0;
const SETTINGS_PANEL_CARD_RADIUS: f32 = 18.0;
const SETTINGS_PANEL_RADIUS: f32 = SETTINGS_PANEL_CARD_INSET + SETTINGS_PANEL_CARD_RADIUS;

static GDI_PLUS_TOKEN: OnceLock<Result<usize, String>> = OnceLock::new();

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorMode {
    Arrow = 0,
    Click = 1,
}

fn cursor_mode_for_hit(hit: Hit) -> CursorMode {
    match hit {
        Hit::Outside => CursorMode::Arrow,
        Hit::None => CursorMode::Arrow,
        _ => CursorMode::Click,
    }
}

fn cursor_mode_from_value(value: u8) -> CursorMode {
    match value {
        1 => CursorMode::Click,
        _ => CursorMode::Arrow,
    }
}

fn cursor_handle(mode: CursorMode) -> Option<windows::Win32::UI::WindowsAndMessaging::HCURSOR> {
    match mode {
        CursorMode::Arrow => unsafe { LoadCursorW(None, IDC_ARROW).ok() },
        CursorMode::Click => unsafe { LoadCursorW(None, IDC_HAND).ok() },
    }
}

fn set_cursor_mode(state: &NativeUiState, mode: CursorMode) {
    state.cursor_mode.store(mode as u8, Ordering::Relaxed);
    if let Some(cursor) = cursor_handle(mode) {
        unsafe { SetCursor(Some(cursor)) };
    }
}

#[derive(Clone)]
pub(crate) struct NativeUiController {
    hwnd: HWND,
}

unsafe impl Send for NativeUiController {}

impl NativeUiController {
    pub(crate) fn request_redraw(&self) {
        unsafe {
            let _ = PostMessageW(
                Some(self.hwnd),
                WM_NATIVE_REDRAW,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }

    pub(crate) fn open_settings_panel(&self, return_to_taskbar: bool) -> Result<(), String> {
        unsafe {
            PostMessageW(
                Some(self.hwnd),
                WM_NATIVE_OPEN_SETTINGS,
                windows::Win32::Foundation::WPARAM(if return_to_taskbar { 1 } else { 0 }),
                windows::Win32::Foundation::LPARAM(0),
            )
            .map_err(|error| format!("failed to open settings panel: {error}"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Outside,
    None,
    Refresh,
    FiveHour,
    Weekly,
    Interval(u32),
    DisplayRemaining,
    DisplayUsed,
    ThemeLight,
    ThemeDark,
    AlwaysOnTop,
    Close,
    PanelOutside,
}

struct WindowUi {
    app: AppHandle,
    window: Window,
    renderer: Renderer,
    panel_open: bool,
    return_to_taskbar_after_panel: bool,
    hovered: Hit,
    pressed: Hit,
    press_started: Option<Instant>,
    tracking_mouse: bool,
}

struct NativeUiState {
    ui: Mutex<WindowUi>,
    cursor_mode: AtomicU8,
}

struct Renderer {
    font_family: *mut GpFontFamily,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            if !self.font_family.is_null() {
                let _ = GdipDeleteFontFamily(self.font_family);
            }
        }
    }
}

pub(crate) fn install(app: &AppHandle, window: &Window) -> Result<NativeUiController, String> {
    ensure_gdiplus()?;
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    ensure_native_window_style(hwnd)?;

    let renderer = Renderer::new()?;
    let ui = Arc::new(NativeUiState {
        ui: Mutex::new(WindowUi {
            app: app.clone(),
            window: window.clone(),
            renderer,
            panel_open: false,
            return_to_taskbar_after_panel: false,
            hovered: Hit::Outside,
            pressed: Hit::Outside,
            press_started: None,
            tracking_mouse: false,
        }),
        cursor_mode: AtomicU8::new(CursorMode::Arrow as u8),
    });
    let callback_state = Arc::into_raw(ui.clone()) as usize;
    let subclass: SUBCLASSPROC = Some(window_proc);
    let attached = unsafe { SetWindowSubclass(hwnd, subclass, SUBCLASS_ID, callback_state) };
    if !attached.as_bool() {
        unsafe { drop(Arc::from_raw(callback_state as *const NativeUiState)) };
        return Err("failed to attach native window input handler".to_string());
    }

    let controller = NativeUiController { hwnd };
    ui.ui
        .lock()
        .map_err(|_| "failed to initialize native window".to_string())?
        .renderer
        .draw(hwnd, app, false, Hit::None)?;
    Ok(controller)
}

pub(crate) fn restore_window_style(window: &Window) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|error| error.to_string())?;
    ensure_native_window_style(hwnd)
}

fn native_chrome_bits() -> u32 {
    WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0
}

fn ensure_native_window_style(hwnd: HWND) -> Result<(), String> {
    unsafe {
        // Tao can reapply caption styles when show/topmost/taskbar flags change.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let chrome = native_chrome_bits() as isize;
        let wanted_style = style & !chrome;
        let wanted_exstyle = (exstyle | WS_EX_LAYERED.0 as isize) & !(WS_EX_WINDOWEDGE.0 as isize);
        if style != wanted_style || exstyle != wanted_exstyle {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted_style);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted_exstyle);
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            )
            .map_err(|error| format!("failed to restore native window style: {error}"))?;
            diagnostics::event(
                "INFO",
                "ui.window_style_restored",
                &format!("style=0x{:X} exstyle=0x{:X}", style as u32, exstyle as u32),
            );
            if GetWindowLongPtrW(hwnd, GWL_STYLE) & chrome != 0
                || GetWindowLongPtrW(hwnd, GWL_EXSTYLE) != wanted_exstyle
            {
                return Err("Windows did not retain the borderless layered style".to_string());
            }
        }
    }
    Ok(())
}

pub(crate) fn request_redraw(app: &AppHandle) {
    let state = app.state::<AppState>();
    let controller = state.native_ui.lock().ok().and_then(|guard| guard.clone());
    if let Some(controller) = controller {
        controller.request_redraw();
    }
}

fn ensure_gdiplus() -> Result<usize, String> {
    GDI_PLUS_TOKEN
        .get_or_init(|| unsafe {
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                DebugEventCallback: 0,
                SuppressBackgroundThread: false.into(),
                SuppressExternalCodecs: false.into(),
            };
            let mut token = 0;
            let status = GdiplusStartup(&mut token, &input, null_mut());
            if status == Status(0) {
                Ok(token)
            } else {
                Err(format!("GDI+ startup failed with status {status:?}"))
            }
        })
        .clone()
}

impl Renderer {
    fn new() -> Result<Self, String> {
        let _ = ensure_gdiplus()?;
        let family_name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let mut font_family = null_mut();
        let status = unsafe {
            GdipCreateFontFamilyFromName(PCWSTR(family_name.as_ptr()), null_mut(), &mut font_family)
        };
        if status != Status(0) || font_family.is_null() {
            return Err(format!(
                "Segoe UI font is unavailable (GDI+ status {status:?})"
            ));
        }
        Ok(Self { font_family })
    }

    fn draw(
        &mut self,
        hwnd: HWND,
        app: &AppHandle,
        panel_open: bool,
        hovered: Hit,
    ) -> Result<(), String> {
        ensure_native_window_style(hwnd)?;
        let model = build_view_model(app, panel_open, hovered);
        let mut client = RECT::default();
        unsafe { GetClientRect(hwnd, &mut client).map_err(|error| error.to_string())? };
        let window_width = (client.right - client.left).max(1) as u32;
        let window_height = (client.bottom - client.top).max(1) as u32;
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        let scale = dpi as f32 / 96.0;
        let padding = (WINDOW_EFFECT_PADDING * scale).round() as u32;
        let content_width = window_width
            .saturating_sub(padding.saturating_mul(2))
            .max(1);
        let content_height = window_height
            .saturating_sub(padding.saturating_mul(2))
            .max(1);
        let logical_width = content_width as f32 / scale;
        let logical_height = content_height as f32 / scale;
        let content_stride = (content_width * 4) as i32;
        let mut content_pixels = vec![0_u8; content_stride as usize * content_height as usize];
        let mut bitmap: *mut GpBitmap = null_mut();
        let mut graphics: *mut GpGraphics = null_mut();

        let create_status = unsafe {
            GdipCreateBitmapFromScan0(
                content_width as i32,
                content_height as i32,
                content_stride,
                PIXEL_FORMAT_32BPP_PARGB,
                Some(content_pixels.as_mut_ptr()),
                &mut bitmap,
            )
        };
        if create_status != Status(0) || bitmap.is_null() {
            return Err(format!(
                "GDI+ bitmap creation failed with status {create_status:?}"
            ));
        }
        let graphics_status = unsafe { GdipGetImageGraphicsContext(bitmap.cast(), &mut graphics) };
        if graphics_status != Status(0) || graphics.is_null() {
            unsafe {
                let _ = GdipDisposeImage(bitmap.cast());
            }
            return Err(format!(
                "GDI+ graphics creation failed with status {graphics_status:?}"
            ));
        }
        unsafe {
            let _ = GdipGraphicsClear(graphics, 0);
            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            let _ = GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
            let _ = GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
            let _ = GdipSetCompositingMode(graphics, CompositingModeSourceOver);
            let _ = GdipScaleWorldTransform(graphics, scale, scale, MatrixOrderPrepend);
        }
        fill_shell_pixels(
            &mut content_pixels,
            content_width,
            content_height,
            scale,
            &model,
        );

        let fonts = FontSet::new(self.font_family)?;
        draw_window(graphics, logical_width, logical_height, &model, &fonts);
        drop(fonts);
        unsafe {
            let _ = GdipDeleteGraphics(graphics);
        }
        mask_window_shell(
            &mut content_pixels,
            content_width,
            content_height,
            scale,
            matches!(model.settings.shape_mode, ShapeMode::Compact),
            panel_open,
        );
        let mut pixels = compose_padded_content(
            &content_pixels,
            content_width,
            content_height,
            window_width,
            window_height,
            padding,
        );
        draw_edge_effect(
            &mut pixels,
            window_width,
            window_height,
            scale,
            logical_width,
            logical_height,
            matches!(model.settings.shape_mode, ShapeMode::Compact),
            model.panel_open,
            matches!(model.settings.theme_mode, ThemeMode::Light),
        );

        let result = unsafe { update_layered_window(hwnd, window_width, window_height, &pixels) };
        unsafe {
            let _ = GdipDisposeImage(bitmap.cast());
        }
        result
    }
}

struct FontSet {
    title: *mut GpFont,
    body: *mut GpFont,
    small: *mut GpFont,
    metric: *mut GpFont,
    bold_small: *mut GpFont,
    compact: *mut GpFont,
}

impl FontSet {
    fn new(family: *mut GpFontFamily) -> Result<Self, String> {
        unsafe fn make_font(
            family: *mut GpFontFamily,
            size: f32,
            style: i32,
        ) -> Result<*mut GpFont, String> {
            let mut font = null_mut();
            let status = unsafe { GdipCreateFont(family, size, style, UnitPixel, &mut font) };
            if status == Status(0) && !font.is_null() {
                Ok(font)
            } else {
                Err(format!("GDI+ font creation failed with status {status:?}"))
            }
        }
        let title = unsafe { make_font(family, 11.0, FontStyleBold.0) }?;
        let body = unsafe { make_font(family, 11.0, FontStyleRegular.0) }?;
        let small = unsafe { make_font(family, 10.0, FontStyleRegular.0) }?;
        let metric = unsafe { make_font(family, 24.0, FontStyleBold.0) }?;
        let bold_small = unsafe { make_font(family, 10.0, FontStyleBold.0) }?;
        let compact = unsafe { make_font(family, 11.0, FontStyleBold.0) }?;
        Ok(Self {
            title,
            body,
            small,
            metric,
            bold_small,
            compact,
        })
    }

    fn dispose(&mut self) {
        unsafe {
            for font in [
                self.title,
                self.body,
                self.small,
                self.metric,
                self.bold_small,
                self.compact,
            ] {
                if !font.is_null() {
                    let _ = GdipDeleteFont(font);
                }
            }
        }
    }
}

impl Drop for FontSet {
    fn drop(&mut self) {
        self.dispose();
    }
}

struct ViewModel {
    settings: AppSettings,
    snapshot: Option<UsageSnapshot>,
    last_good: Option<UsageSnapshot>,
    refreshing: bool,
    panel_open: bool,
    hovered: Hit,
}

fn build_view_model(app: &AppHandle, panel_open: bool, hovered: Hit) -> ViewModel {
    let state = app.state::<AppState>();
    let snapshot = state.snapshot.lock().ok().and_then(|value| value.clone());
    let last_good = state
        .last_good_snapshot
        .lock()
        .ok()
        .and_then(|value| value.clone());
    ViewModel {
        settings: state
            .settings
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default(),
        snapshot,
        last_good,
        refreshing: state.refreshing.load(std::sync::atomic::Ordering::Relaxed),
        panel_open,
        hovered,
    }
}

unsafe fn update_layered_window(
    hwnd: HWND,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), String> {
    ensure_native_window_style(hwnd)?;
    let memory_dc = unsafe { CreateCompatibleDC(None) };
    if memory_dc.is_invalid() {
        return Err("failed to create layered-window memory DC".to_string());
    }

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: -(height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: (width * height * 4) as u32,
            ..Default::default()
        },
        bmiColors: [Default::default()],
    };
    let mut bits = null_mut();
    let bitmap = unsafe {
        CreateDIBSection(Some(memory_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
            .map_err(|error| format!("CreateDIBSection failed: {error}"))?
    };
    if bits.is_null() {
        unsafe {
            let _ = DeleteDC(memory_dc);
        }
        return Err("failed to allocate layered-window pixels".to_string());
    }
    unsafe {
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast(), pixels.len());
    }
    let old_bitmap = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };

    let mut rect = RECT::default();
    let position_result = unsafe { GetWindowRect(hwnd, &mut rect) };
    let mut result = position_result.map_err(|error| format!("GetWindowRect failed: {error}"));
    if result.is_ok() {
        let destination = POINT {
            x: rect.left,
            y: rect.top,
        };
        let source = POINT { x: 0, y: 0 };
        let size = SIZE {
            cx: width as i32,
            cy: height as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        result = unsafe {
            UpdateLayeredWindow(
                hwnd,
                None,
                Some(&destination),
                Some(&size),
                Some(memory_dc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            )
            .map_err(|error| {
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                format!(
                    "UpdateLayeredWindow failed (exstyle=0x{:X}, size={}x{}): {error}",
                    style as u32, width, height
                )
            })
        };
    }

    unsafe {
        let _ = SelectObject(memory_dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
    }
    result
}

fn fill_shell_pixels(pixels: &mut [u8], width: u32, height: u32, scale: f32, model: &ViewModel) {
    let logical_width = width as f32 / scale;
    let logical_height = height as f32 / scale;
    let compact = matches!(model.settings.shape_mode, ShapeMode::Compact);
    let radius = if compact && !model.panel_open {
        logical_width.min(logical_height) / 2.0
    } else {
        SETTINGS_PANEL_RADIUS
    };
    let (top_color, bottom_color) = if matches!(model.settings.theme_mode, ThemeMode::Light) {
        (0xF5F9FCFF, 0xEAE7EEFA)
    } else {
        (0xEF1C2738, 0xE80A0E13)
    };
    let shell = (3.0, 3.0, logical_width - 3.0, logical_height - 3.0, radius);

    for y in 0..height {
        for x in 0..width {
            let logical_x = (x as f32 + 0.5) / scale;
            let logical_y = (y as f32 + 0.5) / scale;
            let index = ((y * width + x) * 4) as usize;
            let within_shell_bounds = logical_x >= shell.0
                && logical_x < shell.2
                && logical_y >= shell.1
                && logical_y < shell.3;
            let inside_shell = if compact {
                within_shell_bounds
            } else {
                inside_rounded_rect(logical_x, logical_y, shell)
            };
            if inside_shell {
                let amount = ((logical_y - shell.1) / (shell.3 - shell.1)).clamp(0.0, 1.0);
                let color = interpolate_argb(top_color, bottom_color, amount);
                let alpha = (color >> 24) & 0xFF;
                let red = (color >> 16) & 0xFF;
                let green = (color >> 8) & 0xFF;
                let blue = color & 0xFF;
                pixels[index] = ((blue * alpha + 127) / 255) as u8;
                pixels[index + 1] = ((green * alpha + 127) / 255) as u8;
                pixels[index + 2] = ((red * alpha + 127) / 255) as u8;
                pixels[index + 3] = alpha as u8;
            }
        }
    }
}

fn rounded_rect_signed_distance(x: f32, y: f32, rect: (f32, f32, f32, f32, f32)) -> f32 {
    let (left, top, right, bottom, radius) = rect;
    let radius = radius.min((right - left) / 2.0).min((bottom - top) / 2.0);
    let qx = (x - (left + right) / 2.0).abs() - ((right - left) / 2.0 - radius);
    let qy = (y - (top + bottom) / 2.0).abs() - ((bottom - top) / 2.0 - radius);
    let outside_x = qx.max(0.0);
    let outside_y = qy.max(0.0);
    outside_x.hypot(outside_y) + qx.max(qy).min(0.0) - radius
}

fn light_edge_effect_alpha(distance: f32) -> u8 {
    let (inside_limit, outside_limit, peak, fade) = (5.0, 3.0, 78.0, 2.5);
    if distance < -inside_limit || distance > outside_limit {
        return 0;
    }
    let alpha = if distance >= 0.0 {
        peak * (-distance / fade).exp()
    } else {
        18.0 * (distance / 2.5).exp()
    };
    alpha.round().clamp(0.0, 255.0) as u8
}

fn neon_outer_glow_alpha(distance: f32) -> u8 {
    if !(-9.0..=7.0).contains(&distance) {
        return 0;
    }
    let alpha = if distance >= 0.0 {
        54.0 * (-distance / 4.0).exp()
    } else {
        38.0 * (distance / 8.0).exp()
    };
    alpha.round().clamp(0.0, 255.0) as u8
}

fn neon_rim_alpha(distance: f32) -> u8 {
    let distance = distance.abs();
    if distance > 5.0 {
        return 0;
    }
    (145.0 * (-distance / 1.4).exp()).round().clamp(0.0, 255.0) as u8
}

fn blend_edge_color(pixels: &mut [u8], index: usize, color: (u8, u8, u8), alpha: u8) {
    if alpha == 0 {
        return;
    }
    let (red, green, blue) = color;
    let inverse_alpha = 255_u32 - alpha as u32;
    for (channel, source) in [(0, blue), (1, green), (2, red)] {
        let source = source as u32 * alpha as u32;
        let destination = pixels[index + channel] as u32 * inverse_alpha;
        pixels[index + channel] = ((source + destination + 127) / 255) as u8;
    }
    let destination_alpha = pixels[index + 3] as u32 * inverse_alpha;
    pixels[index + 3] = (alpha as u32 + (destination_alpha + 127) / 255) as u8;
}

fn draw_edge_effect(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    scale: f32,
    content_width: f32,
    content_height: f32,
    compact: bool,
    panel_open: bool,
    light: bool,
) {
    let padding = WINDOW_EFFECT_PADDING;
    let radius = if compact && !panel_open {
        content_width.min(content_height) / 2.0
    } else {
        SETTINGS_PANEL_RADIUS
    };
    let shell = (
        padding + 3.0,
        padding + 3.0,
        padding + content_width - 3.0,
        padding + content_height - 3.0,
        radius,
    );
    for y in 0..height {
        for x in 0..width {
            let logical_x = (x as f32 + 0.5) / scale;
            let logical_y = (y as f32 + 0.5) / scale;
            let distance = rounded_rect_signed_distance(logical_x, logical_y, shell);
            let index = ((y * width + x) * 4) as usize;
            if light {
                blend_edge_color(
                    pixels,
                    index,
                    (72, 82, 96),
                    light_edge_effect_alpha(distance),
                );
            } else {
                blend_edge_color(
                    pixels,
                    index,
                    (28, 98, 255),
                    neon_outer_glow_alpha(distance),
                );
                blend_edge_color(pixels, index, (112, 232, 255), neon_rim_alpha(distance));
                let outline = (
                    shell.0 - 0.5,
                    shell.1 - 0.5,
                    shell.2 + 0.5,
                    shell.3 + 0.5,
                    shell.4 + 0.5,
                );
                blend_edge_color(
                    pixels,
                    index,
                    (194, 90, 255),
                    rounded_rect_outline_pixel_coverage(x, y, scale, outline, 1.0),
                );
            }
        }
    }
}

fn mask_window_shell(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    scale: f32,
    compact: bool,
    panel_open: bool,
) {
    let logical_width = width as f32 / scale;
    let logical_height = height as f32 / scale;
    let radius = if compact && !panel_open {
        logical_width.min(logical_height) / 2.0
    } else {
        SETTINGS_PANEL_RADIUS
    };
    let shell = (3.0, 3.0, logical_width - 3.0, logical_height - 3.0, radius);
    for y in 0..height {
        for x in 0..width {
            let coverage = rounded_rect_pixel_coverage(x, y, scale, shell);
            let index = ((y * width + x) * 4) as usize;
            if coverage == 0 {
                pixels[index..index + 4].fill(0);
            } else if coverage < 255 {
                for channel in &mut pixels[index..index + 4] {
                    *channel = ((*channel as u16 * coverage as u16 + 127) / 255) as u8;
                }
            }
        }
    }
}

fn compose_padded_content(
    content_pixels: &[u8],
    content_width: u32,
    content_height: u32,
    window_width: u32,
    window_height: u32,
    padding: u32,
) -> Vec<u8> {
    let window_stride = (window_width * 4) as usize;
    let row_length = (content_width * 4) as usize;
    let mut pixels = vec![0_u8; window_stride * window_height as usize];
    for y in 0..content_height as usize {
        let source_start = y * row_length;
        let target_start = (y + padding as usize) * window_stride + padding as usize * 4;
        pixels[target_start..target_start + row_length]
            .copy_from_slice(&content_pixels[source_start..source_start + row_length]);
    }
    pixels
}

fn rounded_rect_pixel_coverage(
    pixel_x: u32,
    pixel_y: u32,
    scale: f32,
    rect: (f32, f32, f32, f32, f32),
) -> u8 {
    let center_x = (pixel_x as f32 + 0.5) / scale;
    let center_y = (pixel_y as f32 + 0.5) / scale;
    let center_distance = rounded_rect_signed_distance(center_x, center_y, rect);
    let half_diagonal = 0.5_f32.sqrt() / scale;
    if center_distance <= -half_diagonal {
        return 255;
    }
    if center_distance >= half_diagonal {
        return 0;
    }

    let mut covered_samples = 0_u32;
    for sample_y in 0..MASK_SUPERSAMPLE {
        for sample_x in 0..MASK_SUPERSAMPLE {
            let physical_x = pixel_x as f32 + (sample_x as f32 + 0.5) / MASK_SUPERSAMPLE as f32;
            let physical_y = pixel_y as f32 + (sample_y as f32 + 0.5) / MASK_SUPERSAMPLE as f32;
            if inside_rounded_rect(physical_x / scale, physical_y / scale, rect) {
                covered_samples += 1;
            }
        }
    }
    ((covered_samples * 255 + (MASK_SUPERSAMPLE * MASK_SUPERSAMPLE) / 2)
        / (MASK_SUPERSAMPLE * MASK_SUPERSAMPLE)) as u8
}

fn rounded_rect_outline_pixel_coverage(
    pixel_x: u32,
    pixel_y: u32,
    scale: f32,
    rect: (f32, f32, f32, f32, f32),
    thickness: f32,
) -> u8 {
    let center_x = (pixel_x as f32 + 0.5) / scale;
    let center_y = (pixel_y as f32 + 0.5) / scale;
    let center_distance = rounded_rect_signed_distance(center_x, center_y, rect);
    let half_thickness = thickness / 2.0;
    let half_diagonal = 0.5_f32.sqrt() / scale;
    if center_distance.abs() >= half_thickness + half_diagonal {
        return 0;
    }

    let mut covered_samples = 0_u32;
    for sample_y in 0..MASK_SUPERSAMPLE {
        for sample_x in 0..MASK_SUPERSAMPLE {
            let physical_x = pixel_x as f32 + (sample_x as f32 + 0.5) / MASK_SUPERSAMPLE as f32;
            let physical_y = pixel_y as f32 + (sample_y as f32 + 0.5) / MASK_SUPERSAMPLE as f32;
            let distance =
                rounded_rect_signed_distance(physical_x / scale, physical_y / scale, rect);
            if distance.abs() <= half_thickness {
                covered_samples += 1;
            }
        }
    }
    ((covered_samples * 255 + (MASK_SUPERSAMPLE * MASK_SUPERSAMPLE) / 2)
        / (MASK_SUPERSAMPLE * MASK_SUPERSAMPLE)) as u8
}

fn should_toggle_settings_on_right_click(shape_mode: ShapeMode) -> bool {
    matches!(shape_mode, ShapeMode::Square | ShapeMode::Compact)
}

fn should_hide_window_after_settings_panel(
    return_to_taskbar: bool,
    presentation_mode: PresentationMode,
) -> bool {
    return_to_taskbar && presentation_mode == PresentationMode::Taskbar
}

fn inside_rounded_rect(x: f32, y: f32, rect: (f32, f32, f32, f32, f32)) -> bool {
    let (left, top, right, bottom, radius) = rect;
    if x < left || x >= right || y < top || y >= bottom {
        return false;
    }
    let radius = radius.min((right - left) / 2.0).min((bottom - top) / 2.0);
    let nearest_x = x.clamp(left + radius, right - radius);
    let nearest_y = y.clamp(top + radius, bottom - radius);
    let dx = x - nearest_x;
    let dy = y - nearest_y;
    dx * dx + dy * dy <= radius * radius
}

fn interpolate_argb(from: u32, to: u32, amount: f32) -> u32 {
    let mut result = 0_u32;
    for shift in [0, 8, 16, 24] {
        let start = ((from >> shift) & 0xFF) as f32;
        let end = ((to >> shift) & 0xFF) as f32;
        result |= ((start + (end - start) * amount).round() as u32) << shift;
    }
    result
}

fn draw_window(
    graphics: *mut GpGraphics,
    width: f32,
    height: f32,
    model: &ViewModel,
    fonts: &FontSet,
) {
    let compact = matches!(model.settings.shape_mode, ShapeMode::Compact);
    let light = matches!(model.settings.theme_mode, ThemeMode::Light);
    let language = model.settings.effective_language();
    if compact {
        draw_compact(graphics, width, height, model, fonts, false);
    } else {
        draw_square(graphics, width, height, model, fonts, false);
    }
    if model.snapshot.is_none() {
        text(
            graphics,
            fonts.body,
            language.text("加载中", "Loading"),
            13.0,
            72.0,
            width - 26.0,
            24.0,
            if light { 0xFF4D5F7C } else { 0xFFD2DBEE },
            StringAlignmentCenter,
        );
    }

    if model.panel_open {
        draw_settings_panel(graphics, width, height, model, fonts, compact, light);
    }
}

fn draw_square(
    graphics: *mut GpGraphics,
    _width: f32,
    _height: f32,
    model: &ViewModel,
    fonts: &FontSet,
    dimmed: bool,
) {
    let light = matches!(model.settings.theme_mode, ThemeMode::Light);
    let language = model.settings.effective_language();
    if dimmed {
        return;
    }
    let text_main = if light { 0xFF132033 } else { 0xFFF5F7FA };
    let text_muted = if light { 0xFF61728E } else { 0xFF90A0B7 };
    let text_sub = if light { 0xFF58708F } else { 0xFFD4DDED };
    let display = model.display_snapshot();
    let fallback = model.is_fallback();
    let account = if fallback {
        language.text("缓存", "Cached").to_string()
    } else {
        shorten_account(display.and_then(|s| s.account_label.as_deref()))
    };
    text(
        graphics,
        fonts.body,
        &account.to_uppercase(),
        13.0,
        8.0,
        142.0,
        20.0,
        if light { 0xFF4D5F7C } else { 0xFFD2DBEE },
        StringAlignmentNear,
    );
    draw_refresh_button(
        graphics,
        159.0,
        9.0,
        model.refreshing,
        model.settings.theme_mode,
        model.hovered == Hit::Refresh,
    );

    if let Some(snapshot) = display.filter(|value| !matches!(value.status, UsageStatus::Ok)) {
        draw_error_panel(graphics, snapshot, fonts, text_main, light, language);
    } else {
        let five = display.and_then(|s| s.five_hour_usage.as_ref());
        let weekly = display.and_then(|s| s.weekly_usage.as_ref());
        draw_metric_card(
            graphics,
            13.0,
            39.0,
            78.0,
            104.0,
            "5H",
            five,
            model.settings.display_mode,
            language,
            light,
            fonts,
        );
        draw_metric_card(
            graphics,
            99.0,
            39.0,
            78.0,
            104.0,
            "7D",
            weekly,
            model.settings.display_mode,
            language,
            light,
            fonts,
        );
    }
    let footer = if fallback {
        let plan = plan_text(display.and_then(|s| s.plan_type.as_deref()));
        match language {
            Language::Zh => format!("{plan} · 上次刷新失败"),
            Language::En => format!("{plan} · Last refresh failed"),
        }
    } else {
        format!(
            "{} · {}",
            plan_text(display.and_then(|s| s.plan_type.as_deref())),
            status_text(display.map(|s| &s.status), language)
        )
    };
    text(
        graphics,
        fonts.small,
        &footer,
        13.0,
        151.0,
        164.0,
        13.0,
        text_muted,
        StringAlignmentNear,
    );
    let footer_sub = format!(
        "5h {} | 7d {}",
        reset_time(display.and_then(|s| s.five_hour_usage.as_ref())),
        reset_date(display.and_then(|s| s.weekly_usage.as_ref()))
    );
    text(
        graphics,
        fonts.small,
        &footer_sub,
        13.0,
        164.0,
        164.0,
        13.0,
        text_sub,
        StringAlignmentNear,
    );
    let _ = text_main;
}

fn draw_compact(
    graphics: *mut GpGraphics,
    _width: f32,
    _height: f32,
    model: &ViewModel,
    fonts: &FontSet,
    dimmed: bool,
) {
    let light = matches!(model.settings.theme_mode, ThemeMode::Light);
    if dimmed {
        return;
    }
    let snapshot = model.display_snapshot();
    let fill_colors = if model.settings.display_mode == DisplayMode::Remaining {
        if light {
            (0x5B79B3FF, 0x4679B3FF)
        } else {
            (0x667EB9FF, 0x447EB9FF)
        }
    } else if light {
        (0x64FF9A82, 0x40FF9A82)
    } else {
        (0x6FFF8A71, 0x44FF8A71)
    };
    for (x, label, usage) in [
        (
            10.0,
            "5H",
            snapshot.and_then(|s| s.five_hour_usage.as_ref()),
        ),
        (100.0, "7D", snapshot.and_then(|s| s.weekly_usage.as_ref())),
    ] {
        let rect_width = 80.0;
        let radius = 12.0;
        fill_gradient_rounded(
            graphics,
            x,
            8.0,
            rect_width,
            24.0,
            radius,
            if light { 0xB8FFFFFF } else { 0x0AFFFFFF },
            if light { 0x9EE7EFFC } else { 0x06FFFFFF },
        );
        let percent = displayed_percent(usage, model.settings.display_mode);
        let pct = percent.map_or_else(|| "--".to_string(), |value| format!("{value}%"));
        let fill = percent.unwrap_or(0) as f32 / 100.0 * (rect_width - 2.0);
        if fill > 0.5 {
            let fill_x = if model.settings.display_mode == DisplayMode::Used {
                x + rect_width - 1.0 - fill
            } else {
                x + 1.0
            };
            fill_gradient_clipped_to_rounded(
                graphics,
                x + 1.0,
                9.0,
                rect_width - 2.0,
                22.0,
                11.0,
                fill_x,
                9.0,
                fill,
                22.0,
                fill_colors.0,
                fill_colors.1,
            );
        }
        let use_reset = model.settings.compact_display.five_hour_time
            && model.settings.compact_display.weekly_date;
        let value = if use_reset {
            if label == "5H" {
                reset_time(usage)
            } else {
                reset_date(usage)
            }
        } else {
            label.to_string()
        };
        text(
            graphics,
            fonts.compact,
            &value,
            x + COMPACT_VALUE_INSET,
            8.0,
            COMPACT_VALUE_WIDTH,
            24.0,
            if light { 0xFF15263D } else { 0xFFF7FBFF },
            StringAlignmentNear,
        );
        text(
            graphics,
            fonts.compact,
            &pct,
            x + COMPACT_PERCENT_INSET,
            8.0,
            COMPACT_PERCENT_WIDTH,
            24.0,
            if light { 0xFF15263D } else { 0xFFF7FBFF },
            StringAlignmentFar,
        );
    }
}

fn draw_settings_panel(
    graphics: *mut GpGraphics,
    width: f32,
    height: f32,
    model: &ViewModel,
    fonts: &FontSet,
    compact: bool,
    light: bool,
) {
    fill_rounded(
        graphics,
        3.0,
        3.0,
        width - 6.0,
        height - 6.0,
        SETTINGS_PANEL_RADIUS,
        if light { 0xF4E9EFF8 } else { 0xEB090C11 },
    );
    if compact {
        draw_compact(graphics, width, height, model, fonts, true);
    } else {
        draw_square(graphics, width, height, model, fonts, true);
    }
    let (panel_bg, border, title_color, btn_bg, btn_text, btn_active) = if light {
        (
            0xFFFFFFFF, 0x243F6195, 0xFF6C7F9D, 0xFFEEF3FB, 0xFF213B63, 0xFF7EA4E8,
        )
    } else {
        (
            0xFF161D29, 0x1EE9F0FF, 0xFF95A4BA, 0xFF202B3D, 0xFFF0F5FF, 0xFF34598D,
        )
    };
    for inset in [0.0, 1.0, 2.0] {
        let alpha = if light { 0x0C } else { 0x18 };
        fill_rounded(
            graphics,
            5.0 + inset,
            7.0 + inset,
            180.0 - inset * 2.0,
            176.0 - inset * 2.0,
            24.0,
            (alpha as u32) << 24,
        );
    }
    let card_origin = 3.0 + SETTINGS_PANEL_CARD_INSET;
    let card_width = width - card_origin * 2.0;
    let card_height = height - card_origin * 2.0;
    fill_rounded(
        graphics,
        card_origin,
        card_origin,
        card_width,
        card_height,
        SETTINGS_PANEL_CARD_RADIUS,
        panel_bg,
    );
    stroke_rounded(
        graphics,
        card_origin,
        card_origin,
        card_width,
        card_height,
        SETTINGS_PANEL_CARD_RADIUS,
        border,
        1.0,
    );
    text(
        graphics,
        fonts.small,
        model
            .settings
            .effective_language()
            .text("自动刷新", "AUTO REFRESH"),
        16.0,
        17.0,
        158.0,
        13.0,
        title_color,
        StringAlignmentNear,
    );
    for (i, minutes) in [1, 5, 15].into_iter().enumerate() {
        let x = 16.0 + i as f32 * 52.0;
        panel_button(
            graphics,
            fonts,
            x,
            31.0,
            48.0,
            24.0,
            &match model.settings.effective_language() {
                Language::Zh => format!("{minutes}分"),
                Language::En => format!("{minutes}m"),
            },
            model.settings.auto_refresh_enabled && model.settings.auto_refresh_minutes == minutes,
            model.hovered == Hit::Interval(minutes),
            btn_bg,
            btn_text,
            btn_active,
        );
    }
    text(
        graphics,
        fonts.small,
        model.settings.effective_language().text("显示", "DISPLAY"),
        16.0,
        60.0,
        158.0,
        13.0,
        title_color,
        StringAlignmentNear,
    );
    panel_button(
        graphics,
        fonts,
        16.0,
        74.0,
        73.0,
        24.0,
        model.settings.effective_language().text("剩余", "Left"),
        model.settings.display_mode == DisplayMode::Remaining,
        model.hovered == Hit::DisplayRemaining,
        btn_bg,
        btn_text,
        btn_active,
    );
    panel_button(
        graphics,
        fonts,
        94.0,
        74.0,
        73.0,
        24.0,
        model.settings.effective_language().text("已用", "Used"),
        model.settings.display_mode == DisplayMode::Used,
        model.hovered == Hit::DisplayUsed,
        btn_bg,
        btn_text,
        btn_active,
    );
    text(
        graphics,
        fonts.small,
        model.settings.effective_language().text("主题", "THEME"),
        16.0,
        103.0,
        158.0,
        13.0,
        title_color,
        StringAlignmentNear,
    );
    panel_button(
        graphics,
        fonts,
        16.0,
        117.0,
        73.0,
        24.0,
        model.settings.effective_language().text("浅色", "Light"),
        light,
        model.hovered == Hit::ThemeLight,
        btn_bg,
        btn_text,
        btn_active,
    );
    panel_button(
        graphics,
        fonts,
        94.0,
        117.0,
        73.0,
        24.0,
        model.settings.effective_language().text("深色", "Dark"),
        !light,
        model.hovered == Hit::ThemeDark,
        btn_bg,
        btn_text,
        btn_active,
    );
    panel_button(
        graphics,
        fonts,
        16.0,
        150.0,
        73.0,
        24.0,
        if model.settings.always_on_top {
            model
                .settings
                .effective_language()
                .text("取消置顶", "Unpin")
        } else {
            model.settings.effective_language().text("置顶", "Pin")
        },
        model.settings.always_on_top,
        model.hovered == Hit::AlwaysOnTop,
        btn_bg,
        btn_text,
        btn_active,
    );
    panel_button(
        graphics,
        fonts,
        94.0,
        150.0,
        73.0,
        24.0,
        model.settings.effective_language().text("关闭", "Close"),
        false,
        model.hovered == Hit::Close,
        btn_bg,
        btn_text,
        btn_active,
    );
}

fn draw_metric_card(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    label: &str,
    usage: Option<&super::UsageWindowSnapshot>,
    mode: DisplayMode,
    language: Language,
    light: bool,
    fonts: &FontSet,
) {
    let (card_top, card_bottom) = if light {
        (0xC8FFFFFF, 0xA9E7EFFC)
    } else {
        (0x16FFFFFF, 0x08FFFFFF)
    };
    let border = if light { 0x1F26497E } else { 0x14FFFFFF };
    let radius = 16.0;
    fill_gradient_rounded(graphics, x, y, width, height, radius, card_top, card_bottom);
    if let Some(percent) = displayed_percent(usage, mode) {
        let fill_height = (height - 2.0) * (percent as f32 / 100.0);
        let (fill_top, fill_bottom) = if mode == DisplayMode::Remaining {
            if light {
                (0x8279B3FF, 0x5A79B3FF)
            } else {
                (0x9A7EB9FF, 0x667EB9FF)
            }
        } else if light {
            (0x82FFA684, 0x5AFFA684)
        } else {
            (0x9FFF8A71, 0x66FF8A71)
        };
        let fy = if mode == DisplayMode::Remaining {
            y + height - fill_height
        } else {
            y + 1.0
        };
        fill_gradient_clipped_to_rounded(
            graphics,
            x + 1.0,
            y + 1.0,
            width - 2.0,
            height - 2.0,
            radius - 1.0,
            x + 1.0,
            fy,
            width - 2.0,
            fill_height,
            fill_top,
            fill_bottom,
        );
    }
    stroke_rounded(graphics, x, y, width, height, radius, border, 1.0);
    let label_color = if light { 0xFF6780A6 } else { 0xFF98A8C2 };
    let mode_color = if mode == DisplayMode::Remaining {
        if light {
            0xFF5574A3
        } else {
            0xFFB7C7DA
        }
    } else if light {
        0xFFB25745
    } else {
        0xFFFFD0C7
    };
    let main_color = if light { 0xFF15263D } else { 0xFFF7FBFF };
    let sub_color = if light { 0xFF58708F } else { 0xFFD4DDED };
    text(
        graphics,
        fonts.small,
        label,
        x + 8.0,
        y + 8.0,
        22.0,
        13.0,
        label_color,
        StringAlignmentNear,
    );
    text(
        graphics,
        fonts.small,
        if mode == DisplayMode::Remaining {
            language.text("剩余", "LEFT")
        } else {
            language.text("已用", "USED")
        },
        x + 31.0,
        y + 8.0,
        width - 39.0,
        13.0,
        mode_color,
        StringAlignmentFar,
    );
    let value = displayed_percent(usage, mode)
        .map_or_else(|| "--".to_string(), |value| format!("{value}%"));
    text(
        graphics,
        fonts.metric,
        &value,
        x + 3.0,
        y + 35.0,
        width - 6.0,
        37.0,
        main_color,
        StringAlignmentCenter,
    );
    text(
        graphics,
        fonts.small,
        &format_countdown(usage.and_then(|u| u.reset_after_seconds), language),
        x + 7.0,
        y + height - 24.0,
        width - 14.0,
        14.0,
        sub_color,
        StringAlignmentNear,
    );
}

fn draw_error_panel(
    graphics: *mut GpGraphics,
    snapshot: &UsageSnapshot,
    fonts: &FontSet,
    text_color: u32,
    light: bool,
    language: Language,
) {
    let bg = if light { 0xF2FFE2D0 } else { 0x14FFA86E };
    let border = if light { 0x33D97B5B } else { 0x1FFFA86E };
    fill_rounded(graphics, 13.0, 39.0, 164.0, 104.0, 16.0, bg);
    stroke_rounded(graphics, 13.0, 39.0, 164.0, 104.0, 16.0, border, 1.0);
    text(
        graphics,
        fonts.title,
        status_text(Some(&snapshot.status), language),
        23.0,
        57.0,
        144.0,
        18.0,
        if light { 0xFFA64D39 } else { 0xFFFFD3B9 },
        StringAlignmentNear,
    );
    text(
        graphics,
        fonts.small,
        usage_error_text(&snapshot.status, language),
        23.0,
        79.0,
        144.0,
        50.0,
        if light { 0xFF5F4D47 } else { 0xFFE7EDF6 },
        StringAlignmentNear,
    );
    let _ = text_color;
}

fn draw_refresh_button(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    refreshing: bool,
    theme: ThemeMode,
    hovered: bool,
) {
    let light = matches!(theme, ThemeMode::Light);
    let bg = if light {
        if hovered {
            0x292E5C9F
        } else {
            0x142E5C9F
        }
    } else if hovered {
        0x33FFFFFF
    } else {
        0x1FFFFFFF
    };
    let color = if light { 0xFF173055 } else { 0xFFF6F9FF };
    fill_rounded(graphics, x, y, 18.0, 18.0, 9.0, bg);
    if refreshing {
        draw_standalone_text(
            graphics,
            "...",
            x,
            y - 1.0,
            18.0,
            18.0,
            color,
            11.0,
            true,
            StringAlignmentCenter,
        );
    } else {
        let mut pen: *mut GpPen = null_mut();
        if unsafe { GdipCreatePen1(color, 1.2, UnitPixel, &mut pen) } == Status(0) && !pen.is_null()
        {
            unsafe {
                let _ = GdipDrawArc(graphics, pen, x + 5.5, y + 5.5, 7.0, 7.0, 0.0, 315.0);
                let _ = GdipDrawLine(graphics, pen, x + 10.8, y + 5.5, x + 12.5, y + 5.5);
                let _ = GdipDrawLine(graphics, pen, x + 12.5, y + 5.5, x + 12.5, y + 7.2);
                let _ = GdipDeletePen(pen);
            }
        }
    }
}

fn panel_button(
    graphics: *mut GpGraphics,
    fonts: &FontSet,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    label: &str,
    active: bool,
    hovered: bool,
    bg: u32,
    text_color: u32,
    active_color: u32,
) {
    fill_rounded(
        graphics,
        x,
        y,
        w,
        h,
        10.0,
        if active || hovered { active_color } else { bg },
    );
    text(
        graphics,
        fonts.bold_small,
        label,
        x + 2.0,
        y,
        w - 4.0,
        h,
        text_color,
        StringAlignmentCenter,
    );
}

fn fill_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: u32,
) {
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let mut path: *mut GpPath = null_mut();
    let status = unsafe { GdipCreatePath(FillModeAlternate, &mut path) };
    if status != Status(0) || path.is_null() {
        return;
    }
    let d = radius.min(width / 2.0).min(height / 2.0) * 2.0;
    unsafe {
        let _ = GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(path, x + width - d, y, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(path, x + width - d, y + height - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(path, x, y + height - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(path);
        let mut brush = null_mut();
        if GdipCreateSolidFill(color, &mut brush) == Status(0) && !brush.is_null() {
            let _ = GdipFillPath(graphics, brush.cast(), path);
            let _ = GdipDeleteBrush(brush.cast());
        }
        let _ = GdipDeletePath(path);
    }
}

fn fill_gradient_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    top_color: u32,
    bottom_color: u32,
) {
    let mut path: *mut GpPath = null_mut();
    let path_status = unsafe { GdipCreatePath(FillModeAlternate, &mut path) };
    if path_status != Status(0) || path.is_null() {
        return;
    }
    let diameter = radius.min(width / 2.0).min(height / 2.0) * 2.0;
    unsafe {
        let _ = GdipAddPathArc(path, x, y, diameter, diameter, 180.0, 90.0);
        let _ = GdipAddPathArc(
            path,
            x + width - diameter,
            y,
            diameter,
            diameter,
            270.0,
            90.0,
        );
        let _ = GdipAddPathArc(
            path,
            x + width - diameter,
            y + height - diameter,
            diameter,
            diameter,
            0.0,
            90.0,
        );
        let _ = GdipAddPathArc(
            path,
            x,
            y + height - diameter,
            diameter,
            diameter,
            90.0,
            90.0,
        );
        let _ = GdipClosePathFigure(path);
        let rect = RectF {
            X: x,
            Y: y,
            Width: width,
            Height: height,
        };
        let mut brush: *mut GpLineGradient = null_mut();
        let status = GdipCreateLineBrushFromRect(
            &rect,
            top_color,
            bottom_color,
            LinearGradientModeVertical,
            WrapModeTile,
            &mut brush,
        );
        if status == Status(0) && !brush.is_null() {
            let _ = GdipFillPath(graphics, brush.cast(), path);
            let _ = GdipDeleteBrush(brush.cast());
        } else {
            fill_rounded(graphics, x, y, width, height, radius, top_color);
        }
        let _ = GdipDeletePath(path);
    }
}

fn fill_gradient_clipped_to_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    fill_x: f32,
    fill_y: f32,
    fill_width: f32,
    fill_height: f32,
    top_color: u32,
    bottom_color: u32,
) {
    if fill_width <= 0.0 || fill_height <= 0.0 {
        return;
    }
    let mut path: *mut GpPath = null_mut();
    if unsafe { GdipCreatePath(FillModeAlternate, &mut path) } != Status(0) || path.is_null() {
        return;
    }
    let diameter = radius.min(width / 2.0).min(height / 2.0) * 2.0;
    let left = x;
    let top = y;
    let right = x + width;
    let bottom = y + height;
    unsafe {
        let _ = GdipAddPathArc(path, left, top, diameter, diameter, 180.0, 90.0);
        let _ = GdipAddPathArc(path, right - diameter, top, diameter, diameter, 270.0, 90.0);
        let _ = GdipAddPathArc(
            path,
            right - diameter,
            bottom - diameter,
            diameter,
            diameter,
            0.0,
            90.0,
        );
        let _ = GdipAddPathArc(
            path,
            left,
            bottom - diameter,
            diameter,
            diameter,
            90.0,
            90.0,
        );
        let _ = GdipClosePathFigure(path);
        if GdipSetClipRect(
            graphics,
            fill_x,
            fill_y,
            fill_width,
            fill_height,
            CombineModeReplace,
        ) == Status(0)
        {
            let rect = RectF {
                X: fill_x,
                Y: fill_y,
                Width: fill_width,
                Height: fill_height,
            };
            let mut brush: *mut GpLineGradient = null_mut();
            if GdipCreateLineBrushFromRect(
                &rect,
                top_color,
                bottom_color,
                LinearGradientModeVertical,
                WrapModeTile,
                &mut brush,
            ) == Status(0)
                && !brush.is_null()
            {
                let _ = GdipFillPath(graphics, brush.cast(), path);
                let _ = GdipDeleteBrush(brush.cast());
            }
            let _ = GdipResetClip(graphics);
        }
        let _ = GdipDeletePath(path);
    }
}

fn stroke_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: u32,
    stroke: f32,
) {
    let mut path: *mut GpPath = null_mut();
    if unsafe { GdipCreatePath(FillModeAlternate, &mut path) } != Status(0) || path.is_null() {
        return;
    }
    let d = radius.min(width / 2.0).min(height / 2.0) * 2.0;
    unsafe {
        let _ = GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(path, x + width - d, y, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(path, x + width - d, y + height - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(path, x, y + height - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(path);
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, stroke, UnitPixel, &mut pen) == Status(0) && !pen.is_null() {
            let _ = GdipDrawPath(graphics, pen, path);
            let _ = GdipDeletePen(pen);
        }
        let _ = GdipDeletePath(path);
    }
}

fn text(
    graphics: *mut GpGraphics,
    font: *mut GpFont,
    value: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: u32,
    alignment: StringAlignment,
) {
    if value.is_empty() {
        return;
    }
    let mut brush = null_mut();
    let mut format: *mut GpStringFormat = null_mut();
    let created_brush = unsafe { GdipCreateSolidFill(color, &mut brush) } == Status(0);
    let created_format = unsafe { GdipCreateStringFormat(0, 0x0409, &mut format) } == Status(0);
    if created_brush && created_format && !brush.is_null() && !format.is_null() {
        let wide: Vec<u16> = value.encode_utf16().collect();
        unsafe {
            let _ = GdipSetStringFormatFlags(format, STRING_FORMAT_FLAGS_NO_WRAP);
            let _ = GdipSetStringFormatAlign(format, alignment);
            let _ = GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
            let _ = GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter);
            let rect = RectF {
                X: x,
                Y: y,
                Width: width,
                Height: height,
            };
            let _ = GdipDrawString(
                graphics,
                PCWSTR(wide.as_ptr()),
                wide.len() as i32,
                font,
                &rect,
                format,
                brush.cast(),
            );
            let _ = GdipDeleteStringFormat(format);
            let _ = GdipDeleteBrush(brush.cast());
        }
    }
}

fn draw_standalone_text(
    graphics: *mut GpGraphics,
    value: &str,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color: u32,
    size: f32,
    bold: bool,
    alignment: StringAlignment,
) {
    let name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    let mut family: *mut GpFontFamily = null_mut();
    let mut font: *mut GpFont = null_mut();
    let mut brush = null_mut();
    unsafe {
        if GdipCreateFontFamilyFromName(PCWSTR(name.as_ptr()), null_mut(), &mut family) != Status(0)
        {
            return;
        }
        if GdipCreateFont(
            family,
            size,
            if bold {
                FontStyleBold.0
            } else {
                FontStyleRegular.0
            },
            UnitPixel,
            &mut font,
        ) != Status(0)
        {
            let _ = GdipDeleteFontFamily(family);
            return;
        }
        if GdipCreateSolidFill(color, &mut brush) != Status(0) {
            let _ = GdipDeleteFont(font);
            let _ = GdipDeleteFontFamily(family);
            return;
        }
    }
    text(graphics, font, value, x, y, width, height, color, alignment);
    unsafe {
        let _ = GdipDeleteBrush(brush.cast());
        let _ = GdipDeleteFont(font);
        let _ = GdipDeleteFontFamily(family);
    }
}

fn displayed_percent(usage: Option<&super::UsageWindowSnapshot>, mode: DisplayMode) -> Option<u8> {
    usage.map(|value| {
        if mode == DisplayMode::Remaining {
            value.remaining_percent
        } else {
            value.used_percent
        }
    })
}

fn format_countdown(seconds: Option<u64>, language: Language) -> String {
    match seconds {
        Some(value) => {
            let hours = value / 3600;
            let minutes = (value % 3600) / 60;
            if hours > 0 {
                match language {
                    Language::Zh => format!("{hours}时{minutes}分"),
                    Language::En => format!("{hours}h {minutes}m"),
                }
            } else {
                match language {
                    Language::Zh => format!("{}分", minutes.max(0)),
                    Language::En => format!("{}m", minutes.max(0)),
                }
            }
        }
        None => language.text("暂无数据", "No data").to_string(),
    }
}

fn reset_time(usage: Option<&super::UsageWindowSnapshot>) -> String {
    usage
        .and_then(|value| value.reset_at.as_deref())
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|time| time.with_timezone(&Local).format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".to_string())
}

fn reset_date(usage: Option<&super::UsageWindowSnapshot>) -> String {
    usage
        .and_then(|value| value.reset_at.as_deref())
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|time| time.with_timezone(&Local).format("%-m/%-d").to_string())
        .unwrap_or_else(|| "--/--".to_string())
}

fn shorten_account(value: Option<&str>) -> String {
    let Some(value) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return "Codex".to_string();
    };
    let local = value.split('@').next().unwrap_or(value);
    if local.chars().count() <= 12 {
        local.to_string()
    } else {
        format!(
            "{}..{}",
            local.chars().take(6).collect::<String>(),
            local
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        )
    }
}

fn plan_text(value: Option<&str>) -> String {
    value.unwrap_or("PLUS").to_uppercase()
}

fn status_text(value: Option<&UsageStatus>, language: Language) -> &'static str {
    match value {
        Some(UsageStatus::Ok) => language.text("已连接", "Connected"),
        Some(UsageStatus::NotLoggedIn) => language.text("未登录", "Not logged in"),
        Some(UsageStatus::TokenMissing) => language.text("缺少令牌", "Token missing"),
        Some(UsageStatus::RequestFailed) => language.text("请求失败", "Request failed"),
        Some(UsageStatus::UnsupportedResponse) => language.text("数据异常", "Bad response"),
        None => language.text("加载中", "Loading"),
    }
}

fn usage_error_text(status: &UsageStatus, language: Language) -> &'static str {
    match status {
        UsageStatus::NotLoggedIn => language.text("请先登录 Codex。", "Sign in to Codex."),
        UsageStatus::TokenMissing => language.text("请重新登录 Codex。", "Sign in again."),
        UsageStatus::RequestFailed => language.text("请稍后再刷新。", "Refresh later."),
        UsageStatus::UnsupportedResponse => language.text("额度数据异常。", "Invalid quota data."),
        UsageStatus::Ok => "",
    }
}

impl ViewModel {
    fn display_snapshot(&self) -> Option<&UsageSnapshot> {
        if matches!(
            self.snapshot.as_ref().map(|value| &value.status),
            Some(UsageStatus::RequestFailed)
        ) && self.last_good.is_some()
        {
            self.last_good.as_ref()
        } else {
            self.snapshot.as_ref()
        }
    }
    fn is_fallback(&self) -> bool {
        matches!(
            self.snapshot.as_ref().map(|value| &value.status),
            Some(UsageStatus::RequestFailed)
        ) && self.last_good.is_some()
    }
}

fn hit_at(x: f32, y: f32, width: f32, height: f32, panel_open: bool) -> Hit {
    let radius = if height <= 48.0 && !panel_open {
        width.min(height) / 2.0
    } else {
        SETTINGS_PANEL_RADIUS
    };
    let shell = (3.0, 3.0, width - 3.0, height - 3.0, radius);
    if !inside_rounded_rect(x, y, shell) {
        return Hit::Outside;
    }
    if panel_open {
        if (31.0..55.0).contains(&y) {
            for (i, minutes) in [1, 5, 15].into_iter().enumerate() {
                let left = 16.0 + i as f32 * 52.0;
                if (left..left + 48.0).contains(&x) {
                    return Hit::Interval(minutes);
                }
            }
        }
        if (74.0..98.0).contains(&y) {
            if (16.0..89.0).contains(&x) {
                return Hit::DisplayRemaining;
            }
            if (94.0..167.0).contains(&x) {
                return Hit::DisplayUsed;
            }
        }
        if (117.0..141.0).contains(&y) {
            if (16.0..89.0).contains(&x) {
                return Hit::ThemeLight;
            }
            if (94.0..167.0).contains(&x) {
                return Hit::ThemeDark;
            }
        }
        if (150.0..174.0).contains(&y) {
            if (16.0..89.0).contains(&x) {
                return Hit::AlwaysOnTop;
            }
            if (94.0..167.0).contains(&x) {
                return Hit::Close;
            }
        }
        return Hit::PanelOutside;
    }
    if height <= 48.0 && (8.0..32.0).contains(&y) {
        if (10.0..95.0).contains(&x) {
            return Hit::FiveHour;
        }
        if (100.0..180.0).contains(&x) {
            return Hit::Weekly;
        }
    }
    if height > 48.0 && (9.0..27.0).contains(&y) && (159.0..177.0).contains(&x) {
        return Hit::Refresh;
    }
    Hit::None
}

fn hit_at_padded_window(x: f32, y: f32, width: f32, height: f32, panel_open: bool) -> Hit {
    let padding = WINDOW_EFFECT_PADDING;
    if x < padding || y < padding || x >= width - padding || y >= height - padding {
        return Hit::Outside;
    }
    hit_at(
        x - padding,
        y - padding,
        width - padding * 2.0,
        height - padding * 2.0,
        panel_open,
    )
}

fn should_start_drag(hit: Hit, panel_open: bool) -> bool {
    hit == Hit::None && !panel_open
}

fn should_cancel_pending_drag(press_active: bool, pressed: Hit, current: Hit) -> bool {
    press_active && pressed == Hit::None && current != Hit::None
}

fn should_dispatch_click(pressed: Hit, released: Hit) -> bool {
    pressed == released && released != Hit::Outside && released != Hit::None
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
    _id: usize,
    ref_data: usize,
) -> windows::Win32::Foundation::LRESULT {
    let state_ptr = ref_data as *const NativeUiState;
    if state_ptr.is_null() {
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    if message == WM_STYLECHANGING {
        // Reject transient chrome before Windows applies the new style.
        let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        let changing = lparam.0 as *mut STYLESTRUCT;
        if !changing.is_null() {
            let changing = unsafe { &mut *changing };
            if wparam.0 as i32 == GWL_STYLE.0 {
                changing.styleNew &= !native_chrome_bits();
            } else if wparam.0 as i32 == GWL_EXSTYLE.0 {
                changing.styleNew = (changing.styleNew | WS_EX_LAYERED.0) & !WS_EX_WINDOWEDGE.0;
            }
        }
        return result;
    }
    let state = unsafe { &*state_ptr };
    if message == WM_SETCURSOR && should_set_client_cursor(lparam.0 as u16) {
        let mode = cursor_mode_from_value(state.cursor_mode.load(Ordering::Relaxed));
        if let Some(cursor) = cursor_handle(mode) {
            unsafe { SetCursor(Some(cursor)) };
            return windows::Win32::Foundation::LRESULT(1);
        }
    }
    if message == WM_NCHITTEST {
        return windows::Win32::Foundation::LRESULT(
            if hit_from_nchittest(hwnd, lparam, false) == Hit::Outside {
                HTTRANSPARENT
            } else {
                HTCLIENT as isize
            },
        );
    }
    // Window APIs used by these handlers can synchronously dispatch messages
    // back to this subclass procedure. Do not block the window thread trying
    // to reacquire its own state during that re-entrancy.
    let mut ui = match state.ui.try_lock() {
        Ok(value) => value,
        Err(_) => return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    };
    match message {
        WM_NATIVE_REDRAW => {
            if let Err(error) = ui.redraw(hwnd) {
                diagnostics::event("ERROR", "ui.redraw_failed", &error);
            }
            return windows::Win32::Foundation::LRESULT(0);
        }
        WM_NATIVE_OPEN_SETTINGS => {
            diagnostics::event(
                "INFO",
                "ui.settings.open",
                &format!("return_to_taskbar={}", wparam.0 != 0),
            );
            cancel_pending_drag(hwnd, &mut ui);
            set_cursor_mode(state, CursorMode::Arrow);
            ui.panel_open = true;
            ui.return_to_taskbar_after_panel = wparam.0 != 0;
            let settings = ui
                .app
                .state::<AppState>()
                .settings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            apply_runtime_window_shape(
                &ui.window,
                &settings.shape_mode,
                matches!(settings.shape_mode, ShapeMode::Compact),
            );
            if let Err(error) = ui.redraw(hwnd) {
                diagnostics::event("ERROR", "ui.settings_redraw_failed", &error);
            }
            return windows::Win32::Foundation::LRESULT(0);
        }
        WM_MOUSEMOVE => {
            if !ui.tracking_mouse {
                let mut tracking = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                if unsafe { TrackMouseEvent(&mut tracking) }.is_ok() {
                    ui.tracking_mouse = true;
                }
            }
            let hit = hit_from_message(hwnd, lparam, ui.panel_open);
            if should_cancel_pending_drag(ui.press_started.is_some(), ui.pressed, hit) {
                cancel_pending_drag(hwnd, &mut ui);
            }
            set_cursor_mode(state, cursor_mode_for_hit(hit));
            if hit != ui.hovered {
                ui.hovered = hit;
                let _ = ui.redraw(hwnd);
            }
        }
        WM_MOUSELEAVE => {
            ui.tracking_mouse = false;
            if ui.press_started.is_some() {
                cancel_pending_drag(hwnd, &mut ui);
            }
            ui.hovered = Hit::Outside;
            set_cursor_mode(state, CursorMode::Arrow);
            let _ = ui.redraw(hwnd);
        }
        WM_RBUTTONUP => {
            let settings = ui
                .app
                .state::<AppState>()
                .settings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            if hit_from_message(hwnd, lparam, ui.panel_open) != Hit::Outside
                && should_toggle_settings_on_right_click(settings.shape_mode)
            {
                cancel_pending_drag(hwnd, &mut ui);
                set_cursor_mode(state, CursorMode::Arrow);
                ui.panel_open = !ui.panel_open;
                ui.return_to_taskbar_after_panel = false;
                apply_runtime_window_shape(
                    &ui.window,
                    &settings.shape_mode,
                    ui.panel_open && matches!(settings.shape_mode, ShapeMode::Compact),
                );
                let _ = ui.redraw(hwnd);
            }
            return windows::Win32::Foundation::LRESULT(0);
        }
        WM_LBUTTONDOWN => {
            ui.pressed = hit_from_message(hwnd, lparam, ui.panel_open);
            set_cursor_mode(state, cursor_mode_for_hit(ui.pressed));
            if should_start_drag(ui.pressed, ui.panel_open) {
                ui.press_started = Some(Instant::now());
                let _ = unsafe {
                    SetTimer(
                        Some(hwnd),
                        DRAG_TIMER_ID,
                        DRAG_DELAY.as_millis() as u32,
                        None,
                    )
                };
            } else {
                let _ = unsafe { KillTimer(Some(hwnd), DRAG_TIMER_ID) };
                ui.press_started = None;
            }
        }
        WM_TIMER if wparam.0 == DRAG_TIMER_ID => {
            let _ = unsafe { KillTimer(Some(hwnd), DRAG_TIMER_ID) };
            if ui
                .press_started
                .map(|started| started.elapsed() >= DRAG_DELAY)
                .unwrap_or(false)
                && should_start_drag(ui.pressed, ui.panel_open)
            {
                set_cursor_mode(state, CursorMode::Arrow);
                let window = ui.window.clone();
                drop(ui);
                let _ = window.start_dragging();
                set_cursor_mode(state, CursorMode::Arrow);
                return windows::Win32::Foundation::LRESULT(0);
            }
        }
        WM_LBUTTONUP => {
            let _ = unsafe { KillTimer(Some(hwnd), DRAG_TIMER_ID) };
            let was_panel_open = ui.panel_open;
            ui.press_started = None;
            let hit = hit_from_message(hwnd, lparam, ui.panel_open);
            let pressed = ui.pressed;
            let same = hit == pressed;
            ui.pressed = Hit::Outside;
            set_cursor_mode(
                state,
                if was_panel_open && same {
                    CursorMode::Arrow
                } else {
                    cursor_mode_for_hit(hit)
                },
            );
            if should_dispatch_click(pressed, hit) {
                handle_hit(&mut ui, hit);
                if was_panel_open && !ui.panel_open {
                    set_cursor_mode(state, CursorMode::Arrow);
                }
            }
        }
        WM_LBUTTONDBLCLK => {
            let hit = hit_from_message(hwnd, lparam, ui.panel_open);
            set_cursor_mode(state, cursor_mode_for_hit(hit));
            if hit == Hit::None && !ui.panel_open {
                let settings = ui
                    .app
                    .state::<AppState>()
                    .settings
                    .lock()
                    .map(|value| value.clone())
                    .unwrap_or_default();
                let mut updated = settings.clone();
                updated.shape_mode = if matches!(settings.shape_mode, ShapeMode::Square) {
                    ShapeMode::Compact
                } else {
                    ShapeMode::Square
                };
                let shape_mode = updated.shape_mode;
                diagnostics::event("INFO", "ui.shape_toggle", &format!("to={shape_mode:?}"));
                if let Err(error) = set_settings(&ui.app, updated) {
                    diagnostics::event("ERROR", "ui.settings_save_failed", &error);
                }
                apply_runtime_window_shape(&ui.window, &shape_mode, false);
                let _ = ui.redraw(hwnd);
            }
            return windows::Win32::Foundation::LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 as u32 == 0x1B && ui.panel_open => {
            close_settings_panel(&mut ui);
            set_cursor_mode(state, CursorMode::Arrow);
            let _ = ui.redraw(hwnd);
            return windows::Win32::Foundation::LRESULT(0);
        }
        WM_KILLFOCUS if ui.return_to_taskbar_after_panel && ui.panel_open => {
            close_settings_panel(&mut ui);
            set_cursor_mode(state, CursorMode::Arrow);
            let _ = ui.redraw(hwnd);
        }
        WM_DPICHANGED => {
            let result = unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
            let _ = ui.redraw(hwnd);
            return result;
        }
        WM_NCDESTROY => {
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID) };
            drop(ui);
            unsafe {
                drop(Arc::from_raw(state_ptr));
            }
            return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
        }
        _ => {}
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn close_settings_panel(ui: &mut WindowUi) {
    diagnostics::event("INFO", "ui.settings.close", "");
    ui.panel_open = false;
    let settings = ui
        .app
        .state::<AppState>()
        .settings
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    let hide_after_close = should_hide_window_after_settings_panel(
        ui.return_to_taskbar_after_panel,
        settings.presentation_mode,
    );
    ui.return_to_taskbar_after_panel = false;
    apply_runtime_window_shape(&ui.window, &settings.shape_mode, false);
    if hide_after_close {
        let _ = ui.window.hide();
    }
}

fn should_set_client_cursor(hit_test: u16) -> bool {
    hit_test == HTCLIENT
}

fn hit_from_message(
    hwnd: HWND,
    lparam: windows::Win32::Foundation::LPARAM,
    panel_open: bool,
) -> Hit {
    let x = (lparam.0 as u16 as i16) as i32;
    let y = ((lparam.0 >> 16) as u16 as i16) as i32;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    let mut client = RECT::default();
    let _ = unsafe { GetClientRect(hwnd, &mut client) };
    hit_at_padded_window(
        x as f32 / dpi,
        y as f32 / dpi,
        (client.right - client.left) as f32 / dpi,
        (client.bottom - client.top) as f32 / dpi,
        panel_open,
    )
}

fn hit_from_nchittest(
    hwnd: HWND,
    lparam: windows::Win32::Foundation::LPARAM,
    panel_open: bool,
) -> Hit {
    let screen_x = (lparam.0 as u16 as i16) as i32;
    let screen_y = ((lparam.0 >> 16) as u16 as i16) as i32;
    let mut window = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut window) }.is_err() {
        return Hit::Outside;
    }
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return Hit::Outside;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    hit_at_padded_window(
        (screen_x - window.left) as f32 / dpi,
        (screen_y - window.top) as f32 / dpi,
        (client.right - client.left) as f32 / dpi,
        (client.bottom - client.top) as f32 / dpi,
        panel_open,
    )
}

fn cancel_pending_drag(hwnd: HWND, ui: &mut WindowUi) {
    let _ = unsafe { KillTimer(Some(hwnd), DRAG_TIMER_ID) };
    ui.press_started = None;
    ui.pressed = Hit::Outside;
}

fn handle_hit(ui: &mut WindowUi, hit: Hit) {
    if hit != Hit::None && hit != Hit::Outside {
        diagnostics::event("INFO", "ui.action", &format!("hit={hit:?}"));
    }
    match hit {
        Hit::Refresh => {
            let app = ui.app.clone();
            let state = app.state::<AppState>();
            state
                .refreshing
                .store(true, std::sync::atomic::Ordering::Relaxed);
            ui.renderer_redraw();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = super::refresh_and_publish(&app).await {
                    diagnostics::event("ERROR", "ui.refresh_failed", &error);
                }
                app.state::<AppState>()
                    .refreshing
                    .store(false, std::sync::atomic::Ordering::Relaxed);
                request_redraw(&app);
            });
        }
        Hit::FiveHour | Hit::Weekly => {
            let state = ui.app.state::<AppState>();
            let mut updated = state
                .settings
                .lock()
                .map(|settings| settings.clone())
                .unwrap_or_default();
            let show_reset =
                !(updated.compact_display.five_hour_time && updated.compact_display.weekly_date);
            updated.compact_display.five_hour_time = show_reset;
            updated.compact_display.weekly_date = show_reset;
            if let Err(error) = set_settings(&ui.app, updated) {
                diagnostics::event("ERROR", "ui.settings_save_failed", &error);
            }
        }
        Hit::Interval(minutes) => {
            let mut updated = ui
                .app
                .state::<AppState>()
                .settings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            updated.auto_refresh_enabled = true;
            updated.auto_refresh_minutes = minutes;
            if let Err(error) = set_settings(&ui.app, updated) {
                diagnostics::event("ERROR", "ui.settings_save_failed", &error);
            }
            close_settings_panel(ui);
        }
        Hit::DisplayRemaining | Hit::DisplayUsed => {
            let mut updated = ui
                .app
                .state::<AppState>()
                .settings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            updated.display_mode = if hit == Hit::DisplayUsed {
                DisplayMode::Used
            } else {
                DisplayMode::Remaining
            };
            if let Err(error) = set_settings(&ui.app, updated) {
                diagnostics::event("ERROR", "ui.settings_save_failed", &error);
            }
            close_settings_panel(ui);
        }
        Hit::ThemeLight | Hit::ThemeDark => {
            let mut updated = ui
                .app
                .state::<AppState>()
                .settings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            updated.theme_mode = if hit == Hit::ThemeLight {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            };
            if let Err(error) = set_settings(&ui.app, updated) {
                diagnostics::event("ERROR", "ui.settings_save_failed", &error);
            }
            close_settings_panel(ui);
        }
        Hit::AlwaysOnTop => {
            let mut updated = ui
                .app
                .state::<AppState>()
                .settings
                .lock()
                .map(|value| value.clone())
                .unwrap_or_default();
            updated.always_on_top = !updated.always_on_top;
            if let Err(error) = set_settings(&ui.app, updated) {
                diagnostics::event("ERROR", "ui.settings_save_failed", &error);
            }
            close_settings_panel(ui);
        }
        Hit::Close => {
            close_settings_panel(ui);
        }
        Hit::PanelOutside => {
            close_settings_panel(ui);
        }
        Hit::Outside | Hit::None => {}
    }
    let hwnd = ui.window.hwnd().ok();
    if let Some(hwnd) = hwnd {
        let _ = ui.redraw(hwnd);
    }
}

impl WindowUi {
    fn redraw(&mut self, hwnd: HWND) -> Result<(), String> {
        let app = self.app.clone();
        self.renderer
            .draw(hwnd, &app, self.panel_open, self.hovered)
    }

    fn renderer_redraw(&mut self) {
        if let Ok(hwnd) = self.window.hwnd() {
            let _ = self.redraw(hwnd);
        }
    }
}

fn set_settings(app: &AppHandle, updated: AppSettings) -> Result<(), String> {
    let state = app.state::<AppState>();
    let current = state
        .settings
        .lock()
        .map_err(|_| "failed to update app settings".to_string())?
        .clone();
    let updated = super::sanitize_settings(updated);
    let top_changed = updated.always_on_top != current.always_on_top;
    let top_enabled = updated.always_on_top;
    let log_detail = format!(
        "shape={:?} presentation={:?} always_on_top={} auto_refresh={} interval_min={}",
        updated.shape_mode,
        updated.presentation_mode,
        updated.always_on_top,
        updated.auto_refresh_enabled,
        updated.auto_refresh_minutes
    );
    if top_changed && updated.presentation_mode == PresentationMode::Floating {
        if let Some(window) = app.get_window("main") {
            window
                .set_always_on_top(updated.always_on_top)
                .map_err(|error| error.to_string())?;
        }
    }
    persist_settings(&updated)?;
    *state
        .settings
        .lock()
        .map_err(|_| "failed to persist app settings".to_string())? = updated;
    super::sync_tray_always_on_top(&state, top_enabled);
    let _ = write_taskbar_state(&state);
    request_redraw(app);
    diagnostics::event("INFO", "settings.update.finish", &log_detail);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_compact_panel_is_clipped_to_a_rounded_rectangle() {
        let mut pixels = vec![255; 190 * 190 * 4];
        mask_window_shell(&mut pixels, 190, 190, 1.0, false, true);
        let alpha_at = |x: usize, y: usize| pixels[(y * 190 + x) * 4 + 3];
        assert_eq!(alpha_at(7, 7), 0);
        assert_eq!(alpha_at(182, 7), 0);
        assert_eq!(alpha_at(20, 20), 255);
        assert!(alpha_at(9, 9) > 0 && alpha_at(9, 9) < 255);
        assert_eq!(alpha_at(95, 10), 255);
        assert_eq!(alpha_at(95, 95), 255);
        let mut partial = None;
        for y in 0..30 {
            for x in 0..30 {
                let alpha = alpha_at(x, y);
                if alpha > 0 && alpha < 255 {
                    partial = Some((x, y, alpha));
                    break;
                }
            }
            if partial.is_some() {
                break;
            }
        }
        let (x, y, alpha) = partial.expect("rounded panel corner should have fractional coverage");
        assert_eq!(alpha, alpha_at(189 - x, y));
        assert_eq!(alpha, alpha_at(x, 189 - y));
    }

    #[test]
    fn compact_shell_mask_antialiases_curve_and_preserves_symmetry() {
        let mut pixels = vec![255; 190 * 40 * 4];
        mask_window_shell(&mut pixels, 190, 40, 1.0, true, false);
        let alpha_at = |x: usize, y: usize| pixels[(y * 190 + x) * 4 + 3];

        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(95, 20), 255);
        assert!(alpha_at(15, 3) > 0 && alpha_at(15, 3) < 255);
        assert_eq!(alpha_at(15, 3), alpha_at(174, 3));
        assert_eq!(alpha_at(15, 3), alpha_at(15, 36));
        assert_eq!(alpha_at(15, 3), alpha_at(174, 36));

        let mut finer_subpixel_level = false;
        for y in 0..40 {
            for x in 0..190 {
                let alpha = alpha_at(x, y);
                if alpha > 0 && alpha < 255 && alpha % 16 != 0 {
                    finer_subpixel_level = true;
                    break;
                }
            }
            if finer_subpixel_level {
                break;
            }
        }
        assert!(
            finer_subpixel_level,
            "8x supersampling should produce smoother coverage steps than 4x"
        );
    }

    #[test]
    fn compact_shell_mask_keeps_fractional_edges_at_higher_dpi() {
        let (width, height) = (285, 60);
        let mut pixels = vec![255; width as usize * height as usize * 4];
        mask_window_shell(&mut pixels, width, height, 1.5, true, false);
        let alpha_at = |x: usize, y: usize| pixels[(y * width as usize + x) * 4 + 3];

        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(width as usize / 2, height as usize / 2), 255);
        let mut partial = None;
        for y in 0..height as usize / 2 {
            for x in 0..width as usize / 2 {
                let alpha = alpha_at(x, y);
                if alpha > 0 && alpha < 255 {
                    partial = Some((x, y, alpha));
                    break;
                }
            }
            if partial.is_some() {
                break;
            }
        }
        let (x, y, alpha) = partial.expect("higher-DPI curve should have fractional coverage");
        assert_eq!(alpha, alpha_at(width as usize - 1 - x, y));
        assert_eq!(alpha, alpha_at(x, height as usize - 1 - y));
    }

    #[test]
    fn square_shell_mask_antialiases_outer_corners_symmetrically() {
        let mut pixels = vec![255; 190 * 190 * 4];
        mask_window_shell(&mut pixels, 190, 190, 1.0, false, false);
        let alpha_at = |x: usize, y: usize| pixels[(y * 190 + x) * 4 + 3];
        assert_eq!(alpha_at(0, 0), 0);
        assert_eq!(alpha_at(95, 95), 255);
        let mut partial = None;
        for y in 0..40 {
            for x in 0..40 {
                let alpha = alpha_at(x, y);
                if alpha > 0 && alpha < 255 {
                    partial = Some((x, y, alpha));
                    break;
                }
            }
            if partial.is_some() {
                break;
            }
        }
        let (x, y, alpha) = partial.expect("square shell corner should be antialiased");
        assert_eq!(alpha, alpha_at(189 - x, y));
        assert_eq!(alpha, alpha_at(x, 189 - y));
    }

    #[test]
    fn right_click_opens_settings_for_both_shapes() {
        assert!(should_toggle_settings_on_right_click(ShapeMode::Square));
        assert!(should_toggle_settings_on_right_click(ShapeMode::Compact));
    }

    #[test]
    fn temporary_settings_window_hides_only_while_taskbar_mode_remains_active() {
        assert!(should_hide_window_after_settings_panel(
            true,
            PresentationMode::Taskbar
        ));
        assert!(!should_hide_window_after_settings_panel(
            true,
            PresentationMode::Floating
        ));
        assert!(!should_hide_window_after_settings_panel(
            false,
            PresentationMode::Taskbar
        ));
    }

    #[test]
    fn compact_window_hit_zones_match_metric_rows() {
        assert_eq!(hit_at(20.0, 16.0, 190.0, 40.0, false), Hit::FiveHour);
        assert_eq!(hit_at(120.0, 16.0, 190.0, 40.0, false), Hit::Weekly);
        assert_eq!(hit_at(170.0, 16.0, 190.0, 40.0, false), Hit::Weekly);
        assert_eq!(hit_at(98.0, 16.0, 190.0, 40.0, false), Hit::None);
        assert_eq!(hit_at(0.0, 0.0, 190.0, 40.0, false), Hit::Outside);
        assert_eq!(hit_at(95.0, 16.0, 190.0, 40.0, false), Hit::None);
    }

    #[test]
    fn effect_padding_maps_hits_to_content_and_rejects_the_outer_margin() {
        assert_eq!(
            hit_at_padded_window(30.0, 26.0, 210.0, 60.0, false),
            Hit::FiveHour
        );
        assert_eq!(
            hit_at_padded_window(130.0, 26.0, 210.0, 60.0, false),
            Hit::Weekly
        );
        assert_eq!(
            hit_at_padded_window(105.0, 26.0, 210.0, 60.0, false),
            Hit::None
        );
        assert_eq!(
            hit_at_padded_window(5.0, 30.0, 210.0, 60.0, false),
            Hit::Outside
        );
        assert_eq!(
            hit_at_padded_window(205.0, 30.0, 210.0, 60.0, false),
            Hit::Outside
        );
    }

    #[test]
    fn cursor_shape_matches_clickable_draggable_and_transparent_areas() {
        assert_eq!(cursor_mode_for_hit(Hit::Refresh), CursorMode::Click);
        assert_eq!(cursor_mode_for_hit(Hit::FiveHour), CursorMode::Click);
        assert_eq!(cursor_mode_for_hit(Hit::PanelOutside), CursorMode::Click);
        assert_eq!(cursor_mode_for_hit(Hit::None), CursorMode::Arrow);
        assert_eq!(cursor_mode_for_hit(Hit::Outside), CursorMode::Arrow);
        assert!(should_set_client_cursor(HTCLIENT));
        assert!(!should_set_client_cursor(0));
        assert!(!should_set_client_cursor(2));
    }

    #[test]
    fn drag_and_click_gestures_only_complete_in_their_original_regions() {
        assert!(should_start_drag(Hit::None, false));
        assert!(!should_start_drag(Hit::None, true));
        assert!(!should_start_drag(Hit::Refresh, false));
        assert!(!should_start_drag(Hit::Outside, false));

        assert!(!should_cancel_pending_drag(false, Hit::None, Hit::Refresh));
        assert!(!should_cancel_pending_drag(true, Hit::None, Hit::None));
        assert!(should_cancel_pending_drag(true, Hit::None, Hit::FiveHour));
        assert!(should_cancel_pending_drag(true, Hit::None, Hit::Outside));
        assert!(!should_cancel_pending_drag(
            true,
            Hit::Refresh,
            Hit::Outside
        ));

        assert!(should_dispatch_click(Hit::Refresh, Hit::Refresh));
        assert!(should_dispatch_click(Hit::PanelOutside, Hit::PanelOutside));
        assert!(!should_dispatch_click(Hit::Refresh, Hit::Outside));
        assert!(!should_dispatch_click(Hit::None, Hit::None));
        assert!(!should_dispatch_click(Hit::Outside, Hit::Outside));
    }

    #[test]
    fn settings_panel_hit_zones_cover_every_control() {
        assert_eq!(hit_at(20.0, 40.0, 190.0, 190.0, true), Hit::Interval(1));
        assert_eq!(hit_at(70.0, 40.0, 190.0, 190.0, true), Hit::Interval(5));
        assert_eq!(hit_at(125.0, 40.0, 190.0, 190.0, true), Hit::Interval(15));
        assert_eq!(
            hit_at(20.0, 85.0, 190.0, 190.0, true),
            Hit::DisplayRemaining
        );
        assert_eq!(hit_at(110.0, 85.0, 190.0, 190.0, true), Hit::DisplayUsed);
        assert_eq!(hit_at(20.0, 128.0, 190.0, 190.0, true), Hit::ThemeLight);
        assert_eq!(hit_at(110.0, 128.0, 190.0, 190.0, true), Hit::ThemeDark);
        assert_eq!(hit_at(20.0, 162.0, 190.0, 190.0, true), Hit::AlwaysOnTop);
        assert_eq!(hit_at(110.0, 162.0, 190.0, 190.0, true), Hit::Close);
        assert_eq!(hit_at(80.0, 65.0, 190.0, 190.0, true), Hit::PanelOutside);
        assert_eq!(hit_at(180.0, 180.0, 190.0, 190.0, true), Hit::PanelOutside);
        assert_eq!(hit_at(0.0, 0.0, 190.0, 190.0, true), Hit::Outside);
    }

    #[test]
    fn square_window_keeps_only_the_refresh_button_clickable() {
        assert_eq!(hit_at(168.0, 18.0, 190.0, 190.0, false), Hit::Refresh);
        assert_eq!(hit_at(20.0, 16.0, 190.0, 190.0, false), Hit::None);
        assert_eq!(hit_at(0.0, 0.0, 190.0, 190.0, false), Hit::Outside);
    }

    #[test]
    fn themed_edge_effects_fit_inside_the_expanded_window() {
        for (width, height, content_width, content_height, compact, panel_open) in [
            (210, 60, 190.0, 40.0, true, false),
            (210, 210, 190.0, 190.0, false, false),
            (210, 210, 190.0, 190.0, true, true),
        ] {
            for light in [false, true] {
                let mut pixels = vec![0_u8; width * height * 4];
                draw_edge_effect(
                    &mut pixels,
                    width as u32,
                    height as u32,
                    1.0,
                    content_width,
                    content_height,
                    compact,
                    panel_open,
                    light,
                );
                let pixel =
                    |x: usize, y: usize| &pixels[(y * width + x) * 4..(y * width + x) * 4 + 4];
                let outer_glow = pixel(width / 2, 10);
                let inner_rim = pixel(width / 2, 14);
                let border = pixel(width / 2, 12);
                let canvas_edge = pixel(width / 2, 0);
                let center = pixel(width / 2, height / 2);
                assert!(
                    outer_glow[3] > 0,
                    "theme edge should fade through the new transparent padding"
                );
                assert!(
                    inner_rim[3] > 0,
                    "theme edge should fade toward the shell interior"
                );
                assert_eq!(canvas_edge[3], 0, "glow must not touch the bitmap edge");
                assert_eq!(center[3], 0, "theme edge must not cover the window content");
                if light {
                    assert!(outer_glow[0] > outer_glow[1] && outer_glow[1] > outer_glow[2]);
                } else {
                    assert!(outer_glow[0] > outer_glow[1] && outer_glow[1] > outer_glow[2]);
                    assert!(
                        inner_rim[1] > inner_rim[2],
                        "dark-theme rim should shift toward neon cyan"
                    );
                    assert_eq!(border[3], 255, "dark outline should be a solid 1px line");
                    assert!(
                        border[0] > border[1] && border[2] > border[1],
                        "dark outline should be bright purple"
                    );
                }
            }
        }
    }

    #[test]
    fn padded_content_is_centered_without_painting_the_effect_margin() {
        let content = vec![
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ];
        let canvas = compose_padded_content(&content, 2, 4, 6, 8, 2);
        let pixel_at = |x: usize, y: usize| {
            let start = (y * 6 + x) * 4;
            &canvas[start..start + 4]
        };
        assert_eq!(pixel_at(0, 0), &[0, 0, 0, 0]);
        assert_eq!(pixel_at(2, 2), &[1, 2, 3, 4]);
        assert_eq!(pixel_at(3, 5), &[29, 30, 31, 32]);
        assert_eq!(pixel_at(5, 7), &[0, 0, 0, 0]);
    }

    #[test]
    fn compact_metric_fields_fit_full_percent_and_reset_date_on_one_line() {
        assert_eq!(STRING_FORMAT_FLAGS_NO_WRAP, 0x0000_1000);
        assert_eq!(
            COMPACT_VALUE_INSET + COMPACT_VALUE_WIDTH,
            COMPACT_PERCENT_INSET
        );
        assert_eq!(COMPACT_PERCENT_INSET + COMPACT_PERCENT_WIDTH, 76.0);

        ensure_gdiplus().expect("GDI+ should initialize for compact text measurement");
        unsafe {
            let family_name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
            let mut family = null_mut();
            assert_eq!(
                GdipCreateFontFamilyFromName(PCWSTR(family_name.as_ptr()), null_mut(), &mut family),
                Status(0)
            );

            let mut font = null_mut();
            assert_eq!(
                GdipCreateFont(family, 11.0, FontStyleBold.0, UnitPixel, &mut font),
                Status(0)
            );

            let mut bitmap = null_mut();
            assert_eq!(
                GdipCreateBitmapFromScan0(1, 1, 0, PIXEL_FORMAT_32BPP_PARGB, None, &mut bitmap),
                Status(0)
            );
            let mut graphics = null_mut();
            assert_eq!(
                GdipGetImageGraphicsContext(bitmap.cast(), &mut graphics),
                Status(0)
            );

            let mut format = null_mut();
            assert_eq!(GdipCreateStringFormat(0, 0x0409, &mut format), Status(0));
            assert_eq!(
                GdipSetStringFormatFlags(format, STRING_FORMAT_FLAGS_NO_WRAP),
                Status(0)
            );
            assert_eq!(
                GdipSetStringFormatTrimming(format, StringTrimmingEllipsisCharacter),
                Status(0)
            );

            let measure_width = |value: &str| {
                let wide: Vec<u16> = value.encode_utf16().collect();
                let layout = RectF {
                    X: 0.0,
                    Y: 0.0,
                    Width: 100.0,
                    Height: 24.0,
                };
                let mut bounds = RectF::default();
                let mut codepoints_fitted = 0;
                let mut lines_filled = 0;
                assert_eq!(
                    GdipMeasureString(
                        graphics,
                        PCWSTR(wide.as_ptr()),
                        wide.len() as i32,
                        font,
                        &layout,
                        format,
                        &mut bounds,
                        &mut codepoints_fitted,
                        &mut lines_filled,
                    ),
                    Status(0)
                );
                assert_eq!(codepoints_fitted, wide.len() as i32);
                assert_eq!(lines_filled, 1);
                bounds.Width
            };
            let percent_width = measure_width("100%");
            let reset_date_width = measure_width("10/22");
            assert!(
                percent_width <= COMPACT_PERCENT_WIDTH,
                "100% width {percent_width} must fit the {}px value column",
                COMPACT_PERCENT_WIDTH
            );
            assert!(
                reset_date_width <= COMPACT_VALUE_WIDTH,
                "reset date width {reset_date_width} must fit the {}px label column",
                COMPACT_VALUE_WIDTH
            );

            let _ = GdipDeleteStringFormat(format);
            let _ = GdipDeleteGraphics(graphics);
            let _ = GdipDisposeImage(bitmap.cast());
            let _ = GdipDeleteFont(font);
            let _ = GdipDeleteFontFamily(family);
        }
    }

    #[test]
    fn rounded_shell_distance_distinguishes_inside_edge_and_margin() {
        let shell = (3.0, 3.0, 187.0, 37.0, 20.0);
        assert_eq!(rounded_rect_signed_distance(95.0, 3.0, shell), 0.0);
        assert!(rounded_rect_signed_distance(95.0, 0.5, shell) > 0.0);
        assert!(rounded_rect_signed_distance(95.0, 5.0, shell) < 0.0);
    }

}
