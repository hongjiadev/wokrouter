# 完成 WokRouter 自动就绪、升级、开发运行时与双语 Windows 桌面交付

## 背景

目标提取自 Codex 会话 `019fb245-f12b-78c1-b0fe-f954e3c9acd0`。该会话最初报告三个用户问题：Windows 启动 WokRouter 出现控制台黑框、缺少 WokCore 时不会自动下载、首次界面语言选择错误。后续经用户逐段确认，交付范围已扩展并固定为：

- WokCore 缺失时的可信自动下载、安装、启动与授权，以及真实进度。
- WokCore 启动检查更新、显式确认升级、活动请求保护、失败回滚与进度恢复。
- debug WokRouter 优先使用 IDE 管理的 WokCore，未匹配时按时限回退生产运行时；一次桌面会话内不静默切换。
- Windows release 桌面程序不显示控制台窗口。
- 所有当前桌面可见文案和无障碍文案完整支持 `en` 与 `zh-CN`，并在首次 React 渲染前选定语言。

截至 2026-07-31 提取目标时：

- `E:\Projects\wokcore` 的相关实现已经合并并推送到 `main`，提交为 `1fa3775`。
- `E:\Projects\wokrouter` 的实现位于 worktree `E:\Projects\wokrouter\.worktrees\wokrouter-bootstrap-update-i18n`、分支 `codex/wokrouter-bootstrap-update-i18n`，相对提取时的 `main` 领先 30 个提交。
- 开发/生产运行时选择计划 `E:\Projects\wokdocs\docs\superpowers\plans\2026-07-31-wokrouter-dev-runtime-selection.md` 已完成；WokRouter 生命周期计划 `E:\Projects\wokdocs\docs\superpowers\plans\2026-07-30-wokrouter-core-lifecycle.md` 的 Task 1–6 已完成并通过独立复审。
- 生命周期 Task 7 正在执行，worktree 有三个必须保留的未提交文件：`docs/operations/development.md`、`tests/scripts/check-foundation-contract.ps1`、`tests/scripts/check-foundation-contract.tests.ps1`。新增 18 个 mutation 场景已通过，历史 foundation self-test 在原会话被中断时尚未取得最终退出结果。
- Windows GUI 与 `en`/`zh-CN` 计划 `E:\Projects\wokdocs\docs\superpowers\plans\2026-07-30-wokrouter-windows-i18n.md` 的 Task 1–6 尚未开始。
- `E:\Projects\wokdocs` 仓库的 `main` 分支有本任务的 3 个未推提交；`E:\Projects\dotfiles` 仓库的 `main` 分支有 IDE WokCore 连接配置的 1 个未推提交。两处工作区在提取时均无未提交改动。

## 目标

接续并保全现有功能 worktree，把已经确认的 WokRouter 桌面交付范围完成到可合入、可推送、可复验的状态：

1. 完成生命周期 Task 7 的合同、文档和全量验证，不丢弃或重做现有三个未提交文件。
2. 按 `E:\Projects\wokdocs\docs\superpowers\plans\2026-07-30-wokrouter-windows-i18n.md` 完成 Windows GUI subsystem 与 `en`/`zh-CN` 首屏国际化 Task 1–6。
3. 对功能分支执行完整 Rust、前端、合同、Windows release、人工路径和最终 diff 审查；所有门槛通过后，将功能分支合入 `E:\Projects\wokrouter` 仓库的 `main` 分支并推送。
4. 核对并推送 `wokdocs` 的 3 个任务文档提交和 `dotfiles` 的 1 个 IDE 配置提交，确保同一交付的代码、计划与开发入口都已持久化。

执行完成时，用户从 Windows Explorer 启动 release WokRouter 只看到桌面窗口；缺少 WokCore 时应用能可信地自动就绪并显示真实进度；可信 WokCore 的升级只在用户确认后进行且能安全恢复；debug 会话正确绑定 IDE WokCore；界面在首次绘制时按规则显示 English 或简体中文。

## 术语

