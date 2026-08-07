# SVN 图形化管理工具开发计划

目标：开发面向 macOS 的 SVN 图形化管理工具，提供类似 TortoiseSVN 的日常工作流，并把“远程仓库直连与浏览”作为核心能力。

版本：V1.0  
编制日期：2026-08-03  
建议项目名：Tortoise_svn_mac

## 1. 执行摘要

本工具作为独立 macOS 桌面应用建设。第一阶段同时支持本地工作副本与远程仓库：用户无需先 checkout，即可打开 HTTP(S)、SVN 或 file 协议的仓库 URL，查看目录树、文件内容、日志、属性和指定修订版本间差异。

推荐技术路线为 Tauri + Rust + Vue 3 + TypeScript。底层优先调用 Apache Subversion CLI。本机已安装 SVN 1.14.5，包含 svn、svnadmin、svnlook、svnsync；已确认支持 file、svn、http 和 https 协议，并可使用 macOS Keychain 缓存认证信息。

第一版应先完成“远程浏览 + checkout + 本地变更 + update + commit + 远程日志确认”的可靠闭环，再逐步建设锁定、分支标签、属性、合并、重定位、仓库管理和 Finder 集成。

## 2. 产品目标与范围

### 2.1 用户与场景

| 用户 | 主要场景 | 价值 |
|---|---|---|
| 开发人员 | 更新、提交、查看变更和日志 | 降低命令行使用成本 |
| 配置/文档维护人员 | 浏览远程目录、比较版本、下载单文件 | 不需要完整 checkout |
| 运维/发布人员 | 查询标签、分支、发布版本、锁定文件 | 版本可追溯 |
| SVN 初学者 | 理解工作副本、版本、锁定和冲突 | 降低误操作风险 |

### 2.2 第一版闭环

1. 输入远程仓库 URL。
2. 完成认证和连接检测。
3. 浏览远程目录树、文件内容、日志与 Diff。
4. 可选 checkout 成本地工作副本。
5. 修改文件，查看状态和 Diff。
6. 执行 update、resolve、commit。
7. 回到远程日志确认新的 revision。

### 2.3 分期范围

| 优先级 | 功能 | 目标版本 |
|---|---|---|
| P0 | 远程浏览、checkout、工作副本状态、Diff、update、commit、日志 | 0.1.0 |
| P1 | 导出、分支/标签、锁定、属性、revert、cleanup | 0.2.0 |
| P2 | 合并、冲突助手、重定位、切换 URL、稀疏目录、补丁 | 0.3.0 |
| P3 | 仓库管理、镜像、统计、Finder Quick Action、平台集成 | 1.0.0+ |

### 2.4 MVP 不做的内容

- 直接修改 SVN 服务器端配置、权限或数据库；
- 自动执行 svnadmin 修复、dump/load 或 hotcopy；
- 内置完整代码编辑器；
- 保存密码、Token 或私钥；
- 多仓库批量提交和脚本市场。

## 3. SVN 与 Git 工具的关键差异

| 维度 | SVN 特征 | 产品要求 |
|---|---|---|
| 历史模型 | 中央仓库、全局递增 revision | 界面突出 URL 和仓库版本号 |
| 远程访问 | 多数读取操作可直接对 URL 执行 | 远程浏览成为一级页面 |
| 工作副本 | 含 .svn 元数据，可能发生管理锁 | 支持 cleanup、resolve、revert |
| 文件锁 | 常用于二进制、配置和文档 | 显示 lock/unlock、拥有者、注释 |
| 分支标签 | 通常是服务端目录 copy | 使用 URL 到 URL 的远程 copy |
| 认证权限 | 由服务端/WebDAV/SSH 控制 | 复用 SVN 原生认证与证书处理 |

## 4. 技术路线与总体架构

### 4.1 推荐技术栈

