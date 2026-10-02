# macOS 发布流程

1. 固定源码提交，运行前端构建与 Rust 测试，然后构建 App。
2. 检查版本、bundle ID、架构及许可文件。使用自己持有的 Developer ID Application 证书先签 helper，再签外层 App；启用 hardened runtime 和 timestamp。
3. 将 App 打成 ZIP，提交 `xcrun notarytool submit`，确认 Accepted 后 `xcrun stapler staple`、`validate` 并用 codesign / Gatekeeper 验证。
4. 从已签名并附票据的 App 生成发行 ZIP 与 DMG。DMG 可再签名、公证和附票据。
5. 挂载 DMG、复制其中的 App，再检查签名、票据、Gatekeeper 与主程序哈希。确认下载后的安装体验，并生成 SHA256SUMS.txt。
6. 创建版本标签和 GitHub Release，上传 DMG、ZIP 与 SHA256SUMS.txt。

不要把证书、私钥、密码、API 凭据、本机数据库、hook token、个人日志或回退副本提交到仓库。不要在 App 公证之后再次运行会覆盖已签名产物的构建命令。

本次 v1.0.0 为 Apple Silicon 发行，保留 `dev.notice.desktop` 应用身份；源码仓库命名为 ld-session-notice，应用名为 Notice。Intel 和其他 Mac 的完整安装链路尚未实测。
