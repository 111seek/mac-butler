//! 桌面集合（对标 iCollections）。
//!
//! 每个集合都是一个独立的透明窗口，固定在「桌面图标层之上、普通窗口之下」，
//! 跟随所有桌面空间、不出现在 Cmd+Tab / 调度中心里。
//! - 集合（collection）：桌面文件放入集合（手动拖入或按规则自动归类）时，会被**真正移动**到
//!   `~/桌面集合/<集合名>/`，桌面上不再保留，避免桌面与集合中出现同一文件；「移回桌面」/删除集合时再移回。
//!   其他位置（非桌面）的文件拖入只记录「快捷引用」，不移动。
//! - 文件夹视图（folder）：展示任意文件夹内容，拖入 = Finder 语义（同卷移动 / 跨卷拷贝）
//! - 拖拽释放框（dropbox）：暂存文件，桌面文件同样移入其目录，其他位置的文件记引用
//! - 硬盘面板（disk）：展示已挂载硬盘及容量

use crate::util::{self, home};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication, NSEvent, NSOpenPanel, NSScreen, NSWindow, NSWindowCollectionBehavior};
use objc2_foundation::NSString;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent, Wry};

pub const HEADER_H: f64 = 34.0;
const MINI_HEADER_H: f64 = 24.0;
/// 吸附距离 / 与屏幕边缘的间隙
const SNAP_DIST: f64 = 18.0;
const SCREEN_MARGIN: f64 = 8.0;
/// 集合之间的吸附间隙（可在菜单 / 设置中调整）
const DEFAULT_SNAP_GAP: f64 = 4.0;

fn default_gap() -> f64 { DEFAULT_SNAP_GAP }

/// 标题栏高度：字号变大时自动增高（0 = 使用默认外观尺寸）。
fn header_h(c: &Collection) -> f64 {
    let mini = c.view == "mini";
    let base = if mini { MINI_HEADER_H } else { HEADER_H };
    if c.title_size == 0 {
        base
    } else {
        base.max(c.title_size as f64 + if mini { 12.0 } else { 16.0 })
    }
}

/// 左右侧标题的栏宽，规则同上。
fn head_w(c: &Collection) -> f64 {
    header_h(c)
}

/// 标题在左右两侧时，折叠方向是水平的
fn side_title(c: &Collection) -> bool {
    matches!(c.title_pos.as_str(), "left" | "right")
}

/// 窗口当前在屏幕上的实际尺寸（考虑折叠）。
fn shown_size(c: &Collection) -> (f64, f64) {
    match (c.collapsed, side_title(c)) {
        (false, _) => (c.w, c.h),
        (true, true) => (head_w(c), c.h),
        (true, false) => (c.w, header_h(c)),
    }
}
const COLORS: &[(&str, &str)] = &[
    ("#3b82f6", "蓝色"), ("#8b5cf6", "紫色"), ("#ec4899", "粉色"), ("#ef4444", "红色"),
    ("#f59e0b", "橙色"), ("#10b981", "绿色"), ("#06b6d4", "青色"), ("#64748b", "灰色"),
];

/// 集合窗口层级：低于所有普通窗口（0），高于壁纸、Finder 桌面图标和桌面小组件。
/// Finder 桌面图标层约为 -2147483603，普通窗口为 0；-20 与 iCollections 取值相同，
/// 既不会被 Finder 桌面盖住，也不会挡住任何普通应用窗口。
pub(crate) const DESKTOP_COLLECTION_LEVEL: isize = -20;
/// 「将集合置于前方」时使用浮动层（NSFloatingWindowLevel）
const FRONT_LEVEL: isize = 3;

// ---------------------------------------------------------------- 数据模型

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub color: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub collapsed: bool,
    /// 自动归类：扩展名（小写，不含点）
    #[serde(default)]
    pub exts: Vec<String>,
    /// 自动归类：文件名包含的关键字
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 自动归类：收纳桌面上的文件夹
    #[serde(default)]
    pub dirs: bool,
    /// 兜底集合：收纳所有未被其他集合认领的桌面文件
    #[serde(default)]
    pub catch_all: bool,
    #[serde(default)]
    pub folder: Option<String>,
    /// 非桌面文件的引用（集合中的快捷方式 / 释放框中的暂存文件）
    #[serde(default)]
    pub refs: Vec<String>,
    /// 集合 / 释放框的实际存放目录（~/桌面集合/<名称>），首次放入文件时创建
    #[serde(default)]
    pub dir: Option<String>,
    #[serde(default = "default_icon")]
    pub icon_size: u32,
    #[serde(default)]
    pub sort: String,
    /// 背景透明度 0–100：0 = 默认毛玻璃背景，100 = 背景完全透明（只剩图标和文字）
    #[serde(default)]
    pub transparency: u32,
    /// 标题位置："" = 顶部，"bottom" / "left" / "right" / "inside"（浮在集合内）
    #[serde(default)]
    pub title_pos: String,
    /// 显示方式："" = 图标，"list" = 列表，"mini" = 迷你
    #[serde(default)]
    pub view: String,
    /// 标题字号（px）；0 = 跟随显示方式使用默认大小
    #[serde(default)]
    pub title_size: u32,
    /// 锁定位置：不能拖动 / 缩放 / 双击改名，防止误触
    #[serde(default)]
    pub locked: bool,
}
fn yes() -> bool { true }
fn default_icon() -> u32 { 48 }

#[derive(Serialize, Deserialize, Default)]
pub struct Store {
    pub collections: Vec<Collection>,
    /// 桌面文件路径 -> 集合 id。"" = 交给兜底集合，"-" = 不归入任何集合
    #[serde(default)]
    pub assign: HashMap<String, String>,
    #[serde(default)]
    pub on_top: bool,
    #[serde(default)]
    pub hidden_all: bool,
    #[serde(default)]
    pub initialized: bool,
    /// 是否按规则自动把桌面文件移入集合。None = 尚未询问用户
    #[serde(default)]
    pub auto_move: Option<bool>,
    /// 关闭集合之间的吸附对齐（默认开启）
    #[serde(default)]
    pub no_snap: bool,
    /// 吸附后两个集合之间的间隙（px）
    #[serde(default = "default_gap")]
    pub snap_gap: f64,
    /// 最近的移动操作（最多保留 200 条）
    #[serde(default)]
    pub history: Vec<MoveOp>,
    /// 在菜单栏显示 CPU / 内存
    #[serde(default = "yes")]
    pub tray_stats: bool,
}

/// 一次文件移动操作（用于「最近操作」与撤销）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MoveOp {
    pub id: u64,
    pub time: u64,
    pub from: String,
    pub to: String,
    pub name: String,
    pub reason: String,
}

pub struct CollectionsState {
    pub store: Mutex<Store>,
    pub dirty: AtomicBool,
    /// 刚被用户拖动过、等待松开鼠标后吸附的窗口：集合 id -> 最后一次移动时间
    pub moved: Mutex<HashMap<String, std::time::Instant>>,
    /// 实时吸附的冷却时间，避免频繁挪动窗口
    pub snapping: Mutex<HashMap<String, std::time::Instant>>,
    /// 上一次构建托盘菜单时的菜单相关状态（相同则不重建）
    pub menu_sig: Mutex<Option<u64>>,
}

fn store_path() -> PathBuf {
    let dir = home().join("Library/Application Support/mac-butler");
    let _ = fs::create_dir_all(&dir);
    dir.join("collections.json")
}

pub fn load_state() -> CollectionsState {
    let store = fs::read(store_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    CollectionsState { store: Mutex::new(store), dirty: AtomicBool::new(false), moved: Mutex::new(HashMap::new()), snapping: Mutex::new(HashMap::new()), menu_sig: Mutex::new(None) }
}

fn save(store: &Store) {
    if let Ok(b) = serde_json::to_vec_pretty(store) {
        let tmp = store_path().with_extension("json.tmp");
        if fs::write(&tmp, b).is_ok() {
            let _ = fs::rename(tmp, store_path());
        }
    }
}

fn new_id() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    format!("{:x}", n)
}

/// 需要监听的目录：桌面、集合存放根目录、以及所有文件夹合集指向的目录。
fn watch_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut v = vec![desktop_dir(), store_root()];
    {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        v.extend(
            s.collections
                .iter()
                .filter(|c| c.kind == "folder")
                .filter_map(|c| c.folder.as_ref().map(PathBuf::from)),
        );
    }
    v
}

pub(crate) fn refresh_watcher(app: &AppHandle) {
    crate::watcher::restart(&watch_dirs(app));
}

pub(crate) fn desktop_dir() -> PathBuf {
    home().join("Desktop")
}

fn label_of(id: &str) -> String {
    format!("col-{id}")
}

// ---------------------------------------------------------------- 桌面文件与归类

#[derive(Serialize, Clone)]
pub struct Item {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
    pub ext: String,
    pub is_ref: bool,
}

fn make_item(path: &Path, is_ref: bool) -> Option<Item> {
    let meta = fs::metadata(path).ok()?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let is_dir = meta.is_dir() && ext != "app";
    Some(Item {
        name: path.file_name()?.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        is_dir,
        size: if meta.is_dir() { 0 } else { meta.len() },
        modified: util::modified_secs(&meta),
        ext,
        is_ref,
    })
}

fn list_dir(dir: &Path) -> Vec<Item> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    rd.flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| make_item(&e.path(), false))
        .collect()
}

fn rule_match(c: &Collection, it: &Item) -> bool {
    if it.is_dir {
        return c.dirs;
    }
    if c.exts.iter().any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&it.ext)) {
        return true;
    }
    let lower = it.name.to_lowercase();
    c.keywords.iter().any(|k| !k.trim().is_empty() && lower.contains(&k.trim().to_lowercase()))
}

/// 计算一个桌面文件归属哪个集合。
fn owner_of(store: &Store, it: &Item) -> Option<String> {
    let catch_all = || store.collections.iter().find(|c| c.kind == "collection" && c.catch_all).map(|c| c.id.clone());
    if let Some(a) = store.assign.get(&it.path) {
        match a.as_str() {
            "-" => return None,
            "" => return catch_all(),
            id if store.collections.iter().any(|c| c.id == id && c.kind == "collection") => return Some(id.to_string()),
            _ => {}
        }
    }
    store
        .collections
        .iter()
        .filter(|c| c.kind == "collection" && !c.catch_all)
        .find(|c| rule_match(c, it))
        .map(|c| c.id.clone())
        .or_else(catch_all)
}

// ---------------------------------------------------------------- 存放目录与文件移动

