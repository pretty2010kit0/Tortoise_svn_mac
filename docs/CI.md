# CI 与发布流水线

本项目使用 **GitHub Actions** 做持续集成与发布。仓库公开后，所有流水线即可在 GitHub 的 **Actions** 标签页查看运行结果。

## 流水线总览

| 文件 | 触发时机 | 做什么 |
|---|---|---|
| `.github/workflows/ci.yml` | push 到 `main`、任意 PR | 类型检查 → 前端构建 → Rust 测试（真实 svn 仓库闭环）→ 文档空白检查 |
| `.github/workflows/release.yml` | 推送 `v*` 格式 tag | 构建 **Universal DMG**（同时支持 Intel / Apple Silicon）→ 发布 GitHub Release 草稿 |

两套都在 `macos-latest`（macOS arm64）runner 上运行；CI 门禁约 3–6 分钟（主要耗时在 `brew install subversion` 与首次 cargo 编译，后续有缓存）。

## CI（ci.yml）

每次 push / PR 自动执行，作为合并门禁：

1. 检出代码
2. Node 22 + npm 缓存，`npm ci` 安装前端依赖
3. Rust stable + cargo 缓存
4. `brew install subversion` —— 集成测试依赖真实 `svn`/`svnadmin`（测试代码硬编码 `/opt/homebrew/bin/svn`，与 runner 一致）
5. `npm run typecheck` —— 前端类型检查（vue-tsc）
6. `npm run build` —— 前端产物构建
7. `cargo test` —— Rust 单元 + 集成测试（在临时仓库上跑完整闭环：检出/提交/合并/分支/认证/后台任务等）
8. `git diff --check` —— 空白错误检查

> 注意：CI 不做 GUI 冒烟（runner 无图形会话）。GUI 相关问题在本地 `./manage.sh start` 人工验证。

## 发布（release.yml）

### 发布步骤

```bash
# 1. 确认版本号（src-tauri/Cargo.toml 与 package.json 保持一致）
# 2. 打 tag 并推送
git tag v0.1.0
git push origin v0.1.0
```

推送后自动执行：

1. 同 CI 前半段（Node / Rust / 依赖）
2. 安装 **universal 双架构 target**（`aarch64-apple-darwin` + `x86_64-apple-darwin`）
3. `tauri-action` 构建 **Universal DMG**（一个包同时跑 Intel 与 Apple Silicon Mac）
4. 创建 GitHub Release **草稿**（`releaseDraft: true`，不会自动公开）
5. 你到 Releases 页**检查后手动发布**（补充分类、关联截图等）

### Tag 命名规范

- 格式：`v<主>.<次>.<修订>`，如 `v0.1.0`、`v1.0.0`
- 发布前确认三处版本一致：
  - `src-tauri/Cargo.toml` 的 `version`
  - `package.json` 的 `version`
  - tag 号

## 产物说明

- **`svn-desktop-tool_<版本>_universal.dmg`** —— 通用安装包（推荐）
- 首次打开若提示"无法验证开发者"，在 **系统设置 → 隐私与安全性** 中点击"仍要打开"（无签名 + 未公证属预期行为；后续如需消除提示，需 Apple Developer 账号签名 + 公证，见下方）

## 常见问题

| 现象 | 原因 / 处理 |
|---|---|
| CI 失败在 `brew install subversion` | runner 网络/Homebrew 偶发，重跑即可；若持续可给 brew 加缓存 |
| Release 构建时间过长 | Universal 编译双架构，首次约 10–15 分钟，后续有 cargo 缓存 |
| DMG 打开提示损坏/无法验证 | 未签名未公证。临时放行：`xattr -dr com.apple.quarantine /Applications/svn-desktop-tool.app`，或系统设置中"仍要打开" |
| 发布后没有产物 | 检查是否推送了 `v*` 格式 tag；发布是 **草稿**，需人工确认后才公开 |

## 签名与公证（可选，正式分发推荐）

- 用 Apple Developer 账号生成 Developer ID 证书（`Developer ID Application`）
- 配置仓库 secrets：`APPLE_CERTIFICATE`（p12 base64）、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`（app-specific）
- `tauri.conf.json` 配置 `bundle.macOS.signingIdentity` / `providerShortName`，流水线加 `notarize: true`（tauri-action）
- 完成后 DMG 不再有"无法验证开发者"提示，可直接双击安装