- **功能 worktree**：`E:\Projects\wokrouter\.worktrees\wokrouter-bootstrap-update-i18n`。
- **功能分支**：`codex/wokrouter-bootstrap-update-i18n`。
- **开发运行时**：由 IDE 启动并管理、经进程文件身份验证后绑定的 WokCore；WokRouter 不负责启动、停止或升级它。
- **生产运行时**：通过可信安装记录、可信 `PATH` 或签名自动安装发现并由 WokRouter 管理的 WokCore。
- **首次绘制**：调用 React `createRoot(...).render(...)` 之前已经完成 locale 解析与 i18n 初始化，用户不会先看到错误语言再切换。
- **合同 mutation**：有意移除或破坏安全/行为约束，且合同测试必须因此失败的负向验证。

## 边界

范围内：

- `E:\Projects\wokrouter` 中现有功能分支涉及的 WokCore 客户端、平台层、CLI、Tauri 协调器、React 桌面、i18n catalog、Windows release/CI 合同、测试与开发文档。
- 保全并完成生命周期 Task 7 的三个未提交文件。
- Windows release desktop 的 GUI subsystem、系统 locale API、预渲染语言选择、`en`/`zh-CN` catalog 及全部当前桌面可见/无障碍文案。
- 既有 `wokdocs` 3 个计划/设计提交和 `dotfiles` 1 个 IDE 配置提交的范围核对与普通 push。
- 功能分支完成后的本地合并、普通 push 与干净状态核验。

范围外：

- 修改已经完成的 `E:\Projects\wokcore` 仓库 `main` 分支；若验证证明 WokCore 接口存在阻塞性缺陷，必须停止并提交证据，不得顺手跨仓库修复。
- 新增 `en`、`zh-CN` 以外的 catalog；`zh-TW`、`zh-HK`、`zh-Hant` 必须回退 English。
- WokRouter 自身更新、静默自动升级、伪造总体百分比、无关 UI 重构、无关依赖升级。
- 私钥、任意后端错误字符串、PID 或可执行路径进入前端协议或日志。
- 版本 bump、tag、release 发布、删除远端分支、强制推送。
- 在 bake 阶段执行上述目标；bake 只创建、审查并持久化本 memo。

## 已确认判断

- 这是普通工程、测试、文档和配置交付，不是 skill、提示词、流程规则或 agent 纪律改动，因此不适用 skill 文档 TDD 质量门。
- 目标范围以会话中已经确认并部分实现的扩展设计为准，不缩回最初三个表面问题；否则会遗留已完成并经审查的升级与开发运行时实现。
- 必须续作现有功能 worktree；不得 reset、checkout 覆盖或重新创建分支来规避当前三个未提交文件。
- WokCore 的升级进度协议已在 `E:\Projects\wokcore` 仓库 `main` 分支完成，后续只验证接口，不默认修改。
- debug WokRouter 可等待并绑定 IDE WokCore；未匹配时才回退生产路径。已选定开发运行时的桌面进程不静默切换到生产运行时。
- production/release 构建不能依赖或暴露开发运行时环境变量。
- WokCore 升级必须显式确认；首次缺失安装可自动进行，但必须保持签名、哈希、原子安装和稳定错误边界。
- 两个 catalog 必须键和占位符完全一致；品牌、协议、版本、端口、模型 ID 等技术字面量保持不翻译。

## 未决问题

无阻塞性未决问题。若执行时的仓库、分支、未提交文件或远端状态与本 memo 的提取状态不一致，按“停止/阻塞条件”处理，不把漂移解释成用户授权。

## 执行契约

### Goal 命令

/goal docs/subpowers/memos/2026-07-31-wokrouter-bootstrap-update-i18n-delivery.md

### 验收标准

1. **现有工作保全**
   - 开始执行时先确认功能 worktree 仍在 `codex/wokrouter-bootstrap-update-i18n`，并识别生命周期 Task 7 的三个未提交文件；不得丢弃、覆盖或用旧计划重建它们。
   - Task 7 的现有 18 个 lifecycle mutation、历史 foundation self-test、正向合同和文档检查全部通过后，才提交这三个文件。

2. **WokCore 自动就绪与升级**
   - 缺少生产 WokCore 时，桌面只触发一次可信下载/安装/启动/授权操作；下载显示真实字节进度，其余阶段不伪造百分比。
   - 已安装可信 WokCore 时，每个桌面进程只做一次自动更新检查；只有用户确认才安装。
   - 活动请求、验证失败、回滚、人工恢复、窗口关闭/重开与进度桥断开均保持已批准的安全状态转换，且稳定错误码和恢复动作可区分。
   - 开发运行时不被 WokRouter 启停或升级；选择后进程退出只报告停止，不静默改用生产运行时。