pub(crate) fn store_root() -> PathBuf {
    home().join("桌面集合")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 记录一次移动，供「最近操作」展示并支持撤销。
fn log_move(s: &mut Store, from: &Path, to: &Path, reason: &str) {
    let name = from.file_name().or_else(|| to.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    s.history.push(MoveOp {
        id,
        time: now_secs(),
        from: from.to_string_lossy().into_owned(),
        to: to.to_string_lossy().into_owned(),
        name,
        reason: reason.into(),
    });
    if s.history.len() > 200 {
        let drop_n = s.history.len() - 200;
        s.history.drain(0..drop_n);
    }
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if c == '/' || c == ':' { '-' } else { c }).collect();
    let s = s.trim().trim_start_matches('.').trim().to_string();
    if s.is_empty() { "集合".into() } else { s }
}

/// 集合的存放目录，不存在时创建。
fn ensure_dir(s: &mut Store, id: &str) -> Result<PathBuf, String> {
    let others: Vec<PathBuf> = s.collections.iter().filter(|c| c.id != id).filter_map(|c| c.dir.as_ref().map(PathBuf::from)).collect();
    let c = s.collections.iter_mut().find(|c| c.id == id).ok_or("集合不存在")?;
    let dir = match &c.dir {
        Some(d) => PathBuf::from(d),
        None => {
            let base = sanitize(&c.name);
            (1..)
                .map(|n| store_root().join(if n == 1 { base.clone() } else { format!("{base} {n}") }))
                .find(|p| !others.contains(p) && (!p.exists() || p.is_dir()))
                .unwrap()
        }
    };
    fs::create_dir_all(&dir).map_err(|e| format!("无法创建 {}：{e}", dir.display()))?;
    c.dir = Some(dir.to_string_lossy().into_owned());
    Ok(dir)
}

/// 把文件/文件夹移动到 `dir` 下（重名自动加序号），返回新路径。
fn move_into(src: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = src.file_name().ok_or("路径无效")?.to_string_lossy().into_owned();
    let dest = crate::desktop::unique_target(dir, &name);
    if fs::rename(src, &dest).is_err() {
        // 跨卷等情况交给 mv
        util::run("mv", &["-n", &src.to_string_lossy(), &dest.to_string_lossy()]).map_err(|e| format!("{name}：{e}"))?;
    }
    if src.exists() || !dest.exists() {
        return Err(format!("{name}：移动失败"));
    }
    Ok(dest)
}

/// 刚被创建/改名/移动（5 秒内）或明显还在下载中的文件先不动，避免和下载、拖放操作抢文件。
fn is_busy(it: &Item) -> bool {
    if matches!(it.ext.as_str(), "download" | "crdownload" | "part" | "partial" | "tmp") {
        return true;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    fs::symlink_metadata(&it.path).map(|m| now - m.ctime() < 5).unwrap_or(true)
}

/// 计算桌面上哪些文件应按规则移入哪个集合。
fn plan_sweep(s: &Store) -> Vec<(String, String)> {
    list_dir(&desktop_dir())
        .into_iter()
        .filter(|it| s.assign.get(&it.path).map(|v| v != "-").unwrap_or(true))
        // 文件夹视图正在展示的目录（或其上级）不能被挪走
        .filter(|it| !s.collections.iter().any(|c| c.kind == "folder" && c.folder.as_deref().is_some_and(|f| Path::new(f).starts_with(&it.path))))
        .filter_map(|it| owner_of(s, &it).map(|id| (it.path, id)))
        .collect()
}

/// 自动归类：把桌面上符合规则的文件真正移进对应集合的目录。返回是否有文件被移动。
fn sweep(app: &AppHandle) -> bool {
    let st = app.state::<CollectionsState>();
    let mut s = st.store.lock().unwrap();
    let before = s.assign.len();
    s.assign.retain(|p, _| Path::new(p).exists());
    let mut moved = false;
    if s.auto_move == Some(true) {
        for (path, id) in plan_sweep(&s) {
            let Some(it) = make_item(Path::new(&path), false) else { continue };
            if is_busy(&it) {
                continue;
            }
            match ensure_dir(&mut s, &id).and_then(|d| move_into(Path::new(&path), &d)) {
                Ok(dest) => {
                    s.assign.remove(&path);
                    let reason = s.collections.iter().find(|c| c.id == id).map(|c| format!("自动归入「{}」", c.name)).unwrap_or_else(|| "自动归类".into());
                    log_move(&mut s, Path::new(&path), &dest, &reason);
                    moved = true;
                }
                Err(e) => eprintln!("[collections] 自动归类失败 {e}"),
            }
        }
    }
    if moved || s.assign.len() != before {
        save(&s);
    }
    moved
}

#[derive(Serialize)]
pub struct SweepItem {
    pub path: String,
    pub name: String,
    pub target: String,
    pub target_id: String,
    pub is_dir: bool,
}

/// 预演：列出「如果现在自动归类」会被移走哪些文件。
#[tauri::command]
pub async fn sweep_preview(app: AppHandle) -> Vec<SweepItem> {
    let st = app.state::<CollectionsState>();
    let s = st.store.lock().unwrap();
    plan_sweep(&s)
        .into_iter()
        .filter_map(|(path, id)| {
            let it = make_item(Path::new(&path), false)?;
            let target = s.collections.iter().find(|c| c.id == id)?.name.clone();
            Some(SweepItem { path, name: it.name, target, target_id: id, is_dir: it.is_dir })
        })
        .collect()
}

/// 手动执行一次自动归类（预演确认后调用），返回计划整理的文件数。
#[tauri::command]
pub async fn run_sweep(app: AppHandle) -> usize {
    let n = {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        plan_sweep(&s).len()
    };
    {
        let st = app.state::<CollectionsState>();
        st.store.lock().unwrap().auto_move = Some(true);
    }
    sweep(&app);
    {
        let st = app.state::<CollectionsState>();
        st.store.lock().unwrap().auto_move = Some(false);
    }
    commit(&app);
    n
}

/// 开启 / 关闭自动归类（管理页在预演确认后调用）。
#[tauri::command]
pub async fn set_auto_move(app: AppHandle, enabled: bool) {
    crate::collections::set_auto_move_inner(&app, enabled);
}

fn return_collection_files_inner(app: &AppHandle, id: &str) -> Result<usize, String> {
    let (dir, name) = {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        let c = s.collections.iter().find(|c| c.id == id).ok_or("集合不存在")?;
        (c.dir.clone(), c.name.clone())
    };
    let mut moved = 0usize;
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if let Some(c) = s.collections.iter_mut().find(|c| c.id == id) {
            c.refs.clear();
        }
        if let Some(d) = dir {
            moved = fs::read_dir(&d)
                .map(|r| r.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).count())
                .unwrap_or(0);
            return_all_reason(&mut s, Path::new(&d), &format!("从「{name}」全部移回桌面"))?;
        }
        save(&s);
    }
    commit(app);
    Ok(moved)
}

/// 把某个集合里的文件全部移回桌面（集合保留，可继续使用）。
#[tauri::command]
pub async fn return_collection_files(app: AppHandle, id: String) -> Result<usize, String> {
    return_collection_files_inner(&app, &id)
}

/// 把目录中的所有内容移回桌面，并标记为「留在桌面」，避免被自动归类再次收走。
fn return_all(s: &mut Store, dir: &Path) -> Result<(), String> {
    return_all_reason(s, dir, "移回桌面")
}

fn return_all_reason(s: &mut Store, dir: &Path, reason: &str) -> Result<(), String> {
    let mut errs = vec![];
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.file_name() == ".DS_Store" {
            let _ = fs::remove_file(e.path());
            continue;
        }
        let src = e.path();
        match move_into(&src, &desktop_dir()) {
            Ok(d) => {
                s.assign.insert(d.to_string_lossy().into_owned(), "-".into());
                log_move(s, &src, &d, reason);
            }
            Err(x) => errs.push(x),
        }
    }
    if errs.is_empty() { Ok(()) } else { Err(errs.join("；")) }
}

fn sort_items(items: &mut [Item], sort: &str) {
    match sort {
        "name" => items.sort_by_key(|a| a.name.to_lowercase()),
        "kind" => items.sort_by(|a, b| (!a.is_dir, &a.ext, a.name.to_lowercase()).cmp(&(!b.is_dir, &b.ext, b.name.to_lowercase()))),
        _ => items.sort_by(|a, b| b.modified.cmp(&a.modified)),
    }
}

fn snapshot(app: &AppHandle) -> (Vec<Collection>, bool, bool) {
    let st = app.state::<CollectionsState>();
    let s = st.store.lock().unwrap();
    (s.collections.clone(), s.on_top, s.hidden_all)
}

fn find(app: &AppHandle, id: &str) -> Option<Collection> {
    snapshot(app).0.into_iter().find(|c| c.id == id)
}

/// 修改某个集合并提交。
fn with_col(app: &AppHandle, id: &str, f: impl FnOnce(&mut Collection)) {
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if let Some(c) = s.collections.iter_mut().find(|c| c.id == id) {
            f(c);
        }
    }
    commit(app);
}

/// 持久化 + 通知所有窗口刷新 + 同步窗口显示状态 + 重建托盘菜单。
pub fn commit(app: &AppHandle) {
    {
        let st = app.state::<CollectionsState>();
        save(&st.store.lock().unwrap());
    }
    let _ = app.emit("collections-changed", ());
    sync_windows(app);
    rebuild_tray(app);
}

// ---------------------------------------------------------------- 窗口

/// 所有屏幕的可用区域，换算成「主屏左上角为原点、y 向下」的全局逻辑坐标（与 Tauri LogicalPosition 一致）。
/// 必须在主线程调用；多屏且缩放比例不同时，这是唯一可靠的坐标来源。
pub(crate) fn screen_rects() -> Vec<(f64, f64, f64, f64)> {
    let Some(mtm) = MainThreadMarker::new() else { return vec![] };
    let screens = NSScreen::screens(mtm);
    let Some(primary) = screens.firstObject() else { return vec![] };
    let ph = primary.frame().size.height;
    screens
        .iter()
        .map(|sc| {
            let f = sc.visibleFrame();
            (f.origin.x, ph - f.origin.y - f.size.height, f.size.width, f.size.height)
        })
        .collect()
}

pub(crate) fn cursor_logical() -> Option<(f64, f64)> {
    let mtm = MainThreadMarker::new()?;
    let ph = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    let p = NSEvent::mouseLocation();
    Some((p.x, ph - p.y))
}

