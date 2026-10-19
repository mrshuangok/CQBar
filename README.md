# CQBar

CQBar 是 Windows 上的 Codex 额度悬浮窗。它从当前用户的 `~/.codex/auth.json` 读取登录信息，向 ChatGPT 额度接口查询数据，并显示五小时与每周额度。Windows 10 x64 还可使用任务栏 DeskBand；Windows 11 使用悬浮窗。

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

## 登录信息

程序在运行时读取本机 Codex 登录文件；仓库不包含账号、密码、令牌或本地使用数据。不要把 `auth.json`、日志或其他个人配置提交到仓库。

## 许可证

MIT，见 [LICENSE](LICENSE)。
