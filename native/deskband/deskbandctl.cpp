#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <shobjidl.h>
#include <winternl.h>
#include <cstdio>
#include <cwchar>

namespace {
const CLSID kBandId = {0x4a7b6d93, 0x4829, 0x4ce3, {0x97, 0x7e, 0xd2, 0x51, 0x94, 0x34, 0x8a, 0x6a}};
constexpr UINT kRefreshDataMessage = WM_APP + 42;
BOOL CALLBACK refresh_band_child(HWND window, LPARAM value) {
  wchar_t name[64]{};
  if (GetClassNameW(window, name, 64) && lstrcmpW(name, L"CodexQuotaDeskBandWindow") == 0) {
    if (PostMessageW(window, kRefreshDataMessage, 0, 0)) *reinterpret_cast<bool*>(value) = true;
  }
  return TRUE;
}
BOOL CALLBACK refresh_band_top_level(HWND window, LPARAM value) {
  EnumChildWindows(window, refresh_band_child, value);
  return TRUE;
}
bool supported_windows() {
  HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
  if (!ntdll) return false;
  using VersionFn = LONG(WINAPI*)(PRTL_OSVERSIONINFOW);
  auto* version = reinterpret_cast<VersionFn>(GetProcAddress(ntdll, "RtlGetVersion"));
  if (!version) return false;
  RTL_OSVERSIONINFOW info{sizeof(info)};
  return version(&info) == 0 && info.dwMajorVersion == 10 &&
         info.dwBuildNumber >= 10240 && info.dwBuildNumber < 22000;
}
}

int wmain(int argc, wchar_t** argv) {
  if (argc != 2) return 2;
  if (lstrcmpiW(argv[1], L"supported") == 0) return supported_windows() ? 0 : 5;
  if (!supported_windows() && lstrcmpiW(argv[1], L"hide") != 0) return 5;
  if (lstrcmpiW(argv[1], L"refresh-data") == 0) {
    bool posted = false;
    EnumWindows(refresh_band_top_level, reinterpret_cast<LPARAM>(&posted));
    return posted ? 0 : 1;
  }
  HRESULT hr = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  if (FAILED(hr)) return 3;
  if (lstrcmpiW(argv[1], L"self-test") == 0) {
    IDeskBand2* band = nullptr;
    hr = CoCreateInstance(kBandId, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&band));
    if (SUCCEEDED(hr)) band->Release();
    CoUninitialize();
    if (FAILED(hr)) std::fwprintf(stderr, L"Band activation failed: 0x%08X\n", static_cast<unsigned>(hr));
    return SUCCEEDED(hr) ? 0 : 4;
  }
  ITrayDeskBand* tray = nullptr;
  hr = CoCreateInstance(CLSID_TrayDeskBand, nullptr, CLSCTX_INPROC_SERVER | CLSCTX_LOCAL_SERVER,
                        IID_PPV_ARGS(&tray));
  if (FAILED(hr)) std::fwprintf(stderr, L"Tray COM activation failed: 0x%08X\n", static_cast<unsigned>(hr));
  if (SUCCEEDED(hr)) {
    if (lstrcmpiW(argv[1], L"show") == 0) {
      hr = tray->IsDeskBandShown(kBandId);
      if (hr == S_FALSE) hr = tray->ShowDeskBand(kBandId);
      if (FAILED(hr)) std::fwprintf(stderr, L"ShowDeskBand failed: 0x%08X\n", static_cast<unsigned>(hr));
      if (SUCCEEDED(hr)) {
        // The shell may wait for the user to approve a new toolbar. Query the
        // same request long enough instead of issuing another ShowDeskBand call.
        for (int attempt = 0; attempt < 300; ++attempt) {
          hr = tray->IsDeskBandShown(kBandId);
          if (hr == S_OK || FAILED(hr)) break;
          Sleep(100);
        }
      }
    } else if (lstrcmpiW(argv[1], L"hide") == 0) {
      hr = tray->HideDeskBand(kBandId);
      if (SUCCEEDED(hr)) {
        for (int attempt = 0; attempt < 20; ++attempt) {
          HRESULT shown = tray->IsDeskBandShown(kBandId);
          if (shown == S_FALSE) { hr = S_OK; break; }
          if (FAILED(shown)) { hr = shown; break; }
          Sleep(100);
        }
      }
    } else if (lstrcmpiW(argv[1], L"is-shown") == 0) {
      hr = tray->IsDeskBandShown(kBandId);
    } else if (lstrcmpiW(argv[1], L"refresh") == 0) {
      hr = tray->DeskBandRegistrationChanged();
    } else {
      hr = E_INVALIDARG;
    }
    tray->Release();
  }
  CoUninitialize();
  if (hr == S_OK) return 0;
  if (hr == S_FALSE) return 1;
  std::fwprintf(stderr, L"DeskBand operation failed: 0x%08X\n", static_cast<unsigned>(hr));
  return 4;
}
