# 安全报告

本地服务只监听回环地址，并验证本机 token。Claude 额度仅使用当前默认浏览器 claude.ai 的必要 Cookie，凭据不持久化；Claude hooks 仅记录状态元数据。飞书密钥使用 macOS Keychain。

请在 GitHub 使用私密漏洞报告功能报告涉及凭据泄漏、权限或远程请求的问题；不要在公开 Issue 附带真实凭据、Cookie 或原始数据库。
