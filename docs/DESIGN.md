# Mac 管家 —— 产品与技术方案

> 定位：一个 macOS 上的「桌面收纳 + 系统管家」二合一工具。
> 对标：iCollections（桌面整理）+ 腾讯电脑管家 Mac 版（清理 / 加速 / 启动项）。
> 技术栈：Tauri 2 + Rust（后端）+ React 19 + TypeScript + Vite（前端）。

---

## 1. 竞品拆解与差异化

| 维度 | iCollections | 腾讯电脑管家 (Mac) | Mac 管家（本项目） |
|---|---|---|---|
| 桌面分区收纳 | ✅ 核心：桌面上悬浮分区格子 | ❌ | ✅ v0.1 应用内收纳盒；v0.3 桌面层悬浮格子 |
| 自动归类规则 | ✅ | ❌ | ✅ 按扩展名 → v0.2 自定义规则 |
| 垃圾清理 | ❌ | ✅ | ✅ 白名单 + 移到废纸篓（可恢复） |
| 大文件查找 | ❌ | ✅ | ✅ |
| 内存 / CPU 监控 | ❌ | ✅ 加速球 | ✅ 仪表盘 + 进程 Top；v0.2 菜单栏实时数据 |
| 启动项管理 | ❌ | ✅ | ✅ 登录项 + LaunchAgents（launchctl 可逆禁用） |
| 应用卸载 + 残留清理 | ❌ | ✅ | 🔜 v0.2 |
| 包体积 | ~20MB | ~100MB+ | 目标 < 15MB（Tauri 使用系统 WebKit） |

**差异化方向**
1. **一个工具解决「桌面乱」和「电脑慢」**，减少常驻软件数量。
2. **安全优先**：所有删除都走废纸篓、整理操作全部可撤销、清理路径白名单校验；不做“一键加速”式的激进操作（如 `purge` 释放内存这类效果存疑的功能不做）。
3. **轻量**：Tauri 包体和内存远低于 Electron，对“管家类”产品是硬指标。

---

## 2. 功能拆解与版本规划

### v0.1（已实现）
- **系统概览**：CPU / 内存 / 磁盘使用率、健康分、进程 Top 15（按内存/CPU），可结束进程；2 秒刷新。
- **桌面收纳**：扫描 `~/Desktop`，13 类自动分类，以「收纳盒」卡片展示；双击打开、右键在 Finder 中显示；勾选分类 → 预览 → 一键整理到 `~/Desktop/桌面收纳/<分类>/`；整理日志持久化，支持撤销。
- **垃圾清理**：应用缓存、用户日志、Xcode DerivedData、iOS 备份、npm 缓存、下载中的安装包；分组勾选、展开明细；移到废纸篓；废纸篓体积与清倒。
- **大文件**：用户目录内 > 100MB / 500MB / 1GB / 4GB 的文件，显示 / 删除到废纸篓。
- **启动项**：登录项（可移除）、`~/Library/LaunchAgents`（可开关）、`/Library/LaunchAgents|Daemons`（只读）。
- **菜单栏托盘**、权限状态检测与跳转系统设置、深色模式。

### v0.2（已实现，见 v0.5）
- 应用卸载器：扫描 `/Applications`，按 Bundle ID 查找 `~/Library/{Application Support,Caches,Preferences,Containers,Group Containers,Saved Application State,LaunchAgents}` 残留。
- 自定义整理规则（关键字 / 正则 / 时间 → 目标盒子），桌面新文件自动归类（`notify` 监听 `~/Desktop`）。
- 菜单栏实时显示 CPU/内存（托盘 title），点击弹出迷你面板（类似加速球）。
- 重复文件查找（size → 部分 hash → 全量 hash 三级过滤）。
- 浏览器缓存清理（Chrome/Safari/Edge，需检测浏览器未运行）。

### v0.3：悬浮在桌面上的集合（iCollections 核心体验，已实现）
- **每个集合是一个独立的透明窗口**，浮在桌面上：毛玻璃（HudWindow）、无边框、不进 Dock / Cmd+Tab / 调度中心；
  - `NSWindow.level = -20`（高于 Finder 桌面图标层 ≈ -2147483603，低于普通窗口 0；与 iCollections 一致）。
    注意：`CGWindowLevelForKey(kCGDesktopIconWindowLevelKey)+1` 会落在壁纸下面，不能用；
  - `collectionBehavior = CanJoinAllSpaces | Stationary | IgnoresCycle`；「将集合置于前方」切到 level 3；
  - 坐标统一用 `NSScreen` 换算为「主屏左上角原点」的逻辑坐标，启动时把跑出屏幕的集合拉回可见区域。