/// 保证窗口标题栏落在某块屏幕内；完全跑出屏幕时放回主屏右上角。
pub(crate) fn clamp_to_screens(x: f64, y: f64, w: f64, _h: f64, rects: &[(f64, f64, f64, f64)]) -> (f64, f64) {
    if rects.is_empty() {
        return (x, y);
    }
    let (cx, cy) = (x + w / 2.0, y + HEADER_H / 2.0);
    let r = rects
        .iter()
        .find(|(rx, ry, rw, rh)| cx >= *rx && cx <= rx + rw && cy >= *ry && cy <= ry + rh)
        .copied();
    match r {
        Some((rx, ry, rw, rh)) => (x.clamp(rx, (rx + rw - w).max(rx)), y.clamp(ry, (ry + rh - HEADER_H).max(ry))),
        None => {
            let (rx, ry, rw, _) = rects[0];
            ((rx + rw - w - 40.0).max(rx), ry + 40.0)
        }
    }
}

/// 把窗口放到桌面层（或「置于前方」时放到浮动层），并固定在所有桌面空间。
/// 应用锁定状态：锁定后窗口不能拖动 / 缩放。
fn apply_lock(app: &AppHandle, id: &str) {
    if let (Some(c), Some(win)) = (find(app, id), app.get_webview_window(&label_of(id))) {
        let _ = win.set_resizable(!c.locked);
    }
}

fn apply_level(w: &WebviewWindow, on_top: bool) {
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        if let Ok(p) = w2.ns_window() {
            let nsw: &NSWindow = unsafe { &*(p as *const NSWindow) };
            nsw.setLevel(if on_top { FRONT_LEVEL } else { DESKTOP_COLLECTION_LEVEL });
            nsw.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
        }
    });
}

/// window-vibrancy 给毛玻璃 NSVisualEffectView 打的 tag（tauri 的 `effects` 内部使用该 crate）
const BLUR_VIEW_TAG: isize = 91376254;

/// 调整原生毛玻璃层的不透明度（毛玻璃无法用 CSS 控制，必须改 NSView.alphaValue）。
fn apply_transparency(w: &WebviewWindow, transparency: u32) {
    let w2 = w.clone();
    let alpha = 1.0 - transparency.min(100) as f64 / 100.0;
    let _ = w.run_on_main_thread(move || {
        if let Ok(p) = w2.ns_window() {
            let nsw: &NSWindow = unsafe { &*(p as *const NSWindow) };
            if let Some(blur) = nsw.contentView().and_then(|v| v.viewWithTag(BLUR_VIEW_TAG)) {
                blur.setAlphaValue(alpha);
                blur.setHidden(alpha <= 0.0);
            } else {
                eprintln!("[collections] 未找到毛玻璃层，透明度只作用于网页背景");
            }
        }
    });
}

fn open_window(app: &AppHandle, c: &Collection, on_top: bool, rename: bool) {
    let label = label_of(&c.id);
    if app.get_webview_window(&label).is_some() {
        return;
    }
    let url = format!("index.html#/collection/{}{}", c.id, if rename { "?rename=1" } else { "" });
    let (sw, sh) = shown_size(c);
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(&c.name)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .skip_taskbar(true)
        .resizable(!c.locked)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .focused(false)
        .position(c.x, c.y)
        .inner_size(sw, sh)
        .min_inner_size(MINI_HEADER_H, MINI_HEADER_H)
        .effects(EffectsBuilder::new().effect(Effect::HudWindow).state(EffectState::Active).radius(12.0).build())
        .build();
    match built {
        Ok(w) => {
            apply_level(&w, on_top);
            apply_transparency(&w, c.transparency);
        }
        Err(e) => eprintln!("创建集合窗口失败: {e}"),
    }
}

/// 让实际打开的窗口与数据保持一致（注意：不能持有锁时创建窗口，窗口事件也要拿锁）。
pub fn sync_windows(app: &AppHandle) {
    let (cols, on_top, hidden_all) = snapshot(app);
    for c in &cols {
        let want = c.visible && !hidden_all;
        let win = app.get_webview_window(&label_of(&c.id));
        match (want, win) {
            (true, None) => open_window(app, c, on_top, false),
            (false, Some(w)) => { let _ = w.destroy(); }
            _ => {}
        }
    }
    for (label, w) in app.webview_windows() {
        if let Some(id) = label.strip_prefix("col-") {
            if !cols.iter().any(|c| c.id == id) {
                let _ = w.destroy();
            }
        }
    }
}

fn apply_level_all(app: &AppHandle) {
    let on_top = snapshot(app).1;
    for (label, w) in app.webview_windows() {
        if label.starts_with("col-") {
            apply_level(&w, on_top);
        }
    }
}

/// 记录窗口拖动 / 缩放后的位置尺寸；集合窗口被关闭（Cmd+W）时改为隐藏。
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let label = window.label().to_string();
    let Some(id) = label.strip_prefix("col-").map(str::to_string) else { return };
    let app = window.app_handle();
    let st = app.state::<CollectionsState>();
    let scale = window.scale_factor().unwrap_or(1.0);
    match event {
        WindowEvent::Moved(p) => {
            let l = p.to_logical::<f64>(scale);
            if let Some(c) = st.store.lock().unwrap().collections.iter_mut().find(|c| c.id == id) {
                c.x = l.x;
                c.y = l.y;
            }
            st.dirty.store(true, Ordering::Relaxed);
            st.moved.lock().unwrap().insert(id.clone(), std::time::Instant::now());
            // 拖动过程中实时贴齐（事件驱动，最高约 60Hz，无需轮询线程）
            snap_live(app, &id);
        }
        WindowEvent::Resized(s) => {
            let l = s.to_logical::<f64>(scale);
            if let Some(c) = st.store.lock().unwrap().collections.iter_mut().find(|c| c.id == id) {
                // 折叠状态下只记录没被折叠的那一边
                let side = side_title(c);
                if !c.collapsed || side {
                    if l.height >= 60.0 { c.h = l.height; }
                }
                if !c.collapsed || !side {
                    if l.width >= 80.0 { c.w = l.width; }
                }
            }
            st.dirty.store(true, Ordering::Relaxed);
        }
        // 集合是桌面组件，没有关闭按钮：忽略 Cmd+W / 退出时的关闭请求，
        // 否则退出应用会把所有集合都记成“已隐藏”。隐藏请用菜单。
        WindowEvent::CloseRequested { api, .. } => api.prevent_close(),
        _ => {}
    }
}

// ---------------------------------------------------------------- 首次启动 & 后台任务

pub fn init(app: &AppHandle) {
    let _ = fs::create_dir_all(store_root());
    let need_defaults = !app.state::<CollectionsState>().store.lock().unwrap().initialized;
    if need_defaults {
        let (sx, sy, sw0, _) = screen_rects().first().copied().unwrap_or((0.0, 25.0, 1440.0, 875.0));
        let sw = sx + sw0;
        let (w, h, gap) = (300.0, 210.0, 16.0);
        let mk = |name: &str, color: &str, col: f64, row: f64, exts: &[&str], dirs: bool, catch_all: bool| Collection {
            id: format!("{}{}", new_id(), name.len()),
            name: name.into(),
            kind: "collection".into(),
            color: color.into(),
            x: sw - (w + 24.0) - col * (w + gap),
            y: sy + 16.0 + row * (h + gap),
            w,
            h,
            visible: true,
            collapsed: false,
            exts: exts.iter().map(|s| s.to_string()).collect(),
            keywords: vec![],
            dirs,
            catch_all,
            folder: None,
            refs: vec![],
            dir: None,
            icon_size: 48,
            sort: String::new(),
            transparency: 0,
            title_pos: String::new(),
            view: String::new(),
            title_size: 0,
            locked: false,
        };
        let defaults = vec![
            mk("文档", "#3b82f6", 0.0, 0.0, &["pdf", "doc", "docx", "pages", "txt", "md", "rtf", "wps", "xls", "xlsx", "csv", "numbers", "et", "ppt", "pptx", "key", "dps"], false, false),
            mk("图片与视频", "#ec4899", 0.0, 1.0, &["png", "jpg", "jpeg", "gif", "heic", "webp", "bmp", "tiff", "svg", "psd", "mp4", "mov", "mkv", "avi", "m4v", "webm", "mp3", "wav", "m4a", "flac"], false, false),
            mk("压缩包与安装包", "#f59e0b", 0.0, 2.0, &["zip", "rar", "7z", "tar", "gz", "tgz", "dmg", "pkg", "iso"], false, false),
            mk("文件夹", "#10b981", 1.0, 0.0, &[], true, false),
            mk("其他", "#64748b", 1.0, 1.0, &[], false, true),
        ];
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        s.collections = defaults;
        s.initialized = true;
        save(&s);
    }
    {
        let rects = screen_rects();
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        let mut moved = false;
        for c in s.collections.iter_mut() {
            let (nx, ny) = clamp_to_screens(c.x, c.y, c.w, c.h, &rects);
            if (nx - c.x).abs() > 0.5 || (ny - c.y).abs() > 0.5 {
                c.x = nx;
                c.y = ny;
                moved = true;
            }
        }
        if moved {
            save(&s);
        }
    }
    sync_windows(app);
    let a = app.clone();
    let _ = app.run_on_main_thread(move || init_auto_move(&a));

    // 单个后台线程负责：节流写盘、松手后吸附、桌面变化检测。
    // （拖动中的实时吸附由窗口移动事件直接触发，不在这里轮询。）
    let a = app.clone();
    std::thread::spawn(move || {
        let mut saved_last = std::time::Instant::now();
        let mut checked_last = std::time::Instant::now();
        let mut last_sig = 0u64;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let st = a.state::<CollectionsState>();
            // 松手后的完整吸附（拖动中 mousePressed 时不处理）
            if !st.moved.lock().unwrap().is_empty() {
                snap_moved(&a);
            }
            // 小组件的位置 / 尺寸变化（与集合共用这个线程写盘）
            crate::widgets::flush(&a);
            // 位置/尺寸变化：最多每秒写盘一次
            if st.dirty.load(Ordering::Relaxed) && saved_last.elapsed() >= Duration::from_millis(1000) {
                st.dirty.store(false, Ordering::Relaxed);
                save(&st.store.lock().unwrap());
                saved_last = std::time::Instant::now();
            }
            // 兜底检查：正常情况下由 FSEvents 即时触发，这里每 30 秒对一次目录签名
            if checked_last.elapsed() >= Duration::from_secs(30) {
                checked_last = std::time::Instant::now();
                if sweep(&a) {
                    let _ = a.emit("collections-changed", ());
                }
                let sig = desktop_signature(&a);
                if sig != last_sig {
                    if last_sig != 0 {
                        let _ = a.emit("desktop-changed", ());
                    }
                    last_sig = sig;
                }
            }
        }
    });
}