3. **Windows 与国际化**
   - Windows release desktop 的实际 PE subsystem 为 `2`；debug desktop 保留控制台，CLI 与 WokCore subsystem 不改变。
   - 系统 locale 优先于 `navigator.languages`/`navigator.language`，最终回退 English；`zh`、`zh-CN`、`zh-Hans` 选择 `zh-CN`，`zh-TW`、`zh-HK`、`zh-Hant` 选择 English。
   - locale 解析与 i18n 初始化发生在首次 React render 前。
   - `en` 与 `zh-CN` catalog 键、复数规则所需条目和占位符完全一致；当前 shell、生命周期、Provider、Session、用量、诊断、错误、对话框、ARIA 与屏幕阅读器文案均通过 catalog 提供。

4. **完整验证与审查**
   - 所有规定命令退出码为 `0`，Rust/前端测试失败数为 `0`，Clippy warning 数为 `0`，`git diff --check` 无错误。
   - foundation 与 release 合同的全部正向场景和 mutation 场景通过；每个 mutation 都必须证明被破坏的约束会令合同失败，不能只搜索一个可伪造注释 token。
   - 独立代码审查结论为 Critical `0`、Important `0`；Minor 必须修复或由用户明确接受。
   - 人工验收覆盖干净目录自动就绪、更新取消不下载、确认升级、活动请求拒绝、验证失败回滚、操作中关闭/重开、English 首屏、简中首屏与繁中回退。

5. **集成与持久化**
   - 功能 worktree 干净且完整验证通过后，才把功能分支以非强制方式合入 `E:\Projects\wokrouter` 仓库的 `main` 分支并普通 push。
   - `E:\Projects\wokrouter`、`E:\Projects\wokdocs`、`E:\Projects\dotfiles` 三个仓库的 `main` 分支最终均与各自 upstream 对齐，工作区干净；推送前逐仓库确认待推提交只属于本任务。
   - 不创建 tag、不 bump 版本、不发布资产、不删除远端分支。

### 验证证据

实际命令、退出码、测试数和摘要写入最终 Codex 任务报告，不新增仓库内验证报告文件；截图/录屏作为该任务的临时验证附件并在最终报告中引用。Windows 上 Rust 测试只通过功能 worktree 内的仓库固定 test host：

```powershell
cargo +1.97.1 fmt --all -- --check
cargo +1.97.1 clippy --workspace --all-targets --all-features -- -D warnings
$env:OPENAI_API_KEY=""
$env:ANTHROPIC_API_KEY=""
$env:GEMINI_API_KEY=""
$env:GOOGLE_API_KEY=""
powershell.exe -NoProfile -ExecutionPolicy Bypass `
  -File tests/scripts/run-fixed-test-host.ps1 `
  -RepositoryRoot $PWD `
  -TargetDirectory (Join-Path $PWD "target") `
  -Offline
```

前端与 catalog：

```powershell
pnpm --dir apps/desktop install --frozen-lockfile
pnpm --dir apps/desktop i18n:check
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop test:unit
pnpm --dir apps/desktop build
```