- **集合种类**：集合（collection）/ 文件夹视图（folder）/ 拖拽释放框（dropbox）/ 硬盘面板（disk）。
- **入口**：集合右键菜单与菜单栏托盘：新建（集合 / 文件夹视图 / 拖拽释放框 / 硬盘面板）、我的集合（显示/隐藏）、重命名、颜色、图标大小、排序、折叠、自动归类规则、置于前方、隐藏集合、隐藏系统桌面图标。（空桌面右键属于 Finder，第三方无法添加菜单项。）
- **文件只存在一份（真正移动）**：
  - 桌面文件拖入集合 → 移动到 `~/桌面集合/<集合名>/`，桌面上不再保留；集合之间拖动同样是移动；重名自动加序号；
  - 从集合拖出使用 `NSDragOperationMove`，拖到桌面/Finder 后集合中不再保留；拖回桌面的文件标记为「留在桌面」，不会被自动收回；
  - 右键「移回桌面」、删除集合、清空释放框 → 文件移回桌面；有文件移不回去时保留集合，防止文件失联；
  - 重命名集合时同步重命名存放目录；
  - 非桌面位置的文件拖入集合只记录「快捷引用」（↗），不移动原文件。
- **自动归类**（可开关，首次启动询问）：按扩展名 / 关键字 / 文件夹 / 兜底规则，每 1.5 秒把桌面新文件移入对应集合；
  5 秒内刚变化的文件和 `.download/.crdownload/.part` 等下载中文件跳过；被文件夹视图引用的桌面目录不会被移走。
- 「隐藏系统桌面图标」= `defaults write com.apple.finder CreateDesktop false` + 重启 Finder（需确认）。
- 文件图标通过 `icon://` 自定义异步协议由 NSWorkspace 渲染并缓存；拖出使用 `tauri-plugin-drag`。
- WKWebView 不实现 JS `confirm()/alert()`，所有确认框改为 Rust 原生 `NSAlert`（`confirm_dialog` 命令）。
- 数据：`~/Library/Application Support/mac-butler/collections.json`。

### v0.5：锁定、资源占用与 v0.2 工具箱
- **锁定位置**：每个集合可锁定，锁定后不能拖动 / 缩放 / 双击改名，右键菜单与管理页可解锁。
- **标题字号**：12–24 px 可选，标题栏高度（左右侧为宽度）随字号自动增大；折叠尺寸、吸附计算使用同一公式。
- **吸附间隙**：0–24 px 可调，默认 4 px。
- **拖动中实时吸附**：窗口 `Moved` 事件直接触发边缘吸附（事件驱动，约 60Hz，无需轮询线程）；松手后再做一次完整吸附并推开重叠。
- **资源占用**：
  - 后台由 3 个轮询线程合并为 1 个：200ms 处理松手吸附 / 节流写盘，3 秒检查一次桌面与集合目录（只用目录 mtime，不遍历文件）。
  - 图标内存缓存改为 16 MB 字节预算（原为最多 3000 张）；磁盘缓存启动时清理 14 天未使用的内容，设置页可手动清理。
  - release 开启 `lto + strip + codegen-units=1 + panic=abort`，安装包 5.4 MB → 3.9 MB；空载内存 52 MB → 32 MB，线程 23 → 17。
- **应用卸载器**：列出 `/Applications`、`~/Applications`，解析 Bundle ID / 版本 / 体积 / 是否运行；按 Bundle ID 查找 `~/Library` 下的数据、缓存、偏好、容器、登录代理、日志等残留，应用与残留一起移到废纸篓。
- **重复文件**：大小 → 首块哈希（≤1 MB）→ 全文哈希三级过滤，按可释放空间排序；默认保留每组第一份，删除进废纸篓。
- **浏览器缓存**：Chrome / Edge / Safari / Firefox / Arc，展示缓存体积；运行中的浏览器拒绝清理。
- **右键菜单精简**：视觉相关设置收进一级菜单「外观」（颜色 / 透明度 / 标题字号 / 标题位置 / 显示方式 / 图标大小 / 排序方式）；移除与标题栏折叠按钮重复的「折叠」、与托盘重复的「我的集合」和全局开关。
- **菜单重建去抖**：托盘菜单只在名称 / 显示状态 / 吸附等设置变化时重建，文件移动不再触发。
- 移除 v0.1 遗留的窗口内整理命令（`scan_desktop` / `organize_desktop` / `undo_organize`）。
- **最近移动 + 撤销**：记录最近 200 次文件移动（放入 / 自动归类 / 移回桌面 / 拖出），可在管理页逐条撤销，文件移回原目录。