// ---------------------------------------------------------------- 吸附

type Rect = (f64, f64, f64, f64);

fn overlap_1d(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    a0 < b1 && b0 < a1
}

/// 计算吸附后的位置：边缘靠近其他集合或屏幕边缘时贴齐；与其他集合重叠时推到旁边。
/// `push`：与其他集合重叠时是否把它推开（拖动过程中只做边缘吸附，不推开）。
fn snap_position(r: Rect, others: &[Rect], screens: &[Rect], gap: f64, push: bool) -> (f64, f64) {
    let (mut x, mut y, w, h) = r;
    let best = |cur: f64, cands: &[f64]| -> f64 {
        cands.iter().copied().filter(|t| (t - cur).abs() <= SNAP_DIST).min_by(|a, b| (a - cur).abs().total_cmp(&(b - cur).abs())).unwrap_or(cur)
    };

    // 1) 边缘吸附（x、y 分别取最近的目标）
    let mut xs = vec![];
    let mut ys = vec![];
    for &(ox, oy, ow, oh) in others {
        // 竖直方向有交集（或接近）→ 可以左右并排
        if overlap_1d(y, y + h, oy - SNAP_DIST, oy + oh + SNAP_DIST) {
            xs.extend([ox + ow + gap, ox - gap - w, ox, ox + ow - w]);
        }
        // 水平方向有交集（或接近）→ 可以上下叠放
        if overlap_1d(x, x + w, ox - SNAP_DIST, ox + ow + SNAP_DIST) {
            ys.extend([oy + oh + gap, oy - gap - h, oy, oy + oh - h]);
        }
    }
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    if let Some(&(sx, sy, sw, sh)) = screens.iter().find(|(sx, sy, sw, sh)| cx >= *sx && cx <= sx + sw && cy >= *sy && cy <= sy + sh) {
        xs.extend([sx + SCREEN_MARGIN, sx + sw - SCREEN_MARGIN - w]);
        ys.extend([sy + SCREEN_MARGIN, sy + sh - SCREEN_MARGIN - h]);
    }
    x = best(x, &xs);
    y = best(y, &ys);

    // 2) 防重叠：反复把窗口推到重叠集合最近的一侧
    for _ in 0..if push { 6 } else { 0 } {
        let hit = others.iter().find(|&&(ox, oy, ow, oh)| overlap_1d(x, x + w, ox, ox + ow) && overlap_1d(y, y + h, oy, oy + oh));
        let Some(&(ox, oy, ow, oh)) = hit else { break };
        let moves = [(ox - gap - w - x, 0.0), (ox + ow + gap - x, 0.0), (0.0, oy - gap - h - y), (0.0, oy + oh + gap - y)];
        let (dx, dy) = moves.into_iter().min_by(|a, b| (a.0.abs() + a.1.abs()).total_cmp(&(b.0.abs() + b.1.abs()))).unwrap();
        x += dx;
        y += dy;
    }
    clamp_to_screens(x, y, w, h, screens)
}

/// 拖动中的实时边缘吸附：只在靠近其他集合 / 屏幕边缘时贴齐，不推开重叠。
fn snap_live(app: &AppHandle, id: &str) {
    let st = app.state::<CollectionsState>();
    let now = std::time::Instant::now();
    {
        // 冷却：避免把窗口挪动后又被自己的 Moved 事件再次触发
        let mut cool = st.snapping.lock().unwrap();
        match cool.get(id) {
            Some(t) if now.duration_since(*t) < Duration::from_millis(90) => return,
            _ => {
                cool.insert(id.to_string(), now);
            }
        }
    }
    let (cols, no_snap, hidden_all, gap) = {
        let s = st.store.lock().unwrap();
        (s.collections.clone(), s.no_snap, s.hidden_all, s.snap_gap.clamp(0.0, 60.0))
    };
    if no_snap || hidden_all {
        return;
    }
    let Some(c) = cols.iter().find(|c| c.id == id) else { return };
    if c.locked {
        return;
    }
    let others: Vec<Rect> = cols
        .iter()
        .filter(|o| o.id != id && o.visible)
        .map(|o| { let (w, h) = shown_size(o); (o.x, o.y, w, h) })
        .collect();
    let (w, h) = shown_size(c);
    let rects = screen_rects();
    let (nx, ny) = snap_position((c.x, c.y, w, h), &others, &rects, gap, false);
    if (nx - c.x).abs() > 0.5 || (ny - c.y).abs() > 0.5 {
        if let Some(win) = app.get_webview_window(&label_of(id)) {
            let _ = win.set_position(tauri::LogicalPosition::new(nx, ny));
        }
    }
}

/// 松开鼠标后的完整吸附：贴齐其他集合 / 屏幕边缘，并推开仍重叠的集合。
/// 拖动过程中的实时吸附由 `snap_live` 在窗口移动事件里完成。
fn snap_moved(app: &AppHandle) {
    let st = app.state::<CollectionsState>();
    // 鼠标仍按着（正在拖动）时不动它，免得和用户抢窗口
    if NSEvent::pressedMouseButtons() & 1 != 0 {
        return;
    }
    let now = std::time::Instant::now();
    let ids: Vec<String> = {
        let mut m = st.moved.lock().unwrap();
        let ready: Vec<String> = m.iter().filter(|(_, t)| now.duration_since(**t) > Duration::from_millis(120)).map(|(k, _)| k.clone()).collect();
        for k in &ready {
            m.remove(k);
        }
        ready
    };
    if ids.is_empty() {
        return;
    }
    st.snapping.lock().unwrap().clear();
    let (cols, no_snap, hidden_all, gap) = {
        let s = st.store.lock().unwrap();
        (s.collections.clone(), s.no_snap, s.hidden_all, s.snap_gap.clamp(0.0, 60.0))
    };
    if no_snap || hidden_all {
        return;
    }
    let screens = {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = app.run_on_main_thread(move || { let _ = tx.send(screen_rects()); });
        rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default()
    };
    for id in ids {
        let Some(c) = cols.iter().find(|c| c.id == id) else { continue };
        let others: Vec<Rect> = cols
            .iter()
            .filter(|o| o.id != id && o.visible)
            .map(|o| { let (w, h) = shown_size(o); (o.x, o.y, w, h) })
            .collect();
        let (w, h) = shown_size(c);
        let (nx, ny) = snap_position((c.x, c.y, w, h), &others, &screens, gap, true);
        if (nx - c.x).abs() > 0.5 || (ny - c.y).abs() > 0.5 {
            if let Some(win) = app.get_webview_window(&label_of(&id)) {
                let _ = win.set_position(tauri::LogicalPosition::new(nx, ny));
            }
        }
    }
}

/// 开/关自动归类。打开时立即整理一次。
fn set_auto_move_inner(app: &AppHandle, on: bool) {
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        s.auto_move = Some(on);
        save(&s);
    }
    if on {
        sweep(app);
    }
    commit(app);
}

/// 首次运行：自动归类默认关闭，等用户在管理页里预演确认后再开启，避免文件被悄悄移走。
fn init_auto_move(app: &AppHandle) {
    let st = app.state::<CollectionsState>();
    let mut s = st.store.lock().unwrap();
    if s.auto_move.is_none() {
        s.auto_move = Some(false);
        save(&s);
    }
}

/// FSEvents 通知入口（去抖 250ms）：桌面或集合目录有变化时刷新界面并执行自动归类。
pub fn fs_changed(app: &AppHandle) {
    use std::sync::atomic::AtomicU64;
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let last = LAST.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 250 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    if cfg!(debug_assertions) {
        eprintln!("[watcher] 目录变化");
    }
    if sweep(app) {
        let _ = app.emit("collections-changed", ());
    }
    let _ = app.emit("desktop-changed", ());
}

/// 桌面与各集合目录的「签名」：只用目录 mtime，避免每次轮询都遍历成千上万的文件。
fn desktop_signature(app: &AppHandle) -> u64 {
    let mut h = DefaultHasher::new();
    let mut dirs = vec![desktop_dir(), store_root()];
    {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        dirs.extend(s.collections.iter().filter_map(|c| c.dir.as_ref().map(PathBuf::from)));
    }
    for d in dirs {
        if let Ok(m) = fs::metadata(&d) {
            d.hash(&mut h);
            m.mtime().hash(&mut h);
        }
    }
    h.finish() | 1
}

// ---------------------------------------------------------------- 原生对话框

fn activate(mtm: MainThreadMarker) {
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

fn alert_confirm(title: &str, message: &str, ok: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    let a = NSAlert::new(mtm);
    a.setMessageText(&NSString::from_str(title));
    a.setInformativeText(&NSString::from_str(message));
    a.addButtonWithTitle(&NSString::from_str(ok));
    a.addButtonWithTitle(&NSString::from_str("取消"));
    activate(mtm);
    a.runModal() == NSAlertFirstButtonReturn
}

fn pick_folder_main() -> Option<String> {
    let mtm = MainThreadMarker::new()?;
    let p = NSOpenPanel::openPanel(mtm);
    p.setCanChooseDirectories(true);
    p.setCanChooseFiles(false);
    p.setAllowsMultipleSelection(false);
    p.setPrompt(Some(&NSString::from_str("选择")));
    p.setMessage(Some(&NSString::from_str("选择要在桌面上显示的文件夹")));
    activate(mtm);
    if p.runModal() != 1 {
        return None;
    }
    p.URLs().firstObject()?.path().map(|s| s.to_string())
}

/// 在主线程上执行并等待结果（命令线程中调用）。
pub(crate) async fn on_main<T: Send + 'static>(app: &AppHandle, f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn confirm_dialog(app: AppHandle, title: String, message: String, ok: Option<String>) -> Result<bool, String> {
    let ok = ok.unwrap_or_else(|| "确定".into());
    on_main(&app, move || alert_confirm(&title, &message, &ok)).await
}

#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<Option<String>, String> {
    on_main(&app, pick_folder_main).await
}

// ---------------------------------------------------------------- Finder 桌面图标

fn finder_icons_shown() -> bool {
    util::run("defaults", &["read", "com.apple.finder", "CreateDesktop"])
        .map(|s| !matches!(s.trim(), "0" | "false" | "NO"))
        .unwrap_or(true)
}