| 层次 | 技术 | 原因 |
|---|---|---|
| 桌面容器 | Tauri 2.x | 体积小、内存低、可跨平台 |
| UI | Vue 3 + TypeScript + Vite | 维护性好，与现有经验匹配 |
| 核心服务 | Rust | 适合进程、文件系统、并发、安全边界 |
| SVN 适配 | SVN CLI + XML 输出 | 与用户 SVN 行为一致，兼容认证缓存 |
| 远程协议 | HTTP(S)、SVN、file | 已由本机 SVN 能力确认 |
| 后续扩展 | libsvn_client 原生绑定 | 仅在 CLI 无法满足进度交互时引入 |

### 4.2 首版使用 SVN CLI 的理由

1. 与用户已可用的 SVN 1.14.5 行为一致。
2. 自动继承 Keychain、服务端证书、代理和认证配置。
3. XML 输出提供稳定机器可读结果。
4. checkout、commit、update、lock、merge 等操作容易复现与排障。
5. 方便支持企业既有 hooks、证书、SSH 和代理环境。
6. svn 二进制发现：GUI 从 Finder/Dock 启动时 PATH 通常不含 /opt/homebrew/bin，启动时按 /opt/homebrew/bin/svn → /usr/bin/svn → PATH → 用户设置项 的顺序探测，设置页允许手动指定。
7. 输出稳定性：所有 svn 子进程统一以 LC_ALL=C 启动，保证错误消息与解析输出语言稳定，不受用户 locale 影响（svn 帮助/错误文本会随 locale 变化）。

所有命令必须以参数数组启动，禁止拼接 shell 字符串。svn 的 stdout/stderr 一律按原始字节接收（不预先做 UTF-8 转换），编码处理统一放在解析层。

### 4.3 分层架构

| 模块 | 职责 | 约束 |
|---|---|---|
| UI Shell | 窗口、菜单、快捷键、路由、主题 | 不承担 SVN 语义解析 |
| RemoteRepositoryService | 远程连接、目录、日志、文件预览 | 不创建工作副本也可用 |
| WorkingCopyService | status、update、commit、revert、cleanup | 写操作串行执行 |
| SvnCommandRunner | svn 二进制发现、命令、输出、退出码、取消、进度 | 参数数组；LC_ALL=C；原始字节输出；日志脱敏 |
| SvnXmlParser | info/list/status/log/prop XML 解析 | 保留原始输出供诊断 |
| TaskManager | 队列、进度、取消、任务历史 | 同一工作副本禁止并发写；进度数据来源：checkout/update 解析逐文件输出行计数，网络传输阶段仅 busy 态（svn 非 TTY 无传输进度条） |
| CredentialAdapter | Keychain、证书、用户名密码提示 | 不保存明文密码 |
| SettingsStore | 最近 URL、最近工作副本、UI 偏好 | 不保存服务器凭据 |

## 5. 远程仓库功能设计

### 5.1 远程 URL 打开与连接检测

用户输入 URL 后，依次执行：

    svn info --xml URL
    svn list --xml -v URL

页面应显示仓库根 URL、当前目录 URL、UUID、HEAD revision、相对仓库路径、协议、连接状态、认证状态、证书状态、最近访问时间和收藏状态。

认证失败、无权限、证书不受信任、代理失败或 URL 无效时，必须展示分类后的中文说明和修复建议，并可展开查看原始 stderr。

### 5.2 远程目录树

功能要求：

- 初次只请求当前目录，展开时惰性加载子目录；
- 使用 svn list --xml -v 获取名称、类型、最后 revision、作者、日期和大小；
- 支持路径输入、面包屑、返回上级、刷新与收藏；
- 支持按名称、类型、作者、最后修订号过滤；
- 历史目录浏览统一使用 svn list --xml -v -r REV URL@REV；-r 是操作 revision，@REV 是 peg revision；
- 支持从任意远程目录启动 checkout 或导出。

性能要求：

- 禁止递归拉取整个仓库；
- 大目录使用前端虚拟列表（svn list 无服务端分页参数，一次性全量拉取后前端虚拟化呈现）；
- 远程任务支持取消和超时；
- URL 或 revision 改变后，旧请求结果不得覆盖当前页面。

### 5.3 远程文件预览、下载、属性与 Diff