合同与 release；当前机器没有 `pwsh` 时使用同一脚本的 `powershell.exe -NoProfile -ExecutionPolicy Bypass -File <script>` 等价调用，并在报告中记录该环境差异：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/scripts/check-foundation-contract.tests.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/scripts/check-foundation-contract.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/scripts/check-release-contract.tests.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/scripts/check-release-contract.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/release/release-assets.tests.ps1
cargo +1.97.1 build -p wokrouter-desktop --release
Import-Module tests/release/WokRouter.ReleaseContract.psm1 -Force
$desktop = Join-Path $PWD "target/release/wokrouter-desktop.exe"
if ((Get-PeSubsystem -Path $desktop) -ne 2) {
  throw "Built release desktop does not use the Windows GUI subsystem."
}
```

人工验收使用隔离 app-data 和本地签名测试发行源；每个场景记录前置版本/locale、操作时间线、最终状态、稳定事件或错误码，并留存首屏/进度/恢复结果截图或录屏与相关日志摘要：

1. **干净自动就绪**：准备无安装记录、无 WokCore 进程的隔离 app-data，从 Explorer 启动 release desktop；预期无需点击即出现真实字节下载，阶段依次完成并到达 `production/running/authorized`，且只产生一次安装操作。
2. **取消与确认升级**：准备已运行的可信旧版本和本地签名的新版本；第一次打开确认框后取消，预期没有下载/替换事件且旧版本继续运行；再次确认，预期出现真实进度并最终报告新版本。
3. **活动请求保护**：让旧版本保持至少一个可观测的活动请求后确认升级；预期返回 `active_requests_remain` 及有界计数，取消排空，旧版本恢复服务且未替换。
4. **验证失败回滚**：提供签名合法但在替换后启动验证失败的测试候选；预期报告 `rolled_back`，安装记录与运行进程恢复为旧版本。若恢复也失败，必须进入 `recovery_required`，不能伪报成功。
5. **关闭与重开**：在可控的慢速下载/安装期间关闭桌面窗口，再重新打开；预期后台操作不被取消，新窗口通过 lease/status 恢复监控，不启动第二次操作，最终到达同一可信终态。
6. **三类 locale 首屏**：分别把 Windows UI locale 设为 `en-US`、`zh-CN`、`zh-TW` 后冷启动；预期首个可见帧依次为 English、简体中文、English，`document.documentElement.lang` 与最终 catalog 一致，且录屏中没有先显示另一语言再切换。另用测试构建禁用 `system_locale` invoke，证明依次回退 navigator locale、English。

静态边界和最终集成证据：

```powershell
# 合并前，在功能 worktree 检查所有已提交和未提交的功能差异。
git diff --check main...HEAD
git diff --check
rg -n "minisign encrypted secret key|kill_on_drop\(true\)" crates apps tests
rg -n '>[[:space:]]*[A-Za-z][^<{]*<|placeholder="[A-Za-z]|aria-label="[A-Za-z]|title="[A-Za-z]' apps/desktop/src/App.tsx apps/desktop/src/components
rg -n "windows_subsystem" --glob "*.rs"

# 每次 push 前分别核对 upstream 和待推范围；输出必须只含本任务提交。
git -C E:\Projects\wokrouter rev-parse --abbrev-ref --symbolic-full-name '@{upstream}'
git -C E:\Projects\wokrouter log --oneline '@{upstream}..HEAD'
git -C E:\Projects\wokdocs rev-parse --abbrev-ref --symbolic-full-name '@{upstream}'
git -C E:\Projects\wokdocs log --oneline '@{upstream}..HEAD'
git -C E:\Projects\dotfiles rev-parse --abbrev-ref --symbolic-full-name '@{upstream}'
git -C E:\Projects\dotfiles log --oneline '@{upstream}..HEAD'