fn set_finder_icons_inner(show: bool) -> Result<(), String> {
    util::run("defaults", &["write", "com.apple.finder", "CreateDesktop", "-bool", if show { "true" } else { "false" }])?;
    let _ = util::run("killall", &["Finder"]);
    Ok(())
}

// ---------------------------------------------------------------- 命令

#[tauri::command]
pub fn list_collections(app: AppHandle) -> Vec<Collection> {
    snapshot(&app).0
}

#[tauri::command]
pub fn get_collection(app: AppHandle, id: String) -> Option<Collection> {
    find(&app, &id)
}

#[tauri::command]
pub async fn collection_items(app: AppHandle, id: String) -> Result<Vec<Item>, String> {
    let c = find(&app, &id).ok_or("集合不存在")?;
    let mut items = match c.kind.as_str() {
        "collection" | "dropbox" => {
            let mut v = c.dir.as_deref().map(|d| list_dir(Path::new(d))).unwrap_or_default();
            v.extend(c.refs.iter().filter_map(|p| make_item(Path::new(p), true)));
            v
        }
        "folder" => c.folder.as_deref().map(|f| list_dir(Path::new(f))).unwrap_or_default(),
        _ => vec![],
    };
    sort_items(&mut items, &c.sort);
    Ok(items)
}

/// 仍留在桌面上、没有放进任何集合的文件（隐藏系统桌面图标时这些文件会“看不见”，需提示用户）。
#[tauri::command]
pub async fn unassigned_items() -> Vec<Item> {
    list_dir(&desktop_dir())
}

#[derive(Deserialize)]
pub struct Patch {
    id: String,
    name: Option<String>,
    color: Option<String>,
    exts: Option<Vec<String>>,
    keywords: Option<Vec<String>>,
    dirs: Option<bool>,
    catch_all: Option<bool>,
    visible: Option<bool>,
    icon_size: Option<u32>,
    sort: Option<String>,
    folder: Option<String>,
    transparency: Option<u32>,
    title_pos: Option<String>,
    view: Option<String>,
    title_size: Option<u32>,
    locked: Option<bool>,
}

#[tauri::command]
pub async fn update_collection(app: AppHandle, patch: Patch) -> Result<(), String> {
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if patch.catch_all == Some(true) {
            for c in s.collections.iter_mut() {
                c.catch_all = false; // 兜底集合只能有一个
            }
        }
        let c = s.collections.iter_mut().find(|c| c.id == patch.id).ok_or("集合不存在")?;
        if let Some(v) = patch.name {
            c.name = v.trim().chars().take(40).collect();
            if let Some(old) = c.dir.clone().map(PathBuf::from) {
                let new = store_root().join(sanitize(&c.name));
                if old.parent() == Some(store_root().as_path()) && old != new && !new.exists() && fs::rename(&old, &new).is_ok() {
                    c.dir = Some(new.to_string_lossy().into_owned());
                }
            }
        }
        if let Some(v) = patch.color { c.color = v; }
        if let Some(v) = patch.exts { c.exts = v.into_iter().map(|e| e.trim().trim_start_matches('.').to_lowercase()).filter(|e| !e.is_empty()).collect(); }
        if let Some(v) = patch.keywords { c.keywords = v.into_iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect(); }
        if let Some(v) = patch.dirs { c.dirs = v; }
        if let Some(v) = patch.catch_all { c.catch_all = v; }
        if let Some(v) = patch.visible { c.visible = v; }
        if let Some(v) = patch.icon_size { c.icon_size = v.clamp(32, 96); }
        if let Some(v) = patch.sort { c.sort = v; }
        if let Some(v) = patch.folder.clone() { c.folder = Some(v); }
        if let Some(v) = patch.transparency { c.transparency = v.min(100); }
        if let Some(v) = patch.title_pos.clone() { c.title_pos = v; }
        if let Some(v) = patch.view.clone() { c.view = v; }
        if let Some(v) = patch.title_size { c.title_size = if v == 0 { 0 } else { v.clamp(10, 32) }; }
        if let Some(v) = patch.locked { c.locked = v; }
    }
    if patch.locked.is_some() {
        apply_lock(&app, &patch.id);
    }
    if patch.folder.is_some() {
        refresh_watcher(&app);
    }
    if patch.title_pos.is_some() || patch.view.is_some() {
        apply_shape(&app, &patch.id);
    }
    if let Some(v) = patch.transparency {
        if let Some(w) = app.get_webview_window(&label_of(&patch.id)) {
            apply_transparency(&w, v.min(100));
        }
    }
    commit(&app);
    Ok(())
}

/// 拖动滑块时的实时调整：只改内存 + 原生毛玻璃 + 通知窗口，节流写盘，不重建菜单。
#[tauri::command]
pub async fn set_transparency(app: AppHandle, id: String, value: u32) {
    let v = value.min(100);
    {
        let st = app.state::<CollectionsState>();
        if let Some(c) = st.store.lock().unwrap().collections.iter_mut().find(|c| c.id == id) {
            c.transparency = v;
        }
        st.dirty.store(true, Ordering::Relaxed);
    }
    if let Some(w) = app.get_webview_window(&label_of(&id)) {
        apply_transparency(&w, v);
    }
    let _ = app.emit("collection-transparency", (id, v));
}

/// `place`：(鼠标位置, 屏幕区域)，需在主线程上获取。
pub fn create_inner(app: &AppHandle, kind: &str, folder: Option<String>, place: (Option<(f64, f64)>, Vec<(f64, f64, f64, f64)>)) -> Result<Collection, String> {
    if kind == "folder" && folder.is_none() {
        return Err("请先选择文件夹".into());
    }
    let (cursor, rects) = place;
    let (cx, cy) = cursor.unwrap_or((300.0, 200.0));
    let (w, h, name, color) = match kind {
        "folder" => (340.0, 250.0, folder.as_deref().and_then(|f| Path::new(f).file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or("文件夹".into()), "#06b6d4"),
        "dropbox" => (240.0, 190.0, "拖拽释放框".to_string(), "#f59e0b"),
        "disk" => (300.0, 190.0, "硬盘".to_string(), "#64748b"),
        _ => (320.0, 230.0, "新集合".to_string(), "#8b5cf6"),
    };
    let (x, y) = clamp_to_screens(cx - 40.0, cy - 16.0, w, h, &rects);
    let c = Collection {
        id: new_id(),
        name,
        kind: kind.into(),
        color: color.into(),
        x,
        y,
        w,
        h,
        visible: true,
        collapsed: false,
        exts: vec![],
        keywords: vec![],
        dirs: false,
        catch_all: false,
        folder,
        refs: vec![],
        dir: None,
        icon_size: 48,
        sort: String::new(),
        transparency: 0,
        title_pos: String::new(),
        view: String::new(),
        title_size: 0,
        locked: false,
    };
    let on_top = {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        s.collections.push(c.clone());
        s.hidden_all = false;
        s.on_top
    };
    open_window(app, &c, on_top, kind == "collection" || kind == "dropbox");
    commit(app);
    if kind == "folder" {
        refresh_watcher(app);
    }
    Ok(c)
}

#[tauri::command]
pub async fn create_collection(app: AppHandle, kind: String, folder: Option<String>) -> Result<Collection, String> {
    let place = on_main(&app, || (cursor_logical(), screen_rects())).await?;
    create_inner(&app, &kind, folder, place)
}

fn delete_inner(app: &AppHandle, id: &str) -> Result<(), String> {
    let r = {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        let dir = s.collections.iter().find(|c| c.id == id).and_then(|c| c.dir.clone());
        let r = match dir {
            Some(d) => return_all_reason(&mut s, Path::new(&d), "删除集合后移回桌面").map(|_| { let _ = fs::remove_dir(&d); }),
            None => Ok(()),
        };
        // 有文件没能移回桌面时保留集合，防止文件“失联”
        if r.is_ok() {
            s.collections.retain(|c| c.id != id);
            s.assign.retain(|_, v| v != id);
        }
        save(&s);
        r
    };
    commit(app);
    refresh_watcher(app);
    r.map_err(|e| format!("部分文件未能移回桌面，集合已保留：{e}"))
}

#[tauri::command]
pub async fn delete_collection(app: AppHandle, id: String) -> Result<(), String> {
    delete_inner(&app, &id)
}

/// 文件被拖进集合窗口。
#[tauri::command]
pub async fn add_to_collection(app: AppHandle, id: String, paths: Vec<String>) -> Result<usize, String> {
    let c = find(&app, &id).ok_or("集合不存在")?;
    let desktop = desktop_dir();
    let mut n = 0;
    match c.kind.as_str() {
        "collection" | "dropbox" => {
            let st = app.state::<CollectionsState>();
            let mut s = st.store.lock().unwrap();
            let store_dirs: Vec<PathBuf> = s.collections.iter().filter_map(|o| o.dir.as_ref().map(PathBuf::from)).collect();
            let mut errs = vec![];
            for p in paths {
                let src = PathBuf::from(&p);
                let parent = src.parent().map(Path::to_path_buf);
                // 桌面上的文件、或从另一个集合拖过来的文件：真正移动，保证只存在一份
                let movable = parent.as_deref() == Some(desktop.as_path()) || parent.as_ref().is_some_and(|pp| store_dirs.contains(pp));
                if movable {
                    let dir = match ensure_dir(&mut s, &id) {
                        Ok(d) => d,
                        Err(e) => { errs.push(e); break; }
                    };
                    if parent.as_deref() == Some(dir.as_path()) || dir.starts_with(&src) {
                        continue;
                    }
                    match move_into(&src, &dir) {
                        Ok(dest) => {
                            s.assign.remove(&p);
                            log_move(&mut s, &src, &dest, &format!("放入「{}」", c.name));
                            n += 1;
                        }
                        Err(e) => errs.push(e),
                    }
                    continue;
                } else {
                    if c.kind == "collection" {
                        for o in s.collections.iter_mut().filter(|o| o.kind == "collection") {
                            o.refs.retain(|r| r != &p);
                        }
                    }
                    if let Some(t) = s.collections.iter_mut().find(|t| t.id == id) {
                        if !t.refs.contains(&p) {
                            t.refs.push(p.clone());
                        }
                    }
                }
                n += 1;
            }
            save(&s);
            drop(s);
            if !errs.is_empty() {
                commit(&app);
                return Err(format!("部分文件未能放入：{}", errs.join("；")));
            }
        }
        "folder" => {
            let dir = PathBuf::from(c.folder.clone().ok_or("文件夹未设置")?);
            let dir_dev = fs::metadata(&dir).map_err(|e| e.to_string())?.dev();
            for p in paths {
                let src = PathBuf::from(&p);
                if src.parent() == Some(dir.as_path()) || dir.starts_with(&src) {
                    continue;
                }
                let name = src.file_name().ok_or("路径无效")?.to_string_lossy().into_owned();
                let dest = crate::desktop::unique_target(&dir, &name);
                let same_volume = fs::metadata(&src).map(|m| m.dev() == dir_dev).unwrap_or(false);
                let r = if same_volume {
                    fs::rename(&src, &dest).map_err(|e| e.to_string())
                } else {
                    util::run("cp", &["-R", &p, &dest.to_string_lossy()]).map(|_| ())
                };
                r.map_err(|e| format!("{name}: {e}"))?;
                n += 1;
            }
        }
        _ => return Err("硬盘面板不支持放入文件".into()),
    }
    commit(&app);
    Ok(n)
}

/// 移出集合：快捷引用直接去掉；集合里的文件移回桌面（并标记为留在桌面）。
fn remove_inner(app: &AppHandle, id: &str, path: &str) -> Result<(), String> {
    let r = {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        let Some(c) = s.collections.iter_mut().find(|c| c.id == id) else { return Ok(()) };
        if c.refs.iter().any(|r| r == path) {
            c.refs.retain(|r| r != path);
            Ok(())
        } else if c.dir.as_deref().is_some_and(|d| Path::new(path).parent() == Some(Path::new(d))) {
            let name = c.name.clone();
            move_into(Path::new(path), &desktop_dir()).map(|d| {
                s.assign.insert(d.to_string_lossy().into_owned(), "-".into());
                log_move(&mut s, Path::new(path), &d, &format!("从「{name}」移回桌面"));
            })
        } else {
            Ok(())
        }
    };
    commit(app);
    r
}

#[tauri::command]
pub async fn remove_from_collection(app: AppHandle, id: String, path: String) -> Result<(), String> {
    remove_inner(&app, &id, &path)
}

/// 集合中的文件被拖出（移动模式）后调用：若它被拖回了桌面，标记为「留在桌面」，不再自动收回。
#[tauri::command]
pub async fn drag_finished(app: AppHandle, paths: Vec<String>) {
    let desk = desktop_dir();
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        for p in paths {
            let src = Path::new(&p);
            if src.exists() {
                continue;
            }
            if let Some(n) = src.file_name() {
                let d = desk.join(n);
                if d.exists() {
                    s.assign.insert(d.to_string_lossy().into_owned(), "-".into());
                    log_move(&mut s, src, &d, "拖出到桌面");
                }
            }
        }
        save(&s);
    }
    commit(&app);
}