| 操作 | SVN 调用 | UI 要求 |
|---|---|---|
| 文本预览 | svn cat -r REV URL@REV | 自动识别编码；按大小阈值与二进制判定决定是否预览（见下方说明） |
| 下载单文件 | svn cat -r REV URL@REV | 选择保存位置；覆盖前确认 |
| 查看属性 | svn proplist -v -r REV URL@REV | 显示 mime-type、keywords 等 |
| 查看差异 | 同一路径：svn diff -r OLD:NEW URL@PEG；跨路径：svn diff OLD_URL@OLD NEW_URL@NEW | 统一视图和 revision 选择 |
| 查看日志 | svn log --xml -v -l N URL | 增量加载和路径过滤 |

二进制文件仅显示元数据、大小、MIME 类型、锁信息和下载入口，不显示错误的文本 Diff。

peg 修正：同一路径 diff 的 peg revision 必须固定为路径确定存在的 revision（URL@HEAD 或 URL@OLD）。URL@NEW 在路径于 NEW 不存在（已删除/改名）时会解析失败，此时提示“路径在该 revision 不存在”。

大文件预览限制：svn cat 不支持按行/字节范围读取，“截断预览”必须先全量下载，对超大文件不可行。策略：预览前先按 svn:mime-type 与内容 NUL 字节判定二进制；超过大小阈值（如 2 MB）的文件只显示元数据与下载入口；文件内容以原始字节传给前端，编码探测与解码在前端进行。

Diff 展示（已落地，2026-08-04）：采用 Beyond Compare 式双栏并排视图（@codemirror/merge 双栏模式），替代统一文本视图：

- 数据通道：并排视图需要两侧完整文本——本地为 `svn cat -r BASE <wc-file>` + 当前文件内容（wc_file_pair），远程为 `svn cat` ×2 个 revision；未版本化文件无基线，左侧留空并标注；
- 交互：A/B 双栏、行内字符级高亮（highlightChanges）、折叠/展开未变更块（collapseUnchanged）、双向同步滚动、顶部差异统计（+N/−M/变更处数）；
- 大文件策略：两侧总字符超过 400K 时，前端不再用 merge 内置算法（主线程计算会卡 UI），改调 Rust 侧 diff_chunks（similar crate 行级 diff，返回与 merge Chunk 语义一致的字符区间），前端仅负责渲染；
- 二进制文件不进入并排视图，显示提示；远程旧版本读取失败（路径在该版本不存在）时降级提示，右侧仍显示新版内容；
- 后续增强（P1）：语法高亮（@codemirror/language-data）、变更块跳转（goToNextChunk/goToPreviousChunk）、外部 Diff/Merge 工具入口。

### 5.4 远程日志与修订版本

日志页面至少提供：

- revision、作者、日期、提交说明；
- 修改路径、动作 A/M/D/R、copy-from 信息；
- 按 revision、作者、关键词、时间范围筛选；
- 跳转指定 revision；
- 在指定 revision 浏览文件和目录；
- 比较两个 revision；
- 将 revision 固定为标签式浏览上下文。

建议调用：

    svn log --xml -v -l LIMIT URL
    svn log --xml -r START:END -v URL
    svn log --search KEYWORD / --author NAME / -r {DATE}:{DATE}   # svn 1.14 原生过滤，避免全量拉取后前端过滤
    svn diff -r OLD:NEW URL@PEG   # peg 固定为路径存在的 revision，见 5.3

### 5.5 远程目录写操作（P1/P2）

| 功能 | SVN 模式 | 风险控制 |
|---|---|---|
| 创建目录 | svn mkdir URL -m MESSAGE | 显示目标 URL 与提交说明 |
| 删除 | svn delete URL -m MESSAGE | 强确认，列出影响路径 |
| 创建分支/标签 | svn copy SRC_URL DST_URL -m MESSAGE | 校验目标不存在并标识源 revision |
| 移动/重命名 | svn move SRC_URL DST_URL -m MESSAGE | 强确认与引用影响提示 |
| 修改版本属性 | svn propset --revprop -r REV NAME VALUE URL | 仅管理员授权且服务端 pre-revprop-change hook 允许时开放 |
| 编辑提交说明 | svn propset --revprop -r REV svn:log MESSAGE URL | 同 revprop 约束，pre-revprop-change hook 允许时开放 |
| 导入本地目录 | svn import LOCAL_PATH URL -m MESSAGE | 显示目标 URL、待导入文件数与提交说明 |

