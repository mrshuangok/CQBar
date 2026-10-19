# CQBar（Codex Quota Bar）

CQBar：**Codex Quota Bar**，是一款 Windows 上的 Codex 额度显示工具。它从当前用户的 `~/.codex/auth.json` 读取登录信息，向 ChatGPT 额度接口查询数据，并显示五小时与每周额度。Windows 10 x64 支持任务栏 DeskBand；Windows 11 使用悬浮窗。

[下载 Windows x64 安装包](https://github.com/mrshuangok/CQBar/releases/latest)

## 界面预览

### 紧凑显示

紧凑显示提供悬浮窗和任务栏两种形态，展示五小时与每周额度。任务栏形态支持 Windows 10 x64 DeskBand。

<table>
  <thead>
    <tr><th>悬浮窗紧凑显示</th><th>任务栏紧凑显示</th></tr>
  </thead>
  <tbody>
    <tr>
      <td align="center"><img src="docs/images/floating-compact.png" alt="悬浮窗紧凑显示" width="280"></td>
      <td align="center"><img src="docs/images/taskbar-compact.png" alt="任务栏紧凑显示" width="280"></td>
    </tr>
  </tbody>
</table>

### 悬停查看额度详情

将鼠标悬停在任务栏额度上，可查看剩余比例、已用比例和重置时间。界面支持简体中文与英文。

<table>
  <thead>
    <tr><th>简体中文</th><th>English</th></tr>
  </thead>
  <tbody>
    <tr>
      <td align="center"><img src="docs/images/taskbar-hover-zh.png" alt="简体中文额度详情" width="320"></td>
      <td align="center"><img src="docs/images/taskbar-hover-en.png" alt="English usage details" width="320"></td>
    </tr>
  </tbody>
</table>

### 悬浮窗设置

设置面板可调整自动刷新间隔、显示方式、主题，以及任务栏固定状态。

![CQBar 设置面板](docs/images/settings-panel.png)

## 从源码运行

需要 Windows、Node.js、Rust MSVC 工具链、Visual Studio C++ x64 构建工具和 Windows SDK。

```powershell
npm ci
npm run tauri:dev
```

构建 Windows x64 安装包：

```powershell
npm run release
```

安装包和 SHA-256 校验文件会生成在 `release/`。安装 DeskBand 需要管理员权限。发布脚本只在本地构建，不会自动上传安装包。

## 许可证

MIT，见 [LICENSE](LICENSE)。