/// 按数据调整窗口实际尺寸（折叠 / 标题位置 / 迷你样式变化后调用）。
fn apply_shape(app: &AppHandle, id: &str) {
    if let (Some(c), Some(win)) = (find(app, id), app.get_webview_window(&label_of(id))) {
        let (w, h) = shown_size(&c);
        let _ = win.set_size(LogicalSize::new(w, h));
    }
}

/// 最近的移动操作（新的在前）。
#[tauri::command]
pub async fn recent_moves(app: AppHandle) -> Vec<MoveOp> {
    let st = app.state::<CollectionsState>();
    let s = st.store.lock().unwrap();
    s.history.iter().rev().take(30).cloned().collect()
}

/// 撤销一次移动：把文件从 `to` 移回 `from` 所在目录。
#[tauri::command]
pub async fn undo_move(app: AppHandle, id: u64) -> Result<(), String> {
    let op = {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        s.history.iter().find(|o| o.id == id).cloned()
    }
    .ok_or("这条记录已经不存在了")?;
    let src = PathBuf::from(&op.to);
    if !src.exists() {
        return Err(format!("「{}」已经不在原位置，无法撤销", op.name));
    }
    let from = PathBuf::from(&op.from);
    let parent = from.parent().ok_or("原路径无效")?.to_path_buf();
    fs::create_dir_all(&parent).map_err(|e| format!("无法创建 {}：{e}", parent.display()))?;
    let restored = move_into(&src, &parent)?;
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if parent == desktop_dir() {
            s.assign.insert(restored.to_string_lossy().into_owned(), "-".into());
        } else {
            s.assign.remove(&restored.to_string_lossy().into_owned());
        }
        s.history.retain(|o| o.id != id);
        save(&s);
    }
    commit(&app);
    Ok(())
}

#[tauri::command]
pub async fn clear_move_history(app: AppHandle) {
    {
        let st = app.state::<CollectionsState>();
        st.store.lock().unwrap().history.clear();
        save(&st.store.lock().unwrap());
    }
    let _ = app.emit("collections-changed", ());
}

fn set_collapsed_inner(app: &AppHandle, id: &str, collapsed: bool) {
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if let Some(c) = s.collections.iter_mut().find(|c| c.id == id) {
            c.collapsed = collapsed;
        }
    }
    apply_shape(app, id);
    commit(app);
}

#[tauri::command]
pub async fn set_collapsed(app: AppHandle, id: String, collapsed: bool) {
    set_collapsed_inner(&app, &id, collapsed);
}

#[derive(Serialize)]
pub struct Prefs {
    on_top: bool,
    hidden_all: bool,
    finder_icons: bool,
    snap: bool,
    snap_gap: f64,
    tray_stats: bool,
    autostart: bool,
    auto_move: bool,
}

#[tauri::command]
pub async fn get_prefs(app: AppHandle) -> Prefs {
    let (_, on_top, hidden_all) = snapshot(&app);
    let (snap, snap_gap, tray_stats, auto_move) = {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        (!s.no_snap, s.snap_gap, s.tray_stats, s.auto_move == Some(true))
    };
    Prefs {
        on_top,
        hidden_all,
        finder_icons: finder_icons_shown(),
        snap,
        snap_gap,
        tray_stats,
        autostart: crate::autostart::is_enabled(),
        auto_move,
    }
}

fn set_flags(app: &AppHandle, on_top: Option<bool>, hidden_all: Option<bool>) {
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if let Some(v) = on_top { s.on_top = v; }
        if let Some(v) = hidden_all { s.hidden_all = v; }
    }
    commit(app);
    apply_level_all(app);
}

#[tauri::command]
pub async fn set_prefs(
    app: AppHandle,
    on_top: Option<bool>,
    hidden_all: Option<bool>,
    snap: Option<bool>,
    snap_gap: Option<f64>,
    tray_stats: Option<bool>,
    autostart: Option<bool>,
) -> Result<(), String> {
    {
        let st = app.state::<CollectionsState>();
        let mut s = st.store.lock().unwrap();
        if let Some(v) = snap {
            s.no_snap = !v;
        }
        if let Some(v) = snap_gap {
            s.snap_gap = v.clamp(0.0, 60.0);
        }
        if let Some(v) = tray_stats {
            s.tray_stats = v;
        }
    }
    if let Some(v) = autostart {
        crate::autostart::set_enabled(v)?;
    }
    set_flags(&app, on_top, hidden_all);
    Ok(())
}

#[tauri::command]
pub async fn set_finder_icons(app: AppHandle, show: bool) -> Result<(), String> {
    set_finder_icons_inner(show)?;
    rebuild_tray(&app);
    let _ = app.emit("collections-changed", ());
    Ok(())
}

#[derive(Serialize)]
pub struct Volume {
    name: String,
    mount: String,
    total: u64,
    available: u64,
}

#[tauri::command]
pub async fn list_volumes() -> Vec<Volume> {
    let disks = sysinfo::Disks::new_with_refreshed_list().ok();
    let mut v: Vec<Volume> = disks
        .as_ref()
        .map(|d| &d[..])
        .unwrap_or_default()
        .iter()
        .filter(|d| {
            let m = d.mount_point().to_string_lossy();
            m == "/" || m.starts_with("/Volumes/")
        })
        .map(|d| {
            let mount = d.mount_point().to_string_lossy().into_owned();
            let name = if mount == "/" { "Macintosh HD".to_string() } else { Path::new(&mount).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(mount.clone()) };
            Volume { name, mount, total: d.total_space(), available: d.available_space() }
        })
        .collect();
    v.dedup_by(|a, b| a.mount == b.mount);
    v
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[tauri::command]
pub fn open_main(app: AppHandle, focus: Option<String>) {
    show_main(&app);
    if let Some(id) = focus {
        let _ = app.emit("focus-collection", id);
    }
}

// ---------------------------------------------------------------- 菜单（托盘 + 集合右键）

fn item(app: &AppHandle, id: &str, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, None::<&str>)
}
fn check(app: &AppHandle, id: &str, text: &str, on: bool) -> tauri::Result<CheckMenuItem<Wry>> {
    CheckMenuItem::with_id(app, id, text, true, on, None::<&str>)
}

fn new_submenu(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
    let m = Submenu::new(app, "新建", true)?;
    m.append(&item(app, "new:collection", "集合 — 桌面文档集")?)?;
    m.append(&item(app, "new:disk", "硬盘面板 — 电脑上的硬盘")?)?;
    m.append(&item(app, "new:folder", "文件夹合集 — 选择文件夹…")?)?;
    m.append(&item(app, "new:dropbox", "拖拽释放框 — 临时文件容器")?)?;
    Ok(m)
}

fn mine_submenu(app: &AppHandle, cols: &[Collection]) -> tauri::Result<Submenu<Wry>> {
    let m = Submenu::new(app, "我的集合", true)?;
    for c in cols {
        m.append(&check(app, &format!("toggle:{}", c.id), &c.name, c.visible)?)?;
    }
    if cols.is_empty() {
        m.append(&MenuItem::new(app, "（暂无集合）", false, None::<&str>)?)?;
    }
    Ok(m)
}