SVN 的版本化路径属性不能直接对远程 URL 设置；必须 checkout 后在工作副本中 propset，再通过 commit 提交。每个远程写操作必须填写提交说明，成功后刷新目标目录与日志。

## 6. 本地工作副本功能设计

### 6.1 工作副本识别

打开目录时调用：

    svn info --xml PATH
    svn status --xml PATH

路径比较统一 canonicalize 后执行（macOS 存在 /var ↔ /private/var、/tmp ↔ /private/tmp 符号链接差异）。若 svn 报“工作副本格式过旧”（旧版工具 checkout 的工作副本），提供 svn upgrade PATH 升级入口，升级前确认影响范围。

概览展示工作副本根目录、URL、仓库根、UUID、当前 revision、最后更新者、相对 URL、switched 子目录、冲突、锁、树冲突和未版本化文件。

### 6.2 状态、Diff 与提交

| 状态 | 说明 | 默认操作 |
|---|---|---|
| modified | 已修改 | Diff、提交、revert |
| added | 已新增 | 提交或取消添加 |
| deleted | 已删除 | 提交或恢复 |
| unversioned | 未版本化 | add 或 ignore |
| conflicted | 内容/属性冲突 | resolve、外部合并工具 |
| obstructed | 本地阻塞 | 展示修复建议 |
| missing | 记录存在但文件缺失 | restore/revert/cleanup 建议 |
| locked | 工作副本管理锁 | cleanup 入口 |

提交页要求：

- 可勾选待提交路径；
- 必填提交说明；提交说明通过 -F 临时文件（0600 权限）传递，避免 -m 的参数长度与首字符边界问题；
- 支持文本 Diff 与属性 Diff；
- 提交前提醒未版本化文件、冲突、锁和 switched 路径；
- 执行中禁止重复提交；
- commit 任务不可取消：一旦开始传输，客户端取消后服务端可能已完成提交，统一走“回远程日志确认 revision”流程；
- 成功后刷新 status、info、远程日志。

### 6.3 更新、冲突与修复

| 功能 | SVN 调用 | 规则 |
|---|---|---|
| 更新 | svn update PATH | 显示每个路径的结果；中途取消会留下工作副本锁，提示运行 cleanup |
| 升级 | svn upgrade PATH | 旧格式工作副本升级；升级前确认影响范围 |
| 解决 | svn resolve --accept POLICY | 先展示接受策略 |
| 清理 | svn cleanup PATH | 说明用途与限制 |
| 恢复 | svn revert -R PATH | 强确认并列出受影响文件 |
| 切换 | svn switch URL PATH | 显示当前/目标 URL |
| 重定位 | svn relocate FROM TO PATH | 高风险，核对仓库 UUID |

MVP 复杂冲突可调用外部 Diff/Merge 工具。内置三方合并编辑器放入 P2。

### 6.4 锁定和属性（P1）

- 支持 svn lock 与 svn unlock；
- 展示锁拥有者、创建时间、注释和令牌状态；
- 提交前提醒锁冲突；
- 支持 proplist、propget 和受控 propset；
- 识别 svn:ignore、svn:externals、svn:mime-type、svn:needs-lock。

### 6.5 浏览与辅助功能（P1/P2）

- svn blame 逐行归属（P2）：单文件按 revision 着色展示，点击跳转对应 revision；远程 URL 与工作副本均可；
- changelist 本地文件分组（P2，可选）：支持按组提交；
- 与服务器比较 svn status -u（P2，可选）：显示过期/远端被改动的文件；
- 工作副本自动刷新（P1）：窗口聚焦时刷新 status，监听 wc.db 与文件系统变化，避免外部编辑器修改后显示陈旧数据。

