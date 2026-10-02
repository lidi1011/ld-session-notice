# 贡献

请先运行 `pnpm build` 与 `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib`。PR 说明问题、行为变化、验证方式及未验证范围。涉及 hooks 时保持幂等并保留用户原有配置；界面修改兼顾简体中文与英文。

生成物、真实数据库、hook token、浏览器凭据和签名材料不得提交。发布流程见 docs/releasing.md。