fn append_global(app: &AppHandle, m: &Menu<Wry>, on_top: bool, hidden_all: bool) -> tauri::Result<()> {
    m.append(&check(app, "front", "将集合置于前方", on_top)?)?;
    m.append(&check(app, "hide_all", "隐藏集合", hidden_all)?)?;
    let (snap, gap) = {
        let st = app.state::<CollectionsState>();
        let s = st.store.lock().unwrap();
        (!s.no_snap, s.snap_gap)
    };
    m.append(&check(app, "snap", "集合互相吸附", snap)?)?;
    let stats = app.state::<CollectionsState>().store.lock().unwrap().tray_stats;
    m.append(&check(app, "tray_stats", "在菜单栏显示 CPU / 内存", stats)?)?;
    let gs = Submenu::new(app, &format!("吸附间隙（{} px）", gap.round()), true)?;
    for n in [0u32, 2, 4, 6, 8, 12, 16, 24] {
        gs.append(&check(app, &format!("gap:{n}"), switch_label(n), (gap - n as f64).abs() < 0.5)?)?;
    }
    m.append(&gs)?;
    let auto = app.state::<CollectionsState>().store.lock().unwrap().auto_move == Some(true);
    m.append(&check(app, "auto_move", "自动把桌面文件归入集合", auto)?)?;
    m.append(&check(app, "finder_icons", "隐藏系统桌面图标", !finder_icons_shown())?)?;
    m.append(&PredefinedMenuItem::separator(app)?)?;
    m.append(&item(app, "open_main", "打开 Mac 管家…")?)?;
    Ok(())
}

pub fn build_tray_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let (cols, on_top, hidden_all) = snapshot(app);
    let m = Menu::new(app)?;
    m.append(&new_submenu(app)?)?;
    m.append(&crate::widgets::new_submenu(app)?)?;
    m.append(&mine_submenu(app, &cols)?)?;
    m.append(&PredefinedMenuItem::separator(app)?)?;
    append_global(app, &m, on_top, hidden_all)?;
    m.append(&PredefinedMenuItem::separator(app)?)?;
    m.append(&item(app, "open_store", "打开集合文件夹（文件存放位置）")?)?;
    m.append(&item(app, "quit", "退出 Mac 管家")?)?;
    Ok(m)
}

/// 菜单相关的状态签名：文件移动等操作不改变菜单，可跳过重建。
fn menu_signature(app: &AppHandle) -> u64 {
    let st = app.state::<CollectionsState>();
    let s = st.store.lock().unwrap();
    let mut h = DefaultHasher::new();
    s.on_top.hash(&mut h);
    s.hidden_all.hash(&mut h);
    s.no_snap.hash(&mut h);
    s.auto_move.hash(&mut h);
    s.snap_gap.to_bits().hash(&mut h);
    for c in &s.collections {
        (c.id.as_str(), c.name.as_str(), c.visible).hash(&mut h);
    }
    h.finish()
}

pub fn rebuild_tray(app: &AppHandle) {
    let sig = menu_signature(app);
    {
        let st = app.state::<CollectionsState>();
        let mut last = st.menu_sig.lock().unwrap();
        if *last == Some(sig) {
            return;
        }
        *last = Some(sig);
    }
    if let (Some(t), Ok(menu)) = (app.tray_by_id("main"), build_tray_menu(app)) {
        let _ = t.set_menu(Some(menu));
    }
}

const SEP: char = '\u{1f}';

fn switch_label(n: u32) -> &'static str {
    match n {
        0 => "0 px（无缝）",
        2 => "2 px",
        4 => "4 px",
        6 => "6 px",
        8 => "8 px",
        12 => "12 px",
        16 => "16 px",
        _ => "24 px",
    }
}

fn build_context_menu(app: &AppHandle, c: &Collection, file: Option<&str>) -> tauri::Result<Menu<Wry>> {
    let m = Menu::new(app)?;
    if let Some(f) = file {
        m.append(&item(app, &format!("f:open:{f}"), "打开")?)?;
        m.append(&item(app, &format!("f:reveal:{f}"), "在 Finder 中显示")?)?;
        let is_ref = c.refs.iter().any(|r| r == f);
        if c.kind == "collection" || c.kind == "dropbox" {
            m.append(&item(app, &format!("f:remove:{}{SEP}{f}", c.id), if is_ref { "移除快捷引用" } else { "移回桌面" })?)?;
        }
        if !is_ref {
            m.append(&PredefinedMenuItem::separator(app)?)?;
            m.append(&item(app, &format!("f:trash:{f}"), "移到废纸篓")?)?;
        }
        return Ok(m);
    }

    // 新建（全局菜单里也有，这里保留是因为桌面空处右键属于 Finder）
    m.append(&new_submenu(app)?)?;
    // 外观：所有视觉相关设置收进一个二级菜单
    let look = Submenu::new(app, "外观", true)?;
    let colors = Submenu::new(app, "颜色", true)?;
    for (hex, name) in COLORS {
        colors.append(&check(app, &format!("c:color:{}:{hex}", c.id), name, c.color == *hex)?)?;
    }
    look.append(&colors)?;
    let tr = Submenu::new(app, &format!("透明度（{}%）", c.transparency), true)?;
    for n in [0u32, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
        let label = match n { 0 => "0%（不透明）".to_string(), 100 => "100%（完全透明）".to_string(), _ => format!("{n}%") };
        tr.append(&check(app, &format!("c:transp:{}:{n}", c.id), &label, c.transparency == n)?)?;
    }
    tr.append(&PredefinedMenuItem::separator(app)?)?;
    tr.append(&item(app, &format!("c:manage:{}", c.id), "更多外观设置…")?)?;
    look.append(&tr)?;
    let ts = Submenu::new(app, &format!("标题字号（{}）", if c.title_size == 0 { "默认".to_string() } else { format!("{} px", c.title_size) }), true)?;
    ts.append(&check(app, &format!("c:tsize:{}:0", c.id), "默认", c.title_size == 0)?)?;
    for n in [12u32, 13, 14, 15, 16, 18, 20, 24] {
        ts.append(&check(app, &format!("c:tsize:{}:{n}", c.id), &format!("{n} px"), c.title_size == n)?)?;
    }
    look.append(&ts)?;
    let tp = Submenu::new(app, "标题位置", true)?;
    for (k, label) in [("", "顶部"), ("bottom", "底部"), ("left", "左侧"), ("right", "右侧"), ("inside", "合集内（浮动）")] {
        tp.append(&check(app, &format!("c:title:{}:{k}", c.id), label, c.title_pos == k)?)?;
    }
    look.append(&tp)?;
    if c.kind != "disk" {
        let vw = Submenu::new(app, "显示方式", true)?;
        for (k, label) in [("", "图标"), ("list", "列表"), ("mini", "迷你")] {
            vw.append(&check(app, &format!("c:view:{}:{k}", c.id), label, c.view == k)?)?;
        }
        look.append(&vw)?;
        // 图标大小只在图标视图下有意义
        if c.view.is_empty() {
            let sizes = Submenu::new(app, "图标大小", true)?;
            for (n, label) in [(36, "小"), (48, "中"), (64, "大"), (80, "特大")] {
                sizes.append(&check(app, &format!("c:icon:{}:{n}", c.id), label, c.icon_size == n)?)?;
            }
            look.append(&sizes)?;
        }
        let sort = Submenu::new(app, "排序方式", true)?;
        for (k, label) in [("", "修改时间"), ("name", "名称"), ("kind", "类型")] {
            sort.append(&check(app, &format!("c:sort:{}:{k}", c.id), label, c.sort == k)?)?;
        }
        look.append(&sort)?;
    }
    m.append(&look)?;

    m.append(&PredefinedMenuItem::separator(app)?)?;
    m.append(&item(app, &format!("c:rename:{}", c.id), "重命名…")?)?;
    m.append(&check(app, &format!("c:lock:{}", c.id), "锁定位置（防止误拖动）", c.locked)?)?;
    if c.kind == "collection" {
        m.append(&item(app, &format!("c:rules:{}", c.id), "自动归类规则…")?)?;
    }
    if let Some(d) = c.dir.as_deref() {
        m.append(&item(app, &format!("f:open:{d}"), "在 Finder 中打开存放目录")?)?;
        m.append(&item(app, &format!("c:returnall:{}", c.id), "全部移回桌面…")?)?;
    }
    if c.kind == "folder" {
        m.append(&item(app, &format!("c:folder:{}", c.id), "选择文件夹…")?)?;
        m.append(&item(app, &format!("f:open:{}", c.folder.clone().unwrap_or_default()), "在 Finder 中打开")?)?;
    }
    if c.kind == "dropbox" {
        m.append(&item(app, &format!("c:clear:{}", c.id), "清空释放框")?)?;
    }
    m.append(&PredefinedMenuItem::separator(app)?)?;
    m.append(&item(app, &format!("c:hide:{}", c.id), "隐藏此集合")?)?;
    m.append(&item(app, &format!("c:delete:{}", c.id), "删除此集合…")?)?;
    Ok(m)
}