# 普通 push 完成后逐仓库核对工作区与左右提交数。
git -C E:\Projects\wokrouter status --short --branch
git -C E:\Projects\wokrouter rev-list --left-right --count '@{upstream}...HEAD'
git -C E:\Projects\wokdocs status --short --branch
git -C E:\Projects\wokdocs rev-list --left-right --count '@{upstream}...HEAD'
git -C E:\Projects\dotfiles status --short --branch
git -C E:\Projects\dotfiles rev-list --left-right --count '@{upstream}...HEAD'
```

预期证据：

- 私钥搜索无真实产品命中；`kill_on_drop(true)` 只允许出现在负向合同 fixture。
- 未翻译字符串搜索只有逐项审阅并记录的品牌/技术字面量 allowlist。
- `windows_subsystem` 只出现在 desktop `main.rs` 的 release 条件属性和测试 fixture。
- 实际 release desktop 的 PE subsystem 为 `2`。
- `git diff --check main...HEAD` 在合并前退出 `0`，证明约 30 个已提交功能提交及后续提交的完整分支差异没有 whitespace error；工作区检查也退出 `0`。
- 人工验收报告能逐场景对应前置条件、操作、事件/错误码、最终版本/状态和截图或录屏；缺任一项即该场景未通过。
- 最终三个仓库没有未提交文件、没有本任务之外的待推提交，`main` 与 upstream 的左右差异均为 `0 0`。

### 任务特定质量门

- **续作门**：先核对原会话、功能 worktree 的 Git 状态与 Task 7 diff，以及以下三个 SDD ledger；任何实现前都要证明是在现有工作上续作：
  - `E:\Projects\wokrouter\.worktrees\wokrouter-bootstrap-update-i18n\.superpowers\sdd\2026-07-31-wokrouter-dev-runtime-selection\progress.md`
  - `E:\Projects\wokrouter\.worktrees\wokrouter-bootstrap-update-i18n\.superpowers\sdd\2026-07-30-wokrouter-core-lifecycle\progress.md`
  - `E:\Projects\wokrouter\.worktrees\wokrouter-bootstrap-update-i18n\.superpowers\sdd\2026-07-30-wokrouter-windows-i18n\progress.md`
- **mutation 门**：合同必须杀死安全公钥、child argv、`CREATE_NO_WINDOW`、进度事件、稳定错误、开发运行时 update gate、显式确认、bridge 就绪和私钥/`kill_on_drop(true)` 等破坏场景；删除生产逻辑或插入同名字注释不能让 mutation 虚假通过。
- **first-paint 门**：测试必须能在交换 `initializeI18n` 与 `createRoot` 顺序后失败。
- **catalog 门**：删除中文 catalog、删键、改占位符或移除 CI `i18n:check` 时，对应检查必须失败。
- **PE 门**：既检查 source contract，也读取实际 release 可执行文件 PE header；只看到源码属性不算通过。
- **审查门**：各剩余实施任务保持测试先行与独立复审；最终再审查完整 `main...feature` diff 和跨仓库持久化边界。

### 范围边界

- 允许修改的主要 surface：`crates/wokrouter-wokcore-client`、`crates/wokrouter-platform`、`apps/cli`、`apps/desktop/src-tauri`、`apps/desktop/src`、`tests/scripts`、`tests/release`、`.github/workflows/ci.yml`、`docs/operations/development.md`。manifest/lockfile 只允许修改 `E:\Projects\wokdocs\docs\superpowers\plans\2026-07-30-wokrouter-windows-i18n.md` Task 1–6 各自 `Files` 段明确列出的文件。
- 新增依赖仅限已批准计划要求的 i18n 运行时与测试依赖；出现额外依赖需求必须先证明现有方案不可行并询问用户。
- 不修改 `E:\Projects\wokcore` 产品代码；不新建并行功能分支或替代 worktree。
- `wokdocs` 与 `dotfiles` 默认只核对并推送已存在的本任务提交；发现需改内容时停止并单独说明，不把修订混入 WokRouter 功能提交。
- bake 自身只提交并 push 本 memo；目标代码的提交、合并和 push 属于后续 goal。

### 停止/阻塞条件

遇到以下任一情况立即停止扩展动作，保留现状并向用户给出仓库、命令、退出码和最小证据：

- 功能 worktree、分支或三个 Task 7 未提交文件缺失、被其他会话修改，或实际 diff 无法与原会话状态对应。
- 发现并行 session 正在修改同一文件，或 session-presence 留言显示范围冲突。
- 需要改动 `E:\Projects\wokcore` 仓库的 `main` 分支、引入计划外依赖、扩大语言范围、改变已批准安全/升级语义。
- 签名测试资产、Windows 打包能力、凭据、网络、权限或外部服务不可用，导致行为门无法真实验证。
- mutation 不能可靠杀死对应破坏、测试出现不稳定/超时但根因不明、实际 PE subsystem 不是 `2`、catalog 检查或 first-paint 顺序无法证明。
- 任一独立审查仍有 Critical/Important，或 Minor 的处理需要产品取舍。
- 任一仓库存在本任务之外的未提交文件或未推提交、upstream 缺失、push 被拒绝，或普通合并需要覆盖用户改动。
- 只有强制推送、删除远端分支、版本 bump、tag 或发布才能继续。

## 后续动作

1. 对本 memo 完成四个固定角度的只读审查；失败项修订后只复审失败维度。
2. 审查全 PASS 后，只提交并 push 本 memo，确认 `E:\Projects\wokrouter` 仓库的 `main` 分支工作区干净且与 upstream 对齐。
3. 用户在工作目录为 `E:\Projects\wokrouter` 的 Codex 任务中执行本 memo “Goal 命令”字段最终生成的仓库相对命令；该命令在 memo 持久化门通过前保持“暂不生成”。后续 goal 续作现有功能 worktree，不在 bake 阶段启动目标。