### v0.4：集合外观与布局
- **透明度**：每个集合 0–100%，同时作用于网页背景（CSS 变量 `--a`）与原生毛玻璃层（`NSVisualEffectView.alphaValue`，tag 91376254）；图标与文字不受影响。
- **吸附**：拖动结束（`NSEvent.pressedMouseButtons` 松开）后，集合边缘在 18px 内贴齐其他集合（间隙 8px）或屏幕边缘；若仍与其他集合重叠，推到重叠最小的一侧。可在菜单/设置中关闭。
- **标题位置**：顶部 / 底部 / 左侧 / 右侧（竖排，折叠方向随之变为水平）/ 合集内（悬浮胶囊）。
- **显示方式**：图标 / 列表（名称、大小、宽度足够时显示日期）/ 迷你（紧凑单行、更矮的标题栏）。
- **标题字号**：默认跟随样式（13 px，迷你 11 px），可单独设为 12–24 px。
- **吸附间隙**：默认 4 px，可在 0–24 px 之间调整（菜单「吸附间隙」，或设置页滑块）；0 = 无缝贴合。

### v1.0
- 签名 + 公证，Sparkle 风格自动更新（`tauri-plugin-updater`），开机自启（`tauri-plugin-autostart`）。
- 特权操作（清理系统级缓存、管理 LaunchDaemons）使用 `SMAppService` 注册 privileged helper，而非在主进程 `sudo`。

---

### v0.6：菜单栏、实时监听、小组件
- **开机自启**：写入 `~/Library/LaunchAgents/com.macbutler.app.plist`（无需签名；从 DMG 直接运行会拒绝并提示先安装）。开关在管理页 / 托盘菜单。
- **菜单栏常驻**：托盘标题显示 `CPU% · 内存%`（可关闭）；左键点击弹出迷你面板（首次点击才创建窗口，平时不占内存），右键仍是完整菜单。
- **FSEvents 监听**：直接调用 CoreServices 的 FSEvents 监听 `~/Desktop` 与 `~/桌面集合`，变化即时刷新并触发自动归类；原 3 秒轮询降级为 30 秒兜底。
- **自动归类预演**：开启前先列出「哪些文件会移到哪个集合」，确认后才移动；首次启动默认关闭，不再直接移动文件。
- **桌面小组件**：系统概览 / 内存排行两种，浮在桌面（与集合同层），小（260×170）/ 中（340×230）/ 大（460×330）三档，也可拖动边缘自定义；右键改大小、隐藏、删除；入口在托盘「新建小组件」、管理页和概览页的「放到桌面」。
- **内存排行**：按 .app 包聚合多进程内存，Dashboard 与管理页小组件共用；进程级明细仍保留在「资源占用 Top 15」。
- 资源占用：空载约 30 MB / 23 线程（含 WebView 常驻线程）。

### v0.6.1
- 小组件窗口加入 capabilities（`wg-*`、`mini`），恢复标题栏拖动；之前只有 `main` / `col-*` 有 `start-dragging` 权限。
- 「文件夹视图」更名为「文件夹合集」：创建时选择文件夹，展示该文件夹内容；更换文件夹后 FSEvents 监听目录会即时重建，文件变化立刻刷新。

### v0.6.3：自定义表情
- 集合里的文件 / 文件夹可右键「设置表情…」，用 emoji 替代系统图标；支持常用表情网格，也可直接粘贴任意 emoji。
- 表情按路径保存在 `collections.json` 的 `emojis` 字段；文件在桌面与集合之间移动、撤销时表情会一起迁移。
- 仅影响 Mac 管家内的显示（桌面集合 / 文件夹合集），不会修改 Finder 里的图标。

### v0.6.4：桌面集合快捷方式
- 原因说明：文件存放在 `~/桌面集合/<集合名>/`，顶层只有集合子文件夹，且不在「桌面」下，所以网页/应用的文件选择框里看不到。
- 新增可选开关「在桌面显示「桌面集合」快捷方式」：在 `~/Desktop/桌面集合` 创建指向 `~/桌面集合` 的软链接，各类选文件框可从「桌面」直接进入。
- 该软链接不会被视为桌面文件参与自动归类或集合列表。

## 3. 架构