## 7. 数据模型与命令白名单

### 7.1 核心模型

| 模型 | 关键字段 |
|---|---|
| RemoteRepository | url、repositoryRoot、uuid、headRevision、protocol、credentialState、connectionState |
| WorkingCopy | path、url、repositoryRoot、revision、wcRoot、hasConflicts、hasLocks、hasSwitchedPaths |
| SvnEntry | path、kind、revision、author、date、size、lock |
| SvnTask | id、operation、target、status、progress、stdout、stderr、exitCode、recoveryHint |
| SvnError | category、summary、detail、recoveryHint |

### 7.2 第一版命令白名单

| 类别 | 命令 |
|---|---|
| 远程只读 | info、list、cat、log、diff、proplist、propget、blame |
| 本地只读 | info、status、diff、log、version、blame |
| 工作副本写 | add、delete、revert、update、commit、cleanup、resolve、upgrade、changelist |
| 远程写（P1） | mkdir、copy、move、delete、propset、import |
| 锁定（P1） | lock、unlock |
| 凭据管理（P1） | auth（查看/清除已保存的认证缓存） |

## 8. 认证、安全与网络

### 8.1 认证策略

- 优先复用 SVN 原生认证缓存和 macOS Keychain；
- 认证缓存写入策略：默认允许 svn 原生认证缓存（写入 ~/.subversion/auth 与 Keychain），这是“复用 Keychain”的基础；提供 svn auth 命令作为“查看/清除已保存凭据”入口；仅当用户明确选择“本次不保存”时使用 --no-auth-cache；
- 证书信任机制（CLI 无“一次性接受”参数，非交互模式下证书验证失败直接报错）：候选方案——① 设置项配置 servers:global:ssl-authority-files 指向私有 CA 文件；② 引导用户用命令行接受一次（写入认证缓存）；③ 沙盒式隔离 $HOME/.subversion 管理缓存。方案在阶段 A 通过 HTTPS fixture 验证后定案；
- 连接预检先以 non-interactive 方式执行；认证不足时由 GUI 弹窗临时收集用户名和密码；
- 重试时使用 username 与 password-from-stdin，把密码仅写入子进程标准输入；禁止把密码放进命令参数、URL、设置或日志；
- 不在应用数据库、偏好文件或任务日志保存密码；
- 不明文显示 URL 中的凭据；
- 支持用户名密码、客户端证书、HTTPS 信任提示；证书的永久信任必须通过 HTTP(S) fixture 验证 SVN 认证缓存与 Keychain 行为后再开放；
- svn+ssh 作为能力探测后扩展协议，复用系统 SSH 配置与 agent。

### 8.2 证书和代理

必须支持未知 CA、证书过期、主机名不匹配、拒绝连接等明确分类；允许用户一次性信任、永久信任或取消。提供 SVN 配置与代理的只读诊断，但不展示代理密码。

### 8.3 命令安全

- 所有调用通过 Command::new(svn).args([...]) 执行；
- 路径、URL、revision、提交说明均为独立参数；
- 默认禁用 force、ignore-ancestry、theirs-full 等危险选项；
- stderr 归类为认证、网络、证书、冲突、锁定、工作副本损坏、权限、未知错误；分类基于 svn 错误码（E170001 认证、E170013/E000111 网络、E155004 工作副本锁、E155037 冲突等）与稳定输出模式匹配，禁止使用 "conflict"/"lock" 等泛子串分类（路径名含这些字样会误分类）；
- 日志始终脱敏密码、Token、Authorization 和私钥内容。

## 9. 详细实施计划

### 阶段 A：验证与原型（第 1 周）