#[tauri::command]
pub fn popup_collection_menu(app: AppHandle, window: WebviewWindow, id: String, file: Option<String>) -> Result<(), String> {
    let c = find(&app, &id).ok_or("集合不存在")?;
    let menu = build_context_menu(&app, &c, file.as_deref()).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

/// 全局菜单事件（托盘菜单与集合右键菜单共用）。在主线程上被调用。
pub fn handle_menu(app: &AppHandle, ev: MenuEvent) {
    let id = ev.id().as_ref().to_string();
    let mut parts = id.splitn(3, ':');
    let (a, b, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    match (a, b) {
        ("new", "folder") => {
            if let Some(f) = pick_folder_main() {
                let _ = create_inner(app, "folder", Some(f), (cursor_logical(), screen_rects()));
            }
        }
        ("new", kind) => { let _ = create_inner(app, kind, None, (cursor_logical(), screen_rects())); }
        ("toggle", cid) => with_col(app, cid, |c| c.visible = !c.visible),
        ("front", _) => { let v = !snapshot(app).1; set_flags(app, Some(v), None); }
        ("hide_all", _) => { let v = !snapshot(app).2; set_flags(app, None, Some(v)); }
        ("tray_stats", _) => {
            {
                let st = app.state::<CollectionsState>();
                let mut s = st.store.lock().unwrap();
                s.tray_stats = !s.tray_stats;
            }
            commit(app);
            if !app.state::<CollectionsState>().store.lock().unwrap().tray_stats {
                if let Some(t) = app.tray_by_id("main") {
                    let _ = t.set_title(None::<&str>);
                }
            }
        }
        ("snap", _) => {
            {
                let st = app.state::<CollectionsState>();
                let mut s = st.store.lock().unwrap();
                s.no_snap = !s.no_snap;
            }
            commit(app);
        }
        ("finder_icons", _) => {
            let show = !finder_icons_shown();
            if show || alert_confirm("隐藏系统桌面图标？", "隐藏后桌面只显示集合；未放入集合的文件仍在「桌面」文件夹中。Finder 会重启一次，随时可以在菜单中恢复。", "隐藏") {
                let _ = set_finder_icons_inner(show);
                rebuild_tray(app);
                let _ = app.emit("collections-changed", ());
            }
        }
        ("auto_move", _) => {
            let on = app.state::<CollectionsState>().store.lock().unwrap().auto_move != Some(true);
            set_auto_move_inner(app, on);
        }
        ("open_main", _) => show_main(app),
        ("open_store", _) => {
            let _ = fs::create_dir_all(store_root());
            let _ = util::run("open", &[&store_root().to_string_lossy()]);
        }
        ("quit", _) => app.exit(0),
        ("c", "rename") => { let _ = app.emit("start-rename", rest.to_string()); }
        ("c", "color") => {
            if let Some((cid, hex)) = rest.split_once(':') {
                let hex = hex.to_string();
                with_col(app, cid, |c| c.color = hex);
            }
        }
        ("c", "transp") => {
            if let Some((cid, n)) = rest.split_once(':') {
                let n: u32 = n.parse().unwrap_or(0);
                with_col(app, cid, |c| c.transparency = n.min(100));
                if let Some(w) = app.get_webview_window(&label_of(cid)) {
                    apply_transparency(&w, n);
                }
            }
        }
        ("c", "tsize") => {
            if let Some((cid, n)) = rest.split_once(':') {
                let n: u32 = n.parse().unwrap_or(0);
                with_col(app, cid, |c| c.title_size = if n == 0 { 0 } else { n.clamp(10, 32) });
            }
        }
        ("gap", n) => {
            let v: f64 = n.parse().unwrap_or(DEFAULT_SNAP_GAP).clamp(0.0, 60.0);
            {
                let st = app.state::<CollectionsState>();
                st.store.lock().unwrap().snap_gap = v;
            }
            commit(app);
        }
        ("c", "title") | ("c", "view") => {
            if let Some((cid, k)) = rest.split_once(':') {
                let k = k.to_string();
                let is_title = b == "title";
                with_col(app, cid, |c| if is_title { c.title_pos = k } else { c.view = k });
                apply_shape(app, cid);
            }
        }
        ("c", "icon") => {
            if let Some((cid, n)) = rest.split_once(':') {
                let n = n.parse().unwrap_or(48);
                with_col(app, cid, |c| c.icon_size = n);
            }
        }
        ("c", "sort") => {
            if let Some((cid, k)) = rest.split_once(':') {
                let k = k.to_string();
                with_col(app, cid, |c| c.sort = k);
            }
        }
        ("c", "lock") => {
            let locked = find(app, rest).map(|c| !c.locked).unwrap_or(false);
            with_col(app, rest, |c| c.locked = locked);
            apply_lock(app, rest);
        }
        ("c", "collapse") => {
            let collapsed = find(app, rest).map(|c| !c.collapsed).unwrap_or(false);
            set_collapsed_inner(app, rest, collapsed);
        }
        ("c", "manage") | ("c", "rules") => {
            show_main(app);
            let _ = app.emit("focus-collection", rest.to_string());
        }
        ("c", "folder") => {
            if let Some(f) = pick_folder_main() {
                with_col(app, rest, |c| c.folder = Some(f));
            }
        }
        ("c", "clear") => {
            if alert_confirm("清空释放框？", "释放框中的文件会移回桌面，快捷引用会被移除。", "清空") {
                {
                    let st = app.state::<CollectionsState>();
                    let mut s = st.store.lock().unwrap();
                    let dir = s.collections.iter().find(|c| c.id == rest).and_then(|c| c.dir.clone());
                    if let Some(d) = dir {
                        if let Err(e) = return_all(&mut s, Path::new(&d)) {
                            eprintln!("[collections] 清空释放框失败 {e}");
                        }
                    }
                    save(&s);
                }
                with_col(app, rest, |c| c.refs.clear());
            }
        }
        ("c", "returnall") => {
            let name = find(app, rest).map(|c| c.name).unwrap_or_default();
            if alert_confirm(
                &format!("把「{name}」里的文件全部移回桌面？"),
                "文件会回到桌面，集合本身保留（清空）。也可以只把个别文件拖出或右键「移回桌面」。",
                "全部移回",
            ) {
                match return_collection_files_inner(app, rest) {
                    Ok(n) => eprintln!("[collections] 已移回 {n} 项"),
                    Err(e) => { let _ = alert_confirm("移回桌面失败", &e, "好"); }
                }
            }
        }
        ("c", "hide") => with_col(app, rest, |c| c.visible = false),
        ("c", "delete") => {
            let name = find(app, rest).map(|c| c.name).unwrap_or_default();
            if alert_confirm(&format!("删除集合「{name}」？"), "只删除这个集合，集合中的文件不会被删除，会全部移回桌面。", "删除") {
                if let Err(e) = delete_inner(app, rest) {
                    let _ = alert_confirm("删除集合失败", &e, "好");
                }
            }
        }
        ("f", "open") => { let _ = util::run("open", &[rest]); }
        ("f", "reveal") => { let _ = util::run("open", &["-R", rest]); }
        ("f", "remove") => {
            if let Some((cid, path)) = rest.split_once(SEP) {
                if let Err(e) = remove_inner(app, cid, path) {
                    let _ = alert_confirm("移回桌面失败", &e, "好");
                }
            }
        }
        ("f", "trash") => {
            let name = Path::new(rest).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if alert_confirm(&format!("将「{name}」移到废纸篓？"), "可以在废纸篓中恢复。", "移到废纸篓") {
                match util::ensure_within(Path::new(rest), &[home()]) {
                    Ok(p) if !p.starts_with(home().join("Library")) => {
                        if let Err(e) = trash::delete(&p) {
                            eprintln!("移到废纸篓失败: {e}");
                        }
                        let _ = app.emit("desktop-changed", ());
                    }
                    _ => eprintln!("拒绝删除: {rest}"),
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(id: &str, name: &str, exts: &[&str], dirs: bool, catch_all: bool) -> Collection {
        Collection {
            id: id.into(), name: name.into(), kind: "collection".into(), color: String::new(),
            x: 0.0, y: 0.0, w: 300.0, h: 200.0, visible: true, collapsed: false,
            exts: exts.iter().map(|s| s.to_string()).collect(), keywords: vec![], dirs, catch_all,
            folder: None, refs: vec![], dir: None, icon_size: 48, sort: String::new(), transparency: 0,
            title_pos: String::new(), view: String::new(), title_size: 0, locked: false,
        }
    }

    /// 用临时 HOME 验证：放入集合后桌面不再保留；移回/删除集合后回到桌面；不会重复出现。
    #[test]
    fn move_semantics() {
        let tmp = std::env::temp_dir().join(format!("mb-test-{}", new_id()));
        fs::create_dir_all(tmp.join("Desktop/项目A")).unwrap();
        fs::write(tmp.join("Desktop/项目A/x.txt"), "x").unwrap();
        fs::write(tmp.join("Desktop/报告.pdf"), "p").unwrap();
        fs::write(tmp.join("Desktop/随手.xyz"), "z").unwrap();
        std::env::set_var("HOME", &tmp);
        let desk = desktop_dir();

        let mut s = Store {
            collections: vec![col("d", "文档", &["pdf"], false, false), col("f", "文件夹", &[], true, false), col("o", "其他", &[], false, true)],
            ..Default::default()
        };
        // 规则计划：三个文件各归其位
        let mut plan = plan_sweep(&s);
        plan.sort();
        assert_eq!(plan.len(), 3);

        // 手动放入：文件夹移入「文件夹」集合
        let dir = ensure_dir(&mut s, "f").unwrap();
        assert_eq!(dir, tmp.join("桌面集合/文件夹"));
        let moved = move_into(&desk.join("项目A"), &dir).unwrap();
        assert!(!desk.join("项目A").exists(), "桌面上不应再有该文件夹");
        assert!(moved.join("x.txt").exists(), "文件夹内容应完整保留");

        // 重名：桌面上又出现同名文件夹，移入时自动加序号，不覆盖
        fs::create_dir(desk.join("项目A")).unwrap();
        let moved2 = move_into(&desk.join("项目A"), &dir).unwrap();
        assert_ne!(moved, moved2);
        assert!(moved.join("x.txt").exists());

        // 移回桌面：标记为留在桌面，不会被规则再收走
        let back = move_into(&moved, &desk).unwrap();
        s.assign.insert(back.to_string_lossy().into_owned(), "-".into());
        assert!(back.join("x.txt").exists() && !moved.exists());
        assert!(plan_sweep(&s).iter().all(|(p, _)| Path::new(p) != back));

        // 删除集合：全部移回桌面，目录被清理
        return_all(&mut s, &dir).unwrap();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let names: Vec<String> = fs::read_dir(&desk).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().filter(|n| n.starts_with("项目A")).count() == 2, "{names:?}");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn snapping() {
        let screen = [(0.0, 25.0, 1600.0, 975.0)];
        let other = [(100.0, 100.0, 300.0, 200.0)];
        // 靠近右侧 → 贴在右边，留 8px 间隙，顶部对齐
        assert_eq!(snap_position((410.0, 110.0, 200.0, 150.0), &other, &screen, 8.0, true), (408.0, 100.0));
        // 靠近下方 → 贴在下方，左边对齐
        assert_eq!(snap_position((95.0, 315.0, 200.0, 150.0), &other, &screen, 8.0, true), (100.0, 308.0));
        // 大面积重叠 → 被推到最近的一侧，不再重叠
        let (x, y) = snap_position((250.0, 150.0, 200.0, 150.0), &other, &screen, 8.0, true);
        assert!(!(overlap_1d(x, x + 200.0, 100.0, 400.0) && overlap_1d(y, y + 150.0, 100.0, 300.0)), "{x},{y}");
        // 远离其他集合 → 不动
        assert_eq!(snap_position((800.0, 500.0, 200.0, 150.0), &other, &screen, 8.0, true), (800.0, 500.0));
        // 靠近屏幕边缘 → 贴边
        assert_eq!(snap_position((1385.0, 500.0, 200.0, 150.0), &other, &screen, 8.0, true), (1392.0, 500.0));
        // 间隙可调：0 = 紧贴，2 = 2px
        assert_eq!(snap_position((401.0, 100.0, 200.0, 150.0), &other, &screen, 0.0, true), (400.0, 100.0));
        assert_eq!(snap_position((403.0, 100.0, 200.0, 150.0), &other, &screen, 2.0, true), (402.0, 100.0));
        // 已吸附位置保持不变（不会来回抖动）
        assert_eq!(snap_position((408.0, 100.0, 200.0, 150.0), &other, &screen, 8.0, true), (408.0, 100.0));
    }
}
