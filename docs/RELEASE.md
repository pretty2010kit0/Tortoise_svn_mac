# 发布操作手册（RELEASE）

> 面向维护者：**每次发布新版本照这份做即可**。流水线原理见 [docs/CI.md](CI.md)。

## 0. 前置准备（一次性）

### 0.1 GitHub Personal Access Token

推送代码 + 流水线文件需要 token，权限勾 **两个**（缺一不可，这是最容易踩的坑）：

```
☑️ repo        —— 推送代码
☑️ workflow    —— 推送 .github/workflows/ 下的 CI 文件（GitHub 单独加锁）
```

生成位置：GitHub → Settings → Developer settings → **Personal access tokens → Tokens (classic)** → Generate new token

> ⚠️ 两个常见报错都源于 token 权限：
> - `Permission to ... denied` / `403` → 缺 `repo` 权限（或仓库还没创建）
> - `refusing to allow a Personal Access Token to create or update workflow ... without workflow scope` → 缺 `workflow` 权限
>
> token 的权限**改不了**，遇到必须删除后重新生成。

### 0.2 首次推送（配置凭据）

```bash
cd /Users/zhou/gitee/Tortoise_svn_mac
git push -u origin main
# 弹出 Username → 输入 pretty2010kit0
# 弹出 Password → 粘贴 token（屏幕不显示，直接回车）
```

成功后凭据存入 macOS 钥匙串（osxkeychain），以后不再询问。

---

## 1. 日常开发提交（每次功能完成）

```bash
cd /Users/zhou/gitee/Tortoise_svn_mac
git add -A
git commit -m "批次 N：功能说明"
git push origin main          # 触发 CI
```

push 后到 **Actions** 页（https://github.com/pretty2010kit0/Tortoise_svn_mac/actions）看 `test-and-build` 是否绿：
- 内容：Node 22 → npm ci → Rust → brew subversion → typecheck → build → cargo test → diff check
- 耗时约 **3–6 分钟**（首次慢，之后有缓存）
- 黄三角 warning 可忽略（如 Homebrew tap 信任提示）；红色 X 需点进去看日志

---

## 2. 发布新版本（完整流程）

### 第 1 步：确认版本号三处一致

| 位置 | 改法 |
|---|---|
| `src-tauri/Cargo.toml` | `version = "0.1.0"` |
| `package.json` | `"version": "0.1.0"` |
| tag | `v0.1.0`（tag 带 `v` 前缀，两处不带） |

> 递增规则：修 bug → `0.1.1`；加功能 → `0.2.0`；大版本 → `1.0.0`。

改完版本号先提交推送，等 CI 绿：

```bash
git add -A && git commit -m "release: v0.2.0" && git push origin main
```

### 第 2 步：打 tag 并推送（触发发布流水线）

```bash
git tag v0.2.0
git push origin v0.2.0
```

触发 release 流水线，自动：
1. 构建 **Universal DMG**（Intel + Apple Silicon 通用包，双架构编译约 **10–15 分钟**）
2. 创建 GitHub Release **草稿**（不会自动公开）

### 第 3 步：确认并公开 Release

1. 打开 https://github.com/pretty2010kit0/Tortoise_svn_mac/releases
2. 找到绿色草稿（写着 "Draft"）→ 点 **Edit**（铅笔图标）
3. 检查发布说明（默认含安装提示）；可选：补截图、关联 tag
4. 点 **Publish release** → 正式公开，访客即可下载 DMG

---

## 3. 版本号修改示例（0.1.0 → 0.2.0）

```bash
# 1. 改两个文件
sed -i '' 's/version = "0.1.0"/version = "0.2.0"/' src-tauri/Cargo.toml
sed -i '' 's/"version": "0.1.0"/"version": "0.2.0"/' package.json

# 2. 提交 + 推送（等 CI 绿）
git add -A && git commit -m "release: v0.2.0"
git push origin main

# 3. 打 tag + 推送（触发发布）
git tag v0.2.0
git push origin v0.2.0
```

---

## 4. 常见问题

| 现象 | 处理 |
|---|---|
| `Permission denied` / 403 | token 缺 `repo` 权限，或仓库未创建 → 检查 0.1 节 |
| `...without workflow scope` | token 缺 `workflow` 权限 → **重新生成** token（权限不可改） |
| CI 红 X | 点进 Actions 运行看具体步骤日志；多数是 brew 偶发失败 → 右上角 **Re-run jobs** |
| Release 草稿不见了 | 确认 tag 已推送（`git ls-remote origin` 看 refs/tags）；草稿在 Releases 页 |
| DMG 打开提示"无法验证开发者" | 未签名属预期。临时放行：`xattr -dr com.apple.quarantine /Applications/svn-desktop-tool.app`，或系统设置 → 隐私与安全性 → 仍要打开 |
| 推错 tag 想重来 | 本地：`git tag -d v0.1.0 && git push origin :refs/tags/v0.1.0`；GitHub 上删掉对应草稿即可 |
| 改了 token 后 push 还要旧密码 | 清钥匙串重输：`git credential-osxkeychain erase <<< $'protocol=https\nhost=github.com\n\n'` 再 push |

---

## 5. 命令速查

```bash
# 日常提交
git add -A && git commit -m "批次 N：..." && git push origin main

# 发布
git tag v1.0.0 && git push origin v1.0.0

# 看 tag 是否推送成功
git ls-remote origin 'refs/tags/*'

# 删除 tag（推错时）
git tag -d v1.0.0 && git push origin :refs/tags/v1.0.0
```

## 6. 签名与公证（可选，正式对外分发推荐）

如需消除"无法验证开发者"提示，需 Apple Developer 账号（$99/年）：
1. 生成 Developer ID 证书（`Developer ID Application`）
2. 配置仓库 secrets：`APPLE_CERTIFICATE`（p12 base64）、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_SIGNING_IDENTITY`、`APPLE_ID`、`APPLE_PASSWORD`（app-specific）
3. `tauri.conf.json` 配置签名，release.yml 开启 notarize

详见 [docs/CI.md](CI.md)「签名与公证」节。
