//! 桌面小组件：系统概览 / 内存排行，浮在桌面上，可切换小 / 中 / 大三种尺寸。

use crate::util::home;
use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Window, WindowEvent, Wry,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Widget {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub size: String,
    #[serde(default = "yes")]
    pub visible: bool,
}
fn yes() -> bool { true }

#[derive(Serialize, Deserialize, Default)]
pub struct WStore {
    pub widgets: Vec<Widget>,
}

pub struct WidgetsState {
    pub store: Mutex<WStore>,
    pub dirty: AtomicBool,
}

fn store_path() -> PathBuf {
    let dir = home().join("Library/Application Support/mac-butler");
    let _ = fs::create_dir_all(&dir);
    dir.join("widgets.json")
}

pub fn load_state() -> WidgetsState {
    let store = fs::read(store_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    WidgetsState { store: Mutex::new(store), dirty: AtomicBool::new(false) }
}

fn save(s: &WStore) {
    if let Ok(b) = serde_json::to_vec_pretty(s) {
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

fn label_of(id: &str) -> String {
    format!("wg-{id}")
}

fn preset(size: &str) -> (f64, f64) {
    match size {
        "s" => (260.0, 170.0),
        "l" => (460.0, 330.0),
        _ => (340.0, 230.0),
    }
}

fn title_of(kind: &str) -> &'static str {
    if kind == "memory" { "内存排行" } else { "系统概览" }
}

fn on_desktop_level(w: &WebviewWindow) {
    let w2 = w.clone();
    let _ = w.run_on_main_thread(move || {
        if let Ok(p) = w2.ns_window() {
            let nsw: &NSWindow = unsafe { &*(p as *const NSWindow) };
            nsw.setLevel(crate::collections::DESKTOP_COLLECTION_LEVEL);
            nsw.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::Stationary
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
        }
    });
}

fn open_window(app: &AppHandle, w: &Widget) {
    let label = label_of(&w.id);
    if app.get_webview_window(&label).is_some() {
        return;
    }
    let url = format!("index.html#/widget/{}", w.id);
    let built = WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(title_of(&w.kind))
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .skip_taskbar(true)
        .resizable(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .focused(false)
        .position(w.x, w.y)
        .inner_size(w.w, w.h)
        .min_inner_size(180.0, 120.0)
        .effects(EffectsBuilder::new().effect(Effect::HudWindow).state(EffectState::Active).radius(12.0).build())
        .build();
    match built {
        Ok(win) => on_desktop_level(&win),
        Err(e) => eprintln!("创建小组件窗口失败: {e}"),
    }
}

pub fn sync_windows(app: &AppHandle) {
    let widgets = app.state::<WidgetsState>().store.lock().unwrap().widgets.clone();
    for w in &widgets {
        let win = app.get_webview_window(&label_of(&w.id));
        match (w.visible, win) {
            (true, None) => open_window(app, w),
            (false, Some(x)) => { let _ = x.destroy(); }
            _ => {}
        }
    }
    for (label, win) in app.webview_windows() {
        if let Some(id) = label.strip_prefix("wg-") {
            if !widgets.iter().any(|w| w.id == id) {
                let _ = win.destroy();
            }
        }
    }
}

fn commit(app: &AppHandle) {
    save(&app.state::<WidgetsState>().store.lock().unwrap());
    let _ = app.emit("widgets-changed", ());
    sync_windows(app);
}

pub fn init(app: &AppHandle) {
    sync_windows(app);
}

/// 由 collections 的统一后台线程调用：位置 / 尺寸变化后节流写盘。
pub fn flush(app: &AppHandle) {
    let st = app.state::<WidgetsState>();
    if st.dirty.swap(false, Ordering::Relaxed) {
        save(&st.store.lock().unwrap());
    }
}

#[tauri::command]
pub fn list_widgets(app: AppHandle) -> Vec<Widget> {
    app.state::<WidgetsState>().store.lock().unwrap().widgets.clone()
}

#[tauri::command]
pub fn get_widget(app: AppHandle, id: String) -> Option<Widget> {
    app.state::<WidgetsState>().store.lock().unwrap().widgets.iter().find(|w| w.id == id).cloned()
}

/// 在主线程取鼠标位置与屏幕区域后新建小组件。
#[tauri::command]
pub async fn create_widget(app: AppHandle, kind: String) -> Result<Widget, String> {
    let place = crate::collections::on_main(&app, || (crate::collections::cursor_logical(), crate::collections::screen_rects())).await?;
    let (cursor, rects) = place;
    let size = "m".to_string();
    let (w, h) = preset(&size);
    let (cx, cy) = cursor.unwrap_or((400.0, 300.0));
    let (x, y) = crate::collections::clamp_to_screens(cx - w / 2.0, cy - h / 2.0, w, h, &rects);
    let widget = Widget { id: new_id(), kind, x, y, w, h, size, visible: true };
    {
        let st = app.state::<WidgetsState>();
        st.store.lock().unwrap().widgets.push(widget.clone());
    }
    open_window(&app, &widget);
    commit(&app);
    Ok(widget)
}

#[derive(Deserialize)]
pub struct WPatch {
    pub id: String,
    pub size: Option<String>,
    pub visible: Option<bool>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub w: Option<f64>,
    pub h: Option<f64>,
}

#[tauri::command]
pub async fn update_widget(app: AppHandle, patch: WPatch) -> Result<(), String> {
    let mut resize = None;
    {
        let st = app.state::<WidgetsState>();
        let mut s = st.store.lock().unwrap();
        let Some(widget) = s.widgets.iter_mut().find(|w| w.id == patch.id) else { return Err("小组件不存在".into()) };
        if let Some(v) = patch.size.clone() {
            let (w, h) = preset(&v);
            widget.size = v;
            widget.w = w;
            widget.h = h;
            resize = Some((w, h));
        }
        if let Some(v) = patch.visible { widget.visible = v; }
        if let Some(v) = patch.x { widget.x = v; }
        if let Some(v) = patch.y { widget.y = v; }
        if let Some(v) = patch.w { widget.w = v; }
        if let Some(v) = patch.h { widget.h = v; }
    }
    if let Some((w, h)) = resize {
        if let Some(win) = app.get_webview_window(&label_of(&patch.id)) {
            let _ = win.set_size(LogicalSize::new(w, h));
        }
    }
    commit(&app);
    Ok(())
}

#[tauri::command]
pub async fn delete_widget(app: AppHandle, id: String) {
    {
        let st = app.state::<WidgetsState>();
        st.store.lock().unwrap().widgets.retain(|w| w.id != id);
    }
    commit(&app);
}

/// 记录拖动 / 缩放后的位置尺寸。
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let label = window.label().to_string();
    let Some(id) = label.strip_prefix("wg-").map(str::to_string) else { return };
    let app = window.app_handle();
    let st = app.state::<WidgetsState>();
    let scale = window.scale_factor().unwrap_or(1.0);
    match event {
        WindowEvent::Moved(p) => {
            let l = p.to_logical::<f64>(scale);
            if let Some(w) = st.store.lock().unwrap().widgets.iter_mut().find(|w| w.id == id) {
                // 拖动时若与预设尺寸接近，保留预设；否则视为自定义尺寸
                w.x = l.x;
                w.y = l.y;
            }
            st.dirty.store(true, Ordering::Relaxed);
        }
        WindowEvent::Resized(s) => {
            let l = s.to_logical::<f64>(scale);
            if let Some(w) = st.store.lock().unwrap().widgets.iter_mut().find(|w| w.id == id) {
                w.w = l.width;
                w.h = l.height;
            }
            st.dirty.store(true, Ordering::Relaxed);
        }
        WindowEvent::CloseRequested { api, .. } => api.prevent_close(),
        _ => {}
    }
}

// ---------------------------------------------------------------- 菜单

fn item(app: &AppHandle, id: &str, text: &str) -> tauri::Result<MenuItem<Wry>> {
    MenuItem::with_id(app, id, text, true, None::<&str>)
}
fn check(app: &AppHandle, id: &str, text: &str, on: bool) -> tauri::Result<CheckMenuItem<Wry>> {
    CheckMenuItem::with_id(app, id, text, true, on, None::<&str>)
}

pub fn new_submenu(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
    let m = Submenu::new(app, "新建小组件", true)?;
    m.append(&item(app, "w:new:overview", "系统概览 — CPU / 内存 / 磁盘")?)?;
    m.append(&item(app, "w:new:memory", "内存排行 — 占用最高的应用")?)?;
    Ok(m)
}

fn build_menu(app: &AppHandle, w: &Widget) -> tauri::Result<Menu<Wry>> {
    let m = Menu::new(app)?;
    let sizes = Submenu::new(app, "小组件大小", true)?;
    for (k, label) in [("s", "小"), ("m", "中"), ("l", "大")] {
        sizes.append(&check(app, &format!("w:size:{}:{k}", w.id), label, w.size == k)?)?;
    }
    m.append(&sizes)?;
    m.append(&PredefinedMenuItem::separator(app)?)?;
    m.append(&item(app, &format!("w:hide:{}", w.id), "隐藏此小组件")?)?;
    m.append(&item(app, &format!("w:delete:{}", w.id), "删除此小组件")?)?;
    Ok(m)
}

#[tauri::command]
pub fn popup_widget_menu(app: AppHandle, window: WebviewWindow, id: String) -> Result<(), String> {
    let w = app
        .state::<WidgetsState>()
        .store
        .lock()
        .unwrap()
        .widgets
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or("小组件不存在")?;
    let menu = build_menu(&app, &w).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

/// 返回是否已处理该菜单事件。
pub fn handle_menu(app: &AppHandle, ev: &MenuEvent) -> bool {
    let id = ev.id().as_ref().to_string();
    let mut parts = id.splitn(3, ':');
    let (a, b, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if a != "w" {
        return false;
    }
    match (b, rest) {
        ("new", kind) => {
            let app = app.clone();
            let kind = kind.to_string();
            tauri::async_runtime::spawn(async move {
                let _ = create_widget(app, kind).await;
            });
        }
        ("size", _) => {
            if let Some((wid, size)) = rest.split_once(':') {
                let wid = wid.to_string();
                let size = size.to_string();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = update_widget(app, WPatch { id: wid, size: Some(size), visible: None, x: None, y: None, w: None, h: None }).await;
                });
            }
        }
        ("hide", wid) => {
            let wid = wid.to_string();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = update_widget(app, WPatch { id: wid, size: None, visible: Some(false), x: None, y: None, w: None, h: None }).await;
            });
        }
        ("delete", wid) => {
            let wid = wid.to_string();
            let app = app.clone();
            tauri::async_runtime::spawn(async move { delete_widget(app, wid).await });
        }
        _ => {}
    }
    true
}
