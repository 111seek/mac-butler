//! 应用卸载器：列出 /Applications 与 ~/Applications 中的应用，
//! 查找与该应用的 Bundle ID 关联的残留文件，一并移到废纸篓。

use crate::util::{self, home};
use plist::Value;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

#[derive(Serialize)]
pub struct AppEntry {
    pub name: String,
    pub path: String,
    pub bundle_id: String,
    pub version: String,
    pub size: u64,
    pub running: bool,
}

#[derive(Serialize)]
pub struct Residue {
    pub path: String,
    pub size: u64,
    pub kind: String,
}

/// 尺寸缓存：应用体积扫描较慢，按「路径 + mtime」缓存。
fn size_cache() -> &'static Mutex<HashMap<String, (i64, u64)>> {
    static C: OnceLock<Mutex<HashMap<String, (i64, u64)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_size(path: &Path) -> u64 {
    let mtime = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let key = path.to_string_lossy().into_owned();
    {
        let c = size_cache().lock().unwrap();
        if let Some((m, s)) = c.get(&key) {
            if *m == mtime {
                return *s;
            }
        }
    }
    let size = util::disk_usage(path);
    size_cache().lock().unwrap().insert(key, (mtime, size));
    size
}

fn plist_str(v: &Value, key: &str) -> Option<String> {
    v.as_dictionary()?.get(key)?.as_string().map(str::to_string)
}

fn read_info(app: &Path) -> (String, String, String) {
    let stem = app.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let Ok(v) = Value::from_file(app.join("Contents/Info.plist")) else {
        return (String::new(), String::new(), stem);
    };
    let id = plist_str(&v, "CFBundleIdentifier").unwrap_or_default();
    let version = plist_str(&v, "CFBundleShortVersionString")
        .or_else(|| plist_str(&v, "CFBundleVersion"))
        .unwrap_or_default();
    let name = plist_str(&v, "CFBundleDisplayName").or_else(|| plist_str(&v, "CFBundleName")).unwrap_or(stem);
    (id, version, name)
}

fn is_running(name: &str) -> bool {
    util::run("pgrep", &["-x", name]).map(|s| !s.trim().is_empty()).unwrap_or(false)
}

/// 收集目录下的 .app（最多向下两层，覆盖 /Applications/Utilities 这类结构）。
fn collect_apps(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if name.ends_with(".app") {
            out.push(p);
        } else if depth > 0 {
            collect_apps(&p, depth - 1, out);
        }
    }
}

#[tauri::command]
pub async fn list_apps() -> Vec<AppEntry> {
    let mut paths = vec![];
    collect_apps(Path::new("/Applications"), 1, &mut paths);
    collect_apps(&home().join("Applications"), 1, &mut paths);
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|p| {
            let (bundle_id, version, name) = read_info(&p);
            let size = cached_size(&p);
            AppEntry { running: is_running(&name), name, path: p.to_string_lossy().into_owned(), bundle_id, version, size }
        })
        .collect()
}

/// 可能残留该应用数据的目录（均位于 ~/Library 下）。
const LIB_DIRS: &[(&str, &str)] = &[
    ("Library/Application Support", "应用数据"),
    ("Library/Caches", "缓存"),
    ("Library/Preferences", "偏好设置"),
    ("Library/Containers", "沙盒容器"),
    ("Library/Group Containers", "共享容器"),
    ("Library/Saved Application State", "窗口状态"),
    ("Library/LaunchAgents", "登录代理"),
    ("Library/Logs", "日志"),
    ("Library/WebKit", "网页数据"),
    ("Library/HTTPStorages", "网络存储"),
];

#[tauri::command]
pub async fn app_residues(bundle_id: String, app_name: String) -> Vec<Residue> {
    let id = bundle_id.to_lowercase();
    let name = app_name.to_lowercase();
    let h = home();
    let mut out = vec![];
    for (rel, kind) in LIB_DIRS {
        let dir = h.join(rel);
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let entry = e.file_name().to_string_lossy().into_owned();
            let low = entry.to_lowercase();
            // 偏好设置是 <bundleid>.plist，其余通常是 bundleid 或应用名目录
            let hit = if !id.is_empty() && low.contains(&id) {
                true
            } else if matches!(*kind, "应用数据" | "日志" | "缓存" | "沙盒容器" | "共享容器") && !name.is_empty() {
                low == name || low == format!("{name}.savedstate")
            } else {
                false
            };
            if !hit {
                continue;
            }
            let p = e.path();
            out.push(Residue { size: cached_size(&p), path: p.to_string_lossy().into_owned(), kind: (*kind).into() });
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}

fn app_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/Applications"), home().join("Applications")]
}

#[tauri::command]
pub async fn uninstall_app(app_path: String, residue_paths: Vec<String>) -> Result<(usize, u64), String> {
    let app = Path::new(&app_path);
    if app.extension().map(|e| e != "app").unwrap_or(true) {
        return Err("只能卸载 .app 应用".into());
    }
    let roots = app_roots();
    let canon = app.canonicalize().map_err(|e| format!("应用路径无效：{e}"))?;
    if !roots.iter().any(|r| r.canonicalize().map(|rc| canon.starts_with(&rc)).unwrap_or(false)) {
        return Err("只能卸载「应用程序」文件夹中的应用".into());
    }

    let lib = home().join("Library");
    let mut targets = vec![canon];
    for p in residue_paths {
        let path = Path::new(&p);
        let ok = path.canonicalize().map(|c| c.starts_with(&lib) && c != lib).unwrap_or(false);
        if ok {
            targets.push(path.to_path_buf());
        }
    }

    let mut removed = 0;
    let mut freed = 0u64;
    let mut errors = vec![];
    for p in targets {
        let size = util::disk_usage(&p);
        match trash::delete(&p) {
            Ok(_) => {
                removed += 1;
                freed += size;
            }
            Err(e) => errors.push(format!("{}：{e}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
        }
    }
    if removed == 0 {
        return Err(errors.join("；"));
    }
    if !errors.is_empty() {
        eprintln!("[apps] 部分残留未能删除：{}", errors.join("；"));
    }
    Ok((removed, freed))
}
