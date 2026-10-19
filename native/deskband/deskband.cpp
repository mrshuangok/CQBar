#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <shobjidl.h>
#include <shlobj.h>
#include <strsafe.h>
#include <gdiplus.h>
#include <atomic>
#include <algorithm>
#include <cstring>
#include <ctime>
#include <new>
#include <string>

namespace {
const CLSID kBandId = {0x4a7b6d93, 0x4829, 0x4ce3, {0x97, 0x7e, 0xd2, 0x51, 0x94, 0x34, 0x8a, 0x6a}};
constexpr wchar_t kBandIdText[] = L"{4A7B6D93-4829-4CE3-977E-D25194348A6A}";
constexpr wchar_t kBandClass[] = L"CodexQuotaDeskBandWindow";
constexpr wchar_t kHoverClass[] = L"CodexQuotaDeskBandHover";
constexpr UINT kRefreshDataMessage = WM_APP + 42;
constexpr UINT_PTR kRefreshTimer = 1;
constexpr UINT_PTR kHoverTimer = 2;
std::atomic<long> g_objects{0};
std::atomic<long> g_locks{0};
HINSTANCE g_instance = nullptr;

HRESULT set_string(HKEY key, const wchar_t* name, const wchar_t* value) {
  return HRESULT_FROM_WIN32(RegSetValueExW(
      key, name, 0, REG_SZ, reinterpret_cast<const BYTE*>(value),
      static_cast<DWORD>((wcslen(value) + 1) * sizeof(wchar_t))));
}

std::wstring state_path() {
  wchar_t* local = nullptr;
  if (FAILED(SHGetKnownFolderPath(FOLDERID_LocalAppData, 0, nullptr, &local))) return {};
  std::wstring path(local);
  CoTaskMemFree(local);
  return path + L"\\codex-quota-float\\taskbar-state.ini";
}

std::wstring reset_text(unsigned int timestamp) {
  if (!timestamp) return L"--";
  time_t value = static_cast<time_t>(timestamp);
  tm local{};
  if (localtime_s(&local, &value)) return L"--";
  wchar_t buffer[32]{};
  wcsftime(buffer, ARRAYSIZE(buffer), L"%Y-%m-%d %H:%M", &local);
  return buffer;
}

std::wstring reset_time_label(int timestamp) {
  if (timestamp <= 0) return L"--:--";
  time_t value = static_cast<time_t>(timestamp);
  tm local{};
  if (localtime_s(&local, &value)) return L"--:--";
  wchar_t buffer[8]{};
  StringCchPrintfW(buffer, ARRAYSIZE(buffer), L"%02d:%02d", local.tm_hour, local.tm_min);
  return buffer;
}

std::wstring reset_date_label(int timestamp) {
  if (timestamp <= 0) return L"--/--";
  time_t value = static_cast<time_t>(timestamp);
  tm local{};
  if (localtime_s(&local, &value)) return L"--/--";
  return std::to_wstring(local.tm_mon + 1) + L"/" + std::to_wstring(local.tm_mday);
}

SIZE text_size(HDC dc, const wchar_t* text) {
  SIZE size{};
  GetTextExtentPoint32W(dc, text, static_cast<int>(wcslen(text)), &size);
  return size;
}

UINT band_dpi(HWND window, HDC dc) {
  using DpiFn = UINT(WINAPI*)(HWND);
  auto* get_dpi = reinterpret_cast<DpiFn>(GetProcAddress(GetModuleHandleW(L"user32.dll"), "GetDpiForWindow"));
  UINT dpi = window && get_dpi ? get_dpi(window) : 0;
  if (!dpi) dpi = static_cast<UINT>(GetDeviceCaps(dc, LOGPIXELSY));
  return dpi ? dpi : 96;
}

struct BandLayout {
  HFONT font = nullptr;
  HGDIOBJ previous_font = nullptr;
  int left = 4;
  int value_left = 61;
  int right_padding = 3;
  int width = 106;
  int height = 40;
  int row_height = 20;
};

BandLayout begin_band_layout(HDC dc, HWND window) {
  BandLayout layout;
  if (!dc) return layout;
  UINT dpi = band_dpi(window, dc);
  layout.left = MulDiv(4, dpi, 96);
  layout.right_padding = MulDiv(3, dpi, 96);
  layout.row_height = MulDiv(16, dpi, 96);
  layout.height = MulDiv(40, dpi, 96);
  int old_label_width = std::max(text_size(dc, L"五小时").cx, text_size(dc, L"周额度").cx);
  int old_gap = MulDiv(103, dpi, 96) - layout.left - old_label_width - text_size(dc, L"100%").cx;
  int previous_gap = std::max(MulDiv(4, dpi, 96), old_gap / 2);
  // Measure the previous text style to preserve its side padding before adding 5px.
  HFONT previous_font = CreateFontW(-MulDiv(11, dpi, 96), 0, 0, 0, FW_BOLD, FALSE, FALSE, FALSE,
                                    DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                    CLEARTYPE_QUALITY, FIXED_PITCH | FF_MODERN, L"Consolas");
  if (previous_font) {
    HGDIOBJ original_font = SelectObject(dc, previous_font);
    int previous_label_width = std::max({text_size(dc, L"23:59").cx, text_size(dc, L"12/31").cx,
                                         text_size(dc, L"--:--").cx, text_size(dc, L"--/--").cx});
    layout.width = layout.left + previous_label_width + previous_gap +
                   text_size(dc, L"100%").cx + layout.right_padding;
    SelectObject(dc, original_font);
    DeleteObject(previous_font);
  }
  layout.font = CreateFontW(-MulDiv(12, dpi, 96), 0, 0, 0, FW_BOLD, FALSE, FALSE, FALSE,
                            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                            CLEARTYPE_QUALITY, FIXED_PITCH | FF_MODERN, L"Consolas");
  if (layout.font) layout.previous_font = SelectObject(dc, layout.font);
  int label_width = std::max({text_size(dc, L"23:59").cx, text_size(dc, L"12/31").cx,
                              text_size(dc, L"--:--").cx, text_size(dc, L"--/--").cx});
  int value_width = text_size(dc, L"100%").cx;
  int gap = std::min(previous_gap / 2, std::max(0, layout.width - label_width - value_width));
  int padding = std::max(0, layout.width - label_width - gap - value_width);
  int previous_left = (padding + 1) / 2;
  int added_padding = MulDiv(5, dpi, 96);
  layout.left = previous_left + added_padding;
  layout.right_padding = padding - previous_left + added_padding;
  layout.value_left = layout.left + label_width;
  layout.width = layout.value_left + value_width + layout.right_padding;
  return layout;
}

void end_band_layout(HDC dc, const BandLayout& layout) {
  if (layout.previous_font) SelectObject(dc, layout.previous_font);
  if (layout.font) DeleteObject(layout.font);
}

std::wstring percent_text(int value) {
  if (value < 0 || value > 100) return L"--";
  return std::to_wstring(value) + L"%";
}

void rounded_path(Gdiplus::GraphicsPath& path, float x, float y, float width, float height,
                  float radius) {
  float diameter = radius * 2.0f;
  path.AddArc(x, y, diameter, diameter, 180.0f, 90.0f);
  path.AddArc(x + width - diameter, y, diameter, diameter, 270.0f, 90.0f);
  path.AddArc(x + width - diameter, y + height - diameter, diameter, diameter, 0.0f, 90.0f);
  path.AddArc(x, y + height - diameter, diameter, diameter, 90.0f, 90.0f);
  path.CloseFigure();
}

class DeskBand final : public IDeskBand2, public IObjectWithSite, public IPersistStream {
 public:
  DeskBand() {
    Gdiplus::GdiplusStartupInput input;
    gdiplus_ready_ = Gdiplus::GdiplusStartup(&gdiplus_token_, &input, nullptr) == Gdiplus::Ok;
    ++g_objects;
  }
  ~DeskBand() {
    SetSite(nullptr);
    if (gdiplus_ready_) Gdiplus::GdiplusShutdown(gdiplus_token_);
    --g_objects;
  }
  IFACEMETHODIMP QueryInterface(REFIID iid, void** out) override {
    if (!out) return E_POINTER;
    *out = nullptr;
    if (iid == IID_IUnknown || iid == IID_IOleWindow || iid == IID_IDockingWindow ||
        iid == IID_IDeskBand || iid == IID_IDeskBand2) {
      *out = static_cast<IDeskBand2*>(this);
    } else if (iid == IID_IObjectWithSite) {
      *out = static_cast<IObjectWithSite*>(this);
    } else if (iid == IID_IPersist || iid == IID_IPersistStream) {
      *out = static_cast<IPersistStream*>(this);
    }
    if (!*out) return E_NOINTERFACE;
    AddRef();
    return S_OK;
  }
  IFACEMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
  IFACEMETHODIMP_(ULONG) Release() override {
    ULONG remaining = --refs_;
    if (!remaining) delete this;
    return remaining;
  }
  IFACEMETHODIMP GetWindow(HWND* out) override {
    if (!out) return E_POINTER;
    *out = window_;
    return window_ ? S_OK : E_FAIL;
  }
  IFACEMETHODIMP ContextSensitiveHelp(BOOL) override { return E_NOTIMPL; }
  IFACEMETHODIMP ShowDW(BOOL show) override {
    if (window_) ShowWindow(window_, show ? SW_SHOW : SW_HIDE);
    if (!show) {
      hovering_ = false;
      if (window_) KillTimer(window_, kHoverTimer);
      if (hover_window_) ShowWindow(hover_window_, SW_HIDE);
    }
    return S_OK;
  }
  IFACEMETHODIMP CloseDW(DWORD) override {
    if (hover_window_) DestroyWindow(hover_window_);
    hover_window_ = nullptr;
    if (window_) DestroyWindow(window_);
    window_ = nullptr;
    return S_OK;
  }
  IFACEMETHODIMP ResizeBorderDW(const RECT*, IUnknown*, BOOL) override { return E_NOTIMPL; }
  IFACEMETHODIMP GetBandInfo(DWORD id, DWORD, DESKBANDINFO* info) override {
    if (!info) return E_POINTER;
    HWND target = window_ ? window_ : GetDesktopWindow();
    HDC dc = GetDC(target);
    BandLayout layout = begin_band_layout(dc, target);
    end_band_layout(dc, layout);
    ReleaseDC(target, dc);
    if (info->dwMask & DBIM_MINSIZE) info->ptMinSize = {layout.width, layout.height};
    if (info->dwMask & DBIM_MAXSIZE) info->ptMaxSize.y = -1;
    if (info->dwMask & DBIM_INTEGRAL) info->ptIntegral.y = 1;
    if (info->dwMask & DBIM_ACTUAL) info->ptActual = {layout.width, layout.height};
    if (info->dwMask & DBIM_TITLE) info->dwMask &= ~DBIM_TITLE;
    if (info->dwMask & DBIM_MODEFLAGS) info->dwModeFlags = DBIMF_NORMAL | DBIMF_VARIABLEHEIGHT;
    if (info->dwMask & DBIM_BKCOLOR) info->dwMask &= ~DBIM_BKCOLOR;
    return S_OK;
  }
  IFACEMETHODIMP CanRenderComposited(BOOL* out) override {
    if (!out) return E_POINTER;
    *out = TRUE;
    return S_OK;
  }
  IFACEMETHODIMP SetCompositionState(BOOL enabled) override {
    composited_ = enabled;
    if (window_) InvalidateRect(window_, nullptr, TRUE);
    return S_OK;
  }
  IFACEMETHODIMP GetCompositionState(BOOL* out) override {
    if (!out) return E_POINTER;
    *out = composited_;
    return S_OK;
  }
  IFACEMETHODIMP SetSite(IUnknown* site) override {
    if (hover_window_) DestroyWindow(hover_window_);
    hover_window_ = nullptr;
    if (window_) DestroyWindow(window_);
    window_ = nullptr;
    if (site_) site_->Release();
    site_ = nullptr;
    if (!site) return S_OK;
    IOleWindow* host = nullptr;
    HRESULT hr = site->QueryInterface(IID_PPV_ARGS(&host));
    if (FAILED(hr)) return hr;
    HWND parent = nullptr;
    hr = host->GetWindow(&parent);
    host->Release();
    if (FAILED(hr) || !parent) return E_FAIL;
    site_ = site;
    site_->AddRef();
    WNDCLASSW cls{};
    cls.lpfnWndProc = WndProc;
    cls.hInstance = g_instance;
    cls.lpszClassName = kBandClass;
    cls.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    RegisterClassW(&cls);
    WNDCLASSW hover_class{};
    hover_class.lpfnWndProc = HoverWndProc;
    hover_class.hInstance = g_instance;
    hover_class.lpszClassName = kHoverClass;
    RegisterClassW(&hover_class);
    HDC dc = GetDC(parent);
    BandLayout layout = begin_band_layout(dc, parent);
    end_band_layout(dc, layout);
    ReleaseDC(parent, dc);
    window_ = CreateWindowExW(0, kBandClass, L"", WS_CHILD | WS_CLIPCHILDREN,
                              0, 0, layout.width, layout.height, parent, nullptr, g_instance, this);
    if (!window_) {
      site_->Release();
      site_ = nullptr;
      return HRESULT_FROM_WIN32(GetLastError());
    }
    hover_window_ = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE |
                                        WS_EX_TRANSPARENT | WS_EX_LAYERED,
                                    kHoverClass, L"", WS_POPUP, 0, 0, 0, 0,
                                    window_, nullptr, g_instance, this);
    SetTimer(window_, kRefreshTimer, 5000, nullptr);
    RefreshData();
    return S_OK;
  }
  IFACEMETHODIMP GetSite(REFIID iid, void** out) override {
    if (!out) return E_POINTER;
    *out = nullptr;
    return site_ ? site_->QueryInterface(iid, out) : E_FAIL;
  }
  IFACEMETHODIMP GetClassID(CLSID* out) override {
    if (!out) return E_POINTER;
    *out = kBandId;
    return S_OK;
  }
  IFACEMETHODIMP IsDirty() override { return S_FALSE; }
  IFACEMETHODIMP Load(IStream*) override { return S_OK; }
  IFACEMETHODIMP Save(IStream*, BOOL) override { return S_OK; }
  IFACEMETHODIMP GetSizeMax(ULARGE_INTEGER* out) override {
    if (!out) return E_POINTER;
    out->QuadPart = 0;
    return S_OK;
  }
 private:
  void RefreshData() {
    const std::wstring path = state_path();
    int process = path.empty() ? 0 : GetPrivateProfileIntW(L"quota", L"pid", 0, path.c_str());
    int heartbeat = path.empty() ? 0 : GetPrivateProfileIntW(L"quota", L"heartbeat", 0, path.c_str());
    const time_t now = time(nullptr);
    bool fresh = process > 0 && heartbeat > 0 && now >= heartbeat && now - heartbeat < 45;
    if (fresh) {
      HANDLE handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, static_cast<DWORD>(process));
      fresh = handle != nullptr;
      if (handle) CloseHandle(handle);
    }
    bool has_data = fresh && GetPrivateProfileIntW(L"quota", L"hasData", 0, path.c_str()) != 0;
    int used_mode = GetPrivateProfileIntW(L"quota", L"usedMode", 0, path.c_str());
    bool english = GetPrivateProfileIntW(L"quota", L"language", 0, path.c_str()) != 0;
    int five_used = has_data ? GetPrivateProfileIntW(L"quota", L"fiveUsed", -1, path.c_str()) : -1;
    int five_left = has_data ? GetPrivateProfileIntW(L"quota", L"fiveRemaining", -1, path.c_str()) : -1;
    int week_used = has_data ? GetPrivateProfileIntW(L"quota", L"weekUsed", -1, path.c_str()) : -1;
    int week_left = has_data ? GetPrivateProfileIntW(L"quota", L"weekRemaining", -1, path.c_str()) : -1;
    int five_reset = has_data ? GetPrivateProfileIntW(L"quota", L"fiveReset", 0, path.c_str()) : 0;
    int week_reset = has_data ? GetPrivateProfileIntW(L"quota", L"weekReset", 0, path.c_str()) : 0;
    bool cached = GetPrivateProfileIntW(L"quota", L"cached", 0, path.c_str()) != 0;
    five_value_ = percent_text(used_mode ? five_used : five_left);
    week_value_ = percent_text(used_mode ? week_used : week_left);
    five_label_ = reset_time_label(five_reset);
    week_label_ = reset_date_label(week_reset);
    english_ = english;
    cached_ = cached && has_data;
    five_used_ = five_used;
    five_left_ = five_left;
    week_used_ = week_used;
    week_left_ = week_left;
    five_reset_ = reset_text(five_reset > 0 ? static_cast<unsigned int>(five_reset) : 0);
    week_reset_ = reset_text(week_reset > 0 ? static_cast<unsigned int>(week_reset) : 0);
    if (hover_window_ && IsWindowVisible(hover_window_)) {
      RECT hover{};
      if (GetWindowRect(hover_window_, &hover))
        RenderHover(hover.left, hover.top, hover.right - hover.left, hover.bottom - hover.top);
    }
    if (window_) InvalidateRect(window_, nullptr, FALSE);
  }

  void ShowHover() {
    if (!window_ || !hover_window_ || !hovering_) return;
    POINT cursor = hover_point_;
    RECT band{};
    if (!GetWindowRect(window_, &band)) return;
    POINT local = cursor;
    ScreenToClient(window_, &local);
    RECT client{};
    GetClientRect(window_, &client);
    if (!PtInRect(&client, local)) return;
    HDC dc = GetDC(window_);
    UINT dpi = dc ? band_dpi(window_, dc) : 96;
    if (dc) ReleaseDC(window_, dc);
    int width = MulDiv(320, dpi, 96);
    int height = MulDiv(178, dpi, 96);
    int gap = MulDiv(8, dpi, 96);
    MONITORINFO monitor{sizeof(monitor)};
    if (!GetMonitorInfoW(MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST), &monitor)) return;
    const RECT& work = monitor.rcWork;
    int x = std::clamp((band.left + band.right - width) / 2, work.left,
                       std::max(work.left, work.right - width));
    int y = static_cast<int>(band.top) - height - gap;
    if (y < work.top) y = band.bottom + gap;
    y = std::clamp(y, static_cast<int>(work.top),
                   std::max(static_cast<int>(work.top), static_cast<int>(work.bottom) - height));
    if (RenderHover(x, y, width, height)) ShowWindow(hover_window_, SW_SHOWNOACTIVATE);
  }

  bool RenderHover(int x, int y, int width, int height) {
    if (!gdiplus_ready_ || !hover_window_ || width <= 0 || height <= 0) return false;
    Gdiplus::Bitmap surface(width, height, PixelFormat32bppPARGB);
    if (surface.GetLastStatus() != Gdiplus::Ok) return false;
    {
      Gdiplus::Graphics graphics(&surface);
      graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
      graphics.SetTextRenderingHint(Gdiplus::TextRenderingHintAntiAlias);
      graphics.Clear(Gdiplus::Color(0, 0, 0, 0));
      graphics.ScaleTransform(static_cast<float>(width) / 320.0f,
                              static_cast<float>(height) / 178.0f);
      Gdiplus::GraphicsPath card;
      rounded_path(card, 0.75f, 0.75f, 318.5f, 176.5f, 11.0f);
      Gdiplus::SolidBrush background(Gdiplus::Color(255, 28, 32, 45));
      Gdiplus::Pen outline(Gdiplus::Color(255, 84, 94, 126), 1.0f);
      graphics.FillPath(&background, &card);
      graphics.DrawPath(&outline, &card);

      Gdiplus::FontFamily family(L"Microsoft YaHei UI");
      Gdiplus::Font heading(&family, 14.0f, Gdiplus::FontStyleBold, Gdiplus::UnitPixel);
      Gdiplus::Font section_font(&family, 13.0f, Gdiplus::FontStyleBold, Gdiplus::UnitPixel);
      Gdiplus::Font detail_font(&family, 12.0f, Gdiplus::FontStyleRegular, Gdiplus::UnitPixel);
      Gdiplus::SolidBrush title_color(Gdiplus::Color(255, 244, 247, 255));
      Gdiplus::SolidBrush section_color(Gdiplus::Color(255, 229, 236, 248));
      Gdiplus::SolidBrush muted_color(Gdiplus::Color(255, 163, 175, 196));
      Gdiplus::SolidBrush cache_color(Gdiplus::Color(255, 250, 195, 113));
      Gdiplus::StringFormat left;
      left.SetLineAlignment(Gdiplus::StringAlignmentCenter);
      left.SetFormatFlags(Gdiplus::StringFormatFlagsNoWrap);
      left.SetTrimming(Gdiplus::StringTrimmingEllipsisCharacter);
      Gdiplus::StringFormat right;
      right.SetLineAlignment(Gdiplus::StringAlignmentCenter);
      right.SetFormatFlags(Gdiplus::StringFormatFlagsNoWrap);
      right.SetTrimming(Gdiplus::StringTrimmingEllipsisCharacter);
      right.SetAlignment(Gdiplus::StringAlignmentFar);
      auto text = [&](const std::wstring& value, const Gdiplus::Font& font,
                      const Gdiplus::Brush& brush, const Gdiplus::RectF& rect,
                      const Gdiplus::StringFormat& format) {
        graphics.DrawString(value.c_str(), -1, &font, rect, &format, &brush);
      };
      text(L"CQBar", heading, title_color, {16.0f, 8.0f, 140.0f, 26.0f}, left);
      if (cached_)
        text(english_ ? L"Cached data" : L"缓存数据", detail_font, cache_color,
             {170.0f, 8.0f, 134.0f, 26.0f}, right);
      Gdiplus::Pen divider(Gdiplus::Color(255, 60, 68, 88), 1.0f);
      graphics.DrawLine(&divider, 16.0f, 38.5f, 304.0f, 38.5f);
      auto section = [&](float top, const std::wstring& label, int remaining, int used,
                         const std::wstring& reset, const Gdiplus::Color& color) {
        text(label, section_font, section_color, {16.0f, top, 145.0f, 19.0f}, left);
        text((english_ ? L"Left " : L"剩余 ") + percent_text(remaining), section_font,
             title_color, {173.0f, top, 131.0f, 19.0f}, right);
        Gdiplus::GraphicsPath track;
        rounded_path(track, 16.0f, top + 26.0f, 288.0f, 8.0f, 4.0f);
        Gdiplus::SolidBrush track_color(Gdiplus::Color(255, 64, 72, 92));
        graphics.FillPath(&track_color, &track);
        if (remaining > 0 && remaining <= 100) {
          float filled = 288.0f * static_cast<float>(remaining) / 100.0f;
          Gdiplus::GraphicsPath progress;
          rounded_path(progress, 16.0f, top + 26.0f, filled, 8.0f,
                       std::min(4.0f, filled / 2.0f));
          Gdiplus::SolidBrush fill(color);
          graphics.FillPath(&fill, &progress);
        }
        text((english_ ? L"Used " : L"已用 ") + percent_text(used), detail_font,
             muted_color, {16.0f, top + 39.0f, 92.0f, 19.0f}, left);
        text((english_ ? L"Reset " : L"重置 ") + reset, detail_font,
             muted_color, {110.0f, top + 39.0f, 194.0f, 19.0f}, right);
      };
      section(44.0f, english_ ? L"5 hours" : L"五小时", five_left_, five_used_, five_reset_,
              Gdiplus::Color(255, 74, 210, 223));
      graphics.DrawLine(&divider, 16.0f, 107.5f, 304.0f, 107.5f);
      section(113.0f, english_ ? L"Weekly" : L"周额度", week_left_, week_used_, week_reset_,
              Gdiplus::Color(255, 172, 143, 255));
    }

    Gdiplus::Rect lock_area(0, 0, width, height);
    Gdiplus::BitmapData data{};
    if (surface.LockBits(&lock_area, Gdiplus::ImageLockModeRead,
                         PixelFormat32bppPARGB, &data) != Gdiplus::Ok) return false;
    HDC screen = GetDC(nullptr);
    HDC memory = screen ? CreateCompatibleDC(screen) : nullptr;
    BITMAPINFO info{};
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    void* pixels = nullptr;
    HBITMAP dib = screen ? CreateDIBSection(screen, &info, DIB_RGB_COLORS, &pixels, nullptr, 0) : nullptr;
    bool rendered = false;
    if (memory && dib && pixels) {
      for (int row = 0; row < height; ++row)
        std::memcpy(static_cast<BYTE*>(pixels) + row * width * 4,
                    static_cast<BYTE*>(data.Scan0) + row * data.Stride, width * 4);
      HGDIOBJ previous = SelectObject(memory, dib);
      POINT destination{x, y};
      POINT origin{0, 0};
      SIZE size{width, height};
      BLENDFUNCTION blend{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
      rendered = UpdateLayeredWindow(hover_window_, screen, &destination, &size, memory,
                                     &origin, 0, &blend, ULW_ALPHA) != FALSE;
      SelectObject(memory, previous);
    }
    if (dib) DeleteObject(dib);
    if (memory) DeleteDC(memory);
    if (screen) ReleaseDC(nullptr, screen);
    surface.UnlockBits(&data);
    return rendered;
  }

  static LRESULT CALLBACK HoverWndProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* self = reinterpret_cast<DeskBand*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
      auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
      self = static_cast<DeskBand*>(create->lpCreateParams);
      SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    }
    if (message == WM_NCHITTEST) return HTTRANSPARENT;
    if (message == WM_MOUSEACTIVATE) return MA_NOACTIVATE;
    if (message == WM_PAINT) {
      PAINTSTRUCT paint{};
      BeginPaint(window, &paint);
      EndPaint(window, &paint);
      return 0;
    }
    if (message == WM_ERASEBKGND) return 1;
    if (message == WM_NCDESTROY && self) self->hover_window_ = nullptr;
    return DefWindowProcW(window, message, wparam, lparam);
  }
  static LRESULT CALLBACK WndProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* self = reinterpret_cast<DeskBand*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
      auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
      self = static_cast<DeskBand*>(create->lpCreateParams);
      SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    }
    if (message == WM_PAINT) {
      PAINTSTRUCT paint{};
      HDC dc = BeginPaint(window, &paint);
      RECT area{};
      GetClientRect(window, &area);
      HBRUSH background = CreateSolidBrush(RGB(32, 32, 32));
      FillRect(dc, &area, background);
      DeleteObject(background);
      SetBkMode(dc, TRANSPARENT);
      SetTextColor(dc, RGB(245, 245, 245));
      BandLayout layout = begin_band_layout(dc, window);
      int top = std::max(0L, (area.bottom - area.top - 2L * layout.row_height) / 2);
      RECT first{layout.left, top, layout.value_left, top + layout.row_height};
      RECT second{layout.left, top + layout.row_height, layout.value_left, top + 2 * layout.row_height};
      RECT value_first{layout.value_left, top, area.right - layout.right_padding, top + layout.row_height};
      RECT value_second{layout.value_left, top + layout.row_height, area.right - layout.right_padding, top + 2 * layout.row_height};
      DrawTextW(dc, self ? self->five_label_.c_str() : L"--:--", -1, &first,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER);
      DrawTextW(dc, self ? self->week_label_.c_str() : L"--/--", -1, &second,
                DT_LEFT | DT_SINGLELINE | DT_VCENTER);
      DrawTextW(dc, self ? self->five_value_.c_str() : L"--", -1, &value_first,
                DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
      DrawTextW(dc, self ? self->week_value_.c_str() : L"--", -1, &value_second,
                DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
      end_band_layout(dc, layout);
      EndPaint(window, &paint);
      return 0;
    }
    if (message == WM_ERASEBKGND) return 1;
    if (message == WM_MOUSEMOVE && self) {
      self->hover_point_ = {static_cast<short>(LOWORD(lparam)), static_cast<short>(HIWORD(lparam))};
      ClientToScreen(window, &self->hover_point_);
      if (!self->hovering_) {
        self->hovering_ = true;
        TRACKMOUSEEVENT tracking{sizeof(tracking), TME_LEAVE, window, 0};
        TrackMouseEvent(&tracking);
        SetTimer(window, kHoverTimer, 300, nullptr);
      }
      return 0;
    }
    if (message == WM_MOUSELEAVE && self) {
      self->hovering_ = false;
      KillTimer(window, kHoverTimer);
      if (self->hover_window_) ShowWindow(self->hover_window_, SW_HIDE);
      return 0;
    }
    if (message == WM_TIMER && wparam == kHoverTimer && self) {
      KillTimer(window, kHoverTimer);
      self->ShowHover();
      return 0;
    }
    if (((message == WM_TIMER && wparam == kRefreshTimer) || message == kRefreshDataMessage) && self) {
      self->RefreshData();
      return 0;
    }
    if (message == WM_NCDESTROY && self) {
      self->hovering_ = false;
      self->window_ = nullptr;
    }
    return DefWindowProcW(window, message, wparam, lparam);
  }
  std::atomic<ULONG> refs_{1};
  IUnknown* site_ = nullptr;
  HWND window_ = nullptr;
  HWND hover_window_ = nullptr;
  ULONG_PTR gdiplus_token_ = 0;
  bool gdiplus_ready_ = false;
  POINT hover_point_{};
  bool hovering_ = false;
  bool english_ = false;
  bool cached_ = false;
  int five_used_ = -1;
  int five_left_ = -1;
  int week_used_ = -1;
  int week_left_ = -1;
  std::wstring five_reset_ = L"--";
  std::wstring week_reset_ = L"--";
  std::wstring five_value_ = L"--";
  std::wstring week_value_ = L"--";
  std::wstring five_label_ = L"--:--";
  std::wstring week_label_ = L"--/--";
  BOOL composited_ = FALSE;
};