```
┌──────────────────────────── Tauri App ────────────────────────────┐
│  WebView (React)                                                  │
│   pages/Dashboard  Collections  CollectionWindow  Cleaner  Startup │
│        │   src/api.ts  —— 类型化 invoke 封装                       │
│  ──────┼──────────────── IPC (invoke / JSON) ──────────────────── │
│  Rust  ▼                                                          │
│   lib.rs       命令注册、托盘、权限检测                             │
│   monitor.rs   sysinfo：CPU/内存/磁盘/进程（共享 State，两次采样） │
│   desktop.rs   桌面扫描、分类、整理计划、移动日志与撤销             │
│   collections.rs 桌面集合：窗口层级、菜单、拖放、移动、自动归类     │
│   icons.rs     NSWorkspace 图标/缩略图渲染（icon:// 协议）          │
│   cleaner.rs   垃圾规则表、体积统计、废纸篓、大文件                 │
│   startup.rs   plist 解析、launchctl、System Events 登录项          │
│   util.rs      路径白名单校验、磁盘占用统计、命令执行               │
└───────────────────────────────────────────────────────────────────┘
```

- 所有耗时操作（扫描、清理）为 `async` 命令，在 Tauri 线程池中执行，不阻塞 UI。
- 体积按 `st_blocks * 512` 统计（真实占用，与 Finder “磁盘上” 一致，正确处理稀疏文件 / APFS 克隆）。
- 前端不直接拿文件系统权限（capabilities 只开 `core:default`），一切经过 Rust 命令和白名单校验。

---

## 4. macOS 权限清单（关键坑）

| 能力 | 权限 | 处理方式 |
|---|---|---|
| 读取桌面 / 下载 / 文稿 | TCC「文件与文件夹」 | 首次访问系统自动弹窗；侧边栏显示状态 + 跳转设置 |
| 读取废纸篓、邮件、Safari 缓存、部分 Containers | 「完全磁盘访问权限」 | 无法弹窗申请，只能引导用户手动添加；检测 `~/.Trash` 可读性 |
| 读取 / 移除登录项、清倒废纸篓 | 「自动化」(Apple Events → System Events / Finder) | 需在 Info.plist 写 `NSAppleEventsUsageDescription`（v0.2 补上） |
| 结束他人 / 系统进程、修改 `/Library/Launch*` | root | v0.1 只读；v1.0 用 privileged helper |
| 桌面层窗口 | 无需权限，需 `macOSPrivateApi` | 已开启；**注意：开启 private API 后不能上架 Mac App Store** |

**分发策略结论**：本产品需要完全磁盘访问、Apple Events、非沙盒文件操作，**不适合上架 Mac App Store**（沙盒限制），走官网 DMG + Developer ID 签名 + 公证。腾讯电脑管家、CleanMyMac 官网版也是这个模式。

---

## 5. 安全设计
1. **白名单**：`clean_items` 只接受位于规则目录之下（且不等于根本身）的路径，并 `canonicalize` 防止 `../` 和符号链接逃逸。
2. **可恢复**：清理 / 大文件删除一律 `trash::delete` 移到废纸篓；只有“清倒废纸篓”不可恢复，且二次确认。
3. **可撤销**：桌面整理记录写入 `~/Library/Application Support/mac-butler/organize-history.json`，按批次撤销；同名冲突自动重命名。
4. **启动项可逆**：使用 `launchctl disable`（写入 launchd 覆盖数据库），不删除 plist。
5. 不结束自身进程；系统进程结束失败时给出明确提示。

---

## 6. 开发与构建

```bash
# 前置：Xcode Command Line Tools、Rust（rustup）、bun 或 npm
cd mac-butler
bun install
bun run tauri dev      # 开发模式（热更新）
bun run tauri build    # 产出 .app 和 .dmg 到 src-tauri/target/release/bundle/
```

目录结构：
```
mac-butler/
├── docs/DESIGN.md          本文档
├── index.html / vite.config.ts / tsconfig.json / package.json
├── src/                    React 前端
│   ├── api.ts              Rust 命令的类型化封装
│   ├── App.tsx             侧边栏 + 路由 + 权限状态
│   ├── pages/              Dashboard / Collections / CollectionWindow / Cleaner / Startup
│   └── styles.css          macOS 风格样式（含深色模式）
└── src-tauri/              Rust 后端
    ├── Cargo.toml / tauri.conf.json / capabilities/ / icons/
    └── src/ lib.rs main.rs util.rs monitor.rs desktop.rs collections.rs icons.rs cleaner.rs startup.rs
```

## 7. 已知限制（v0.3）
- 放进集合的文件实际位于 `~/桌面集合/`，其他应用（如「最近使用」、脚本）按旧桌面路径访问会找不到。
- `tauri build` 的 DMG 美化脚本偶尔因临时镜像「资源忙」失败，可用 `hdiutil create -srcfolder … -format UDZO` 直接打包。
- 未签名构建首次打开需右键 → 打开；TCC 权限绑定签名，重新构建后可能需要重新授权。
- 登录项通过 AppleScript 读取，macOS 13+ 中由 `SMAppService` 注册的后台项目不会出现在列表中（需 `sfltool dumpbtm`，需 root）。
