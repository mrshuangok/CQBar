# CQBar

[English](README.md) | [简体中文](README.zh-CN.md)

See your current Codex quota and reset times on the Windows desktop.

## Features
- Shows the current Codex account's five-hour and weekly limits, usage, and reset times.
- Supports manual and automatic refresh and saved preferences; capsule metrics can switch between usage and reset time, while the tray controls display mode, pinning, and language.
- Windows 10 x64 can use the taskbar toolbar; Windows 11 can use the floating window.
- The Windows 10 taskbar hover card shows remaining progress, used percentages, and reset times for both quota windows.
- One installer includes Simplified Chinese and English interfaces.

## Requirements
- Codex must already be signed in on Windows; the app reads the current user's .codex/auth.json.
- Authentication tokens are not shown in the interface or written to app settings.

## Install
- Download the Windows x64 installer from the latest release and run it; administrator permission is required.

## Interface preview

### Compact display

Compact display is available as a floating window or a taskbar toolbar. The taskbar toolbar supports Windows 10 x64.

<table>
  <thead>
    <tr><th>Floating window</th><th>Taskbar toolbar</th></tr>
  </thead>
  <tbody>
    <tr>
      <td align="center"><img src="docs/images/floating-compact.png" alt="Compact floating window" width="280"></td>
      <td align="center"><img src="docs/images/taskbar-compact.png" alt="Compact taskbar toolbar" width="280"></td>
    </tr>
  </tbody>
</table>

### Taskbar quota details

Hover over a taskbar quota to see remaining progress, usage, and reset times.

<table>
  <thead>
    <tr><th>简体中文</th><th>English</th></tr>
  </thead>
  <tbody>
    <tr>
      <td align="center"><img src="docs/images/taskbar-hover-zh.png" alt="Usage details in Simplified Chinese" width="320"></td>
      <td align="center"><img src="docs/images/taskbar-hover-en.png" alt="Usage details in English" width="320"></td>
    </tr>
  </tbody>
</table>

### Floating window settings

The settings panel controls the refresh interval, display mode, theme, and taskbar pinning state.

![CQBar settings panel](docs/images/settings-panel.png)

## From source

Windows, Node.js, the Rust MSVC toolchain, Visual Studio C++ x64 build tools, and the Windows SDK are required.

Run the app in development:

~~~powershell
npm ci
npm run tauri:dev
~~~

Build the Windows x64 installer:

~~~powershell
npm run release
~~~

The installer and SHA-256 checksum file are written to release/. Installing the DeskBand requires administrator permission. The release script builds locally and does not upload the installer.

## License

MIT; see [LICENSE](LICENSE).

## Releases and downloads
- [Release notes for this version](docs/releases/v0.1.0.en.md)
- [Download the latest release](https://github.com/mrshuangok/CQBar/releases/latest)