class BandFactory final : public IClassFactory {
 public:
  IFACEMETHODIMP QueryInterface(REFIID iid, void** out) override {
    if (!out) return E_POINTER;
    *out = nullptr;
    if (iid == IID_IUnknown || iid == IID_IClassFactory) *out = static_cast<IClassFactory*>(this);
    if (!*out) return E_NOINTERFACE;
    AddRef();
    return S_OK;
  }
  IFACEMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
  IFACEMETHODIMP_(ULONG) Release() override {
    ULONG remaining = --refs_;
    if (!remaining) delete this;
    return remaining;
  }
  IFACEMETHODIMP CreateInstance(IUnknown* outer, REFIID iid, void** out) override {
    if (outer) return CLASS_E_NOAGGREGATION;
    auto* band = new (std::nothrow) DeskBand();
    if (!band) return E_OUTOFMEMORY;
    HRESULT hr = band->QueryInterface(iid, out);
    band->Release();
    return hr;
  }
  IFACEMETHODIMP LockServer(BOOL lock) override {
    if (lock) ++g_locks; else --g_locks;
    return S_OK;
  }
 private:
  std::atomic<ULONG> refs_{1};
};
}

extern "C" BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, void*) {
  if (reason == DLL_PROCESS_ATTACH) {
    g_instance = instance;
    DisableThreadLibraryCalls(instance);
  }
  return TRUE;
}
extern "C" HRESULT __stdcall DllCanUnloadNow() {
  return g_objects == 0 && g_locks == 0 ? S_OK : S_FALSE;
}
extern "C" HRESULT __stdcall DllGetClassObject(REFCLSID clsid, REFIID iid, void** out) {
  if (clsid != kBandId) return CLASS_E_CLASSNOTAVAILABLE;
  auto* factory = new (std::nothrow) BandFactory();
  if (!factory) return E_OUTOFMEMORY;
  HRESULT hr = factory->QueryInterface(iid, out);
  factory->Release();
  return hr;
}
extern "C" HRESULT __stdcall DllRegisterServer() {
  wchar_t module[MAX_PATH]{};
  if (!GetModuleFileNameW(g_instance, module, ARRAYSIZE(module)))
    return HRESULT_FROM_WIN32(GetLastError());
  wchar_t path[256]{};
  StringCchPrintfW(path, ARRAYSIZE(path), L"SOFTWARE\\Classes\\CLSID\\%s", kBandIdText);
  HKEY key = nullptr;
  LONG result = RegCreateKeyExW(HKEY_LOCAL_MACHINE, path, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr);
  if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
  HRESULT hr = set_string(key, nullptr, L"CQBar");
  RegCloseKey(key);
  if (FAILED(hr)) return hr;
  wchar_t subpath[320]{};
  StringCchPrintfW(subpath, ARRAYSIZE(subpath), L"%s\\InprocServer32", path);
  result = RegCreateKeyExW(HKEY_LOCAL_MACHINE, subpath, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr);
  if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
  hr = set_string(key, nullptr, module);
  if (SUCCEEDED(hr)) hr = set_string(key, L"ThreadingModel", L"Apartment");
  RegCloseKey(key);
  if (FAILED(hr)) return hr;
  StringCchPrintfW(subpath, ARRAYSIZE(subpath),
                   L"%s\\Implemented Categories\\{00021492-0000-0000-C000-000000000046}", path);
  result = RegCreateKeyExW(HKEY_LOCAL_MACHINE, subpath, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr);
  if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
  RegCloseKey(key);
  result = RegCreateKeyExW(HKEY_LOCAL_MACHINE,
                           L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Approved",
                           0, nullptr, 0, KEY_SET_VALUE, nullptr, &key, nullptr);
  if (result != ERROR_SUCCESS) return HRESULT_FROM_WIN32(result);
  hr = set_string(key, kBandIdText, L"CQBar");
  RegCloseKey(key);
  if (FAILED(hr)) return hr;
  SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, nullptr, nullptr);
  return S_OK;
}
extern "C" HRESULT __stdcall DllUnregisterServer() {
  wchar_t path[256]{};
  StringCchPrintfW(path, ARRAYSIZE(path), L"SOFTWARE\\Classes\\CLSID\\%s", kBandIdText);
  LONG result = RegDeleteTreeW(HKEY_LOCAL_MACHINE, path);
  if (result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND)
    return HRESULT_FROM_WIN32(result);
  HKEY approved = nullptr;
  if (RegOpenKeyExW(HKEY_LOCAL_MACHINE,
                    L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Shell Extensions\\Approved",
                    0, KEY_SET_VALUE, &approved) == ERROR_SUCCESS) {
    RegDeleteValueW(approved, kBandIdText);
    RegCloseKey(approved);
  }
  SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, nullptr, nullptr);
  return S_OK;
}