1. 创建独立 Tauri 项目和编码规范。
2. 验证 Tauri、Rust、Vue 的 IPC 通路。
3. 探测 svn version 与可用 RA 模块；确定 svn 二进制发现机制与 LC_ALL=C 输出稳定性（见 4.2）。
4. 建立测试仓库：标准 trunk/branches/tags、中文路径、二进制、属性、锁、冲突。
5. 建立 HTTP(S) 远程 fixture，覆盖认证、权限、证书、慢网络；在此 fixture 上验证并定案证书信任机制（见 8.1）；所有测试使用隔离的临时 HOME/.subversion，不污染真实用户认证缓存与 Keychain。
6. 完成 URL 连接原型：info XML 与 list XML。

验收：可连接远程 URL，显示仓库根、HEAD revision 和当前目录。

### 阶段 B：远程浏览器 MVP（第 2-3 周）

1. 实现 RemoteRepositoryService、SvnCommandRunner、XML 解析器；命令输出按原始字节接收，实现非 UTF-8 提交说明/路径的容错与降级展示（XML 解析失败时展示替换符并标记，不崩溃）。
2. 实现目录树、面包屑、路径输入、刷新与收藏。
3. 实现文件预览、下载、属性查看。
4. 实现远程日志、提交详情、修改路径、revision 跳转。
5. 实现远程 revision Diff 与路径过滤。
6. 实现认证、证书、超时、取消和错误分类。

验收：不 checkout 时可浏览远程目录、文件、日志和两个 revision 的差异。

### 阶段 C：工作副本基础能力（第 4-5 周）

1. 自动识别工作副本与最近目录。
2. 解析 status XML，分组显示状态。
3. 实现本地/属性 Diff。
4. 实现 add、delete、revert、ignore。
5. 实现 update、commit、任务进度和结果摘要；提交说明走 -F 临时文件（0600）；commit 任务不可取消，取消后进入“远程日志确认”流程。
6. 实现 checkout 向导：URL、目标、revision、depth、认证。

验收：可从远程 checkout，完成修改、Diff、更新、提交，并在远程日志看到新 revision。

### 阶段 D：稳健性和日常增强（第 6-7 周）

1. cleanup、resolve、冲突状态和外部合并工具。
2. lock/unlock、锁信息、svn:needs-lock。
3. 属性页、svn:ignore、svn:externals。
4. export、远程单文件下载、补丁创建与应用。
5. switched 路径、稀疏工作副本和健康诊断。
6. svn upgrade（旧格式工作副本）、svn blame 逐行浏览、窗口聚焦刷新与 wc.db/文件系统监听。

验收：常见冲突、锁、未版本化文件和工作副本锁有明确处理路径。

### 阶段 E：分支、标签与远程写（第 8-9 周）

1. 识别并允许配置 trunk/branches/tags 布局。
2. 实现 URL 到 URL 的 copy 创建分支和标签。
3. 实现远程 mkdir、move、delete、import 与提交说明；编辑提交说明（svn:log revprop）按 hook 授权开放。
4. 实现分支/标签历史和 revision 比较。
5. 对 switch 与 relocate 加风险确认和恢复说明。

验收：可安全创建标签/分支、浏览历史，并对远程目录操作留下审计日志。

### 阶段 F：复杂合并与发布（第 10-12 周）

1. merge 向导：同步 merge、指定 revision 范围（svn 1.8+ 已废弃 --reintegrate，reintegration 场景自动识别，不再单独列 reintegrate 模式）。
2. mergeinfo 查看和合并预检。
3. 内置三方 Diff/冲突助手（可拆分后续版本）。
4. Finder Quick Action、拖拽打开、外部 Diff/Merge 配置。
5. Apple Silicon 打包、签名、notarization、升级和用户文档。

验收：完成 0.3.0 或 1.0.0 发布候选，具备安装、升级、诊断与恢复说明。

## 10. 测试与验收

### 10.1 Fixture 覆盖

- 空仓库、空目录、标准 trunk/branches/tags；
- 中文、空格、特殊字符、极长路径；
- 远程 HTTP/HTTPS、svn、file；
- 无认证、认证成功、认证失败、无权限、只读权限；
- 自签名、过期、主机名不匹配证书；
- 大目录、长日志、二进制、大文件；
- 文件锁、锁冲突、内容/属性/树冲突；
- 非 UTF-8 提交说明与老仓库数据（GBK 提交信息、非法字节路径）；
- 旧格式工作副本（触发 svn upgrade 流程）；
- cleanup 前后状态；
- switched 子目录、externals、稀疏工作副本；
- 分支 copy、标签 copy、rename、delete、merge。

### 10.2 测试分层

| 层次 | 验证内容 |
|---|---|
| Rust 单元测试 | XML 解析、状态映射、错误分类、URL 校验 |
| 集成测试 | 真实 SVN 命令与临时工作副本 |
| 远程集成测试 | HTTP(S)、认证、权限、证书、超时 |
| UI 测试 | 树加载、状态、确认框、取消、错误提示（tauri-driver WebDriver 自动化） |
| 回归测试 | 发布前执行 P0/P1 全链路 |
| 性能测试 | 10 万条目录、长日志、慢网络、重复刷新 |
| 环境隔离 | 所有集成测试使用独立临时 HOME 与 SVN 配置目录，隔离认证缓存与 Keychain 副作用 |

### 10.3 MVP 验收清单

- 可连接远程 HTTP(S)/SVN/file URL；
- 不 checkout 也能浏览目录、日志、文件和 revision Diff；
- 认证、证书、权限和网络错误正确分类；
- 可 checkout、update、提交并在远程确认 revision；
- 工作副本状态与命令行 svn status 一致；
- 中文路径和提交说明可正常显示、提交；
- 任务可取消，慢网络和大目录不阻塞界面；
- 密码、Token、私钥和敏感 URL 不进入日志。

## 11. 风险与控制

| 风险 | 等级 | 控制措施 |
|---|---|---|
| 远程认证与证书差异 | 高 | 复用 Keychain/原生交互，建立 HTTPS fixture |
| 慢网络导致卡顿 | 高 | 异步任务、取消、超时、惰性目录加载、缓存控制 |
| 工作副本损坏或锁定 | 高 | 仅诊断与受控 cleanup，不擅自修复元数据 |
| 远程 delete/move 误操作 | 高 | 提交说明、影响预览、强确认、操作日志 |
| 合并语义复杂 | 高 | 放到 P2；先提供预检与外部工具协作 |
| 大仓库性能 | 中 | 分页、虚拟列表、增量日志、限制递归深度 |
| SVN 版本差异 | 中 | 启动能力探测、兼容矩阵、CLI 兜底 |
| Finder 集成成本 | 中 | MVP 先支持拖拽、Dock 和 Quick Action |
| 证书信任机制落地 | 高 | 阶段 A 通过 HTTPS fixture 验证并定案 ssl-authority-files / 引导接受 / 缓存沙盒方案 |
| 非 UTF-8 历史数据 | 中 | 原始字节通道 + 解析降级 + fixture 覆盖 |
| svn 二进制环境差异 | 中 | 启动探测 + 设置页手动指定 + 兼容矩阵 |

## 12. 发布与后续演进

首版发布物：

- macOS Apple Silicon 安装包；
- 远程仓库浏览、工作副本、任务和脱敏诊断日志；
- 用户手册：远程连接、checkout、提交、冲突、证书；
- 已知问题与恢复说明。

后续方向：

1. Windows/Linux 构建；
2. Finder 右键与 Quick Action 深度集成；
3. 组织级远程仓库收藏、只读策略和配置；
4. SVN 统计、贡献者视图、revision 时间线；
5. svnsync 镜像任务可视化监控；
6. 管理员模式：仓库健康只读检查、dump/load 向导。

## 13. 推荐启动顺序

1. 创建独立 Tauri 项目。
2. 先实现远程 URL、目录树、文件预览、日志、revision Diff。
3. 再实现 checkout、status、update、commit。
4. 完成认证、证书、取消、诊断后发布 0.1.0。
5. 最后实现锁定、分支标签、合并和 Finder 集成。

结论：SVN 工具的首要竞争力应是可靠的远程仓库体验，而不只是给本地工作副本加 GUI。远程浏览、revision 对比、认证处理和不 checkout 下载必须从第一天进入核心架构和验收范围。
