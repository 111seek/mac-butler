mod cleaner;
mod apps;
mod autostart;
mod browsers;
mod collections;
mod dupfile;
mod desktop;
mod icons;
mod monitor;
mod startup;
mod util;
mod watcher;
mod widgets;

use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, RunEvent, WindowEvent};

#[derive(serde::Serialize)]
struct PermissionStatus {
    desktop: bool,
    full_disk: bool,
}

/// 检测关键权限：桌面访问（TCC 文件夹权限）与完全磁盘访问（读废纸篓/邮件目录作为探针）。
#[tauri::command]
fn check_permissions() -> PermissionStatus {
    let h = util::home();
    PermissionStatus {
        desktop: util::readable(&h.join("Desktop")),
        full_disk: util::readable(&h.join("Library/Mail")) || util::readable(&h.join(".Trash")),
    }
}

#[tauri::command]
fn open_privacy_settings(pane: String) -> Result<(), String> {
    let url = match pane.as_str() {
        "full_disk" => "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
        "automation" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
        _ => "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders",
    };
    util::run("open", &[url]).map(|_| ())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_drag::init())
        .manage(monitor::Monitor::new())
        .manage(collections::load_state())
        .manage(widgets::load_state())
        // 文件图标 / 缩略图：icon://localhost/<urlencoded path>?s=<size>
        .register_asynchronous_uri_scheme_protocol("icon", |_ctx, req, responder| {
            let raw = req.uri().path().trim_start_matches('/').to_string();
            let size = req
                .uri()
                .query()
                .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("s=")))
                .and_then(|v| v.parse().ok())
                .unwrap_or(48);
            std::thread::spawn(move || {
                let path = icons::percent_decode(&raw);
                let resp = match icons::icon_png(&path, size) {
                    Some(b) => tauri::http::Response::builder()
                        .header("Content-Type", "image/png")
                        .header("Cache-Control", "max-age=600")
                        .body(b),
                    None => tauri::http::Response::builder().status(404).body(Vec::new()),
                };
                responder.respond(resp.unwrap());
            });
        })
        .on_menu_event(|app, ev| {
            if widgets::handle_menu(app, &ev) {
                return;
            }
            collections::handle_menu(app, ev);
        })
        .on_window_event(|window, event| {
            // 关闭主窗口时只隐藏：桌面集合需要常驻
            if window.label() == "main" || window.label() == "mini" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
                return;
            }
            if window.label().starts_with("wg-") {
                widgets::on_window_event(window, event);
                return;
            }
            collections::on_window_event(window, event);
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let menu = collections::build_tray_menu(&handle)?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .icon_as_template(true)
                .tooltip("Mac 管家")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        toggle_mini(tray.app_handle());
                    }
                })
                .build(app)?;
            collections::init(&handle);
            widgets::init(&handle);
            // FSEvents：桌面与集合目录即时监听
            watcher::start(handle.clone(), &[collections::desktop_dir(), collections::store_root()]);
            start_tray_stats(handle.clone());
        // 图标 / 拖拽缓存：启动时清理 14 天未使用的内容
        std::thread::spawn(|| { let _ = icons::prune_cache(14); });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_permissions,
            open_privacy_settings,
            desktop::open_path,
            desktop::reveal_in_finder,
            monitor::system_overview,
            monitor::app_memory,
            autostart::autostart_status,
            autostart::set_autostart,
            collections::sweep_preview,
            collections::run_sweep,
            collections::set_auto_move,
            monitor::top_processes,
            monitor::kill_process,
            cleaner::scan_junk,
            cleaner::clean_items,
            cleaner::empty_trash,
            cleaner::scan_large_files,
            cleaner::trash_user_file,
            startup::list_startup_items,
            startup::set_launch_agent_enabled,
            startup::remove_login_item,
            icons::icon_file,
            collections::list_collections,
            collections::get_collection,
            collections::collection_items,
            collections::unassigned_items,
            collections::update_collection,
            collections::create_collection,
            collections::delete_collection,
            collections::add_to_collection,
            collections::remove_from_collection,
            collections::drag_finished,
            collections::return_collection_files,
            collections::set_transparency,
            collections::recent_moves,
            collections::undo_move,
            collections::clear_move_history,
            apps::list_apps,
            apps::app_residues,
            apps::uninstall_app,
            dupfile::find_duplicates,
            dupfile::trash_duplicates,
            browsers::scan_browsers,
            browsers::clean_browser_cache,
            icons::cache_size,
            icons::prune_icon_cache,
            collections::set_collapsed,
            collections::get_prefs,
            collections::set_prefs,
            collections::set_finder_icons,
            collections::list_volumes,
            collections::popup_collection_menu,
            collections::confirm_dialog,
            collections::pick_folder,
            widgets::list_widgets,
            widgets::get_widget,
            widgets::create_widget,
            widgets::update_widget,
            widgets::delete_widget,
            widgets::popup_widget_menu,
            collections::open_main,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        // 点击 Dock 图标时重新显示主窗口
        if let RunEvent::Reopen { .. } = event {
            collections::show_main(app);
        }
    });
}

/// 迷你面板延迟到第一次点击菜单栏图标时创建，减少常驻内存。
fn create_mini(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    if app.get_webview_window("mini").is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "mini", WebviewUrl::App("index.html#/mini".into()))
        .title("Mac 管家")
        .inner_size(320.0, 224.0)
        .decorations(false)
        .transparent(true)
        .shadow(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .effects(EffectsBuilder::new().effect(Effect::HudWindow).state(EffectState::Active).radius(14.0).build())
        .build()?;
    Ok(())
}

/// 点击菜单栏图标：显示 / 隐藏迷你面板。
fn toggle_mini(app: &tauri::AppHandle) {
    if app.get_webview_window("mini").is_none() {
        let _ = create_mini(app);
    }
    let Some(w) = app.get_webview_window("mini") else { return };
    if w.is_visible().unwrap_or(false) {
        let _ = w.hide();
        return;
    }
    // 面板出现在菜单栏图标下方，并限制在当前屏幕内
    if let Ok(Some(m)) = app.cursor_position().map(Some) {
        let mon = app.monitor_from_point(m.x, m.y).ok().flatten();
        let scale = mon.as_ref().map(|x| x.scale_factor()).unwrap_or(1.0);
        let (pw, ph) = (320.0, 224.0);
        let (mx, my, mw, mh) = mon
            .as_ref()
            .map(|x| {
                let p = x.position().to_logical::<f64>(scale);
                let s = x.size().to_logical::<f64>(scale);
                (p.x, p.y, s.width, s.height)
            })
            .unwrap_or((0.0, 0.0, 1440.0, 900.0));
        let x = (m.x / scale - pw / 2.0).clamp(mx + 8.0, (mx + mw - pw - 8.0).max(mx + 8.0));
        let y = (m.y / scale + 6.0).clamp(my + 8.0, (my + mh - ph - 8.0).max(my + 8.0));
        let _ = w.set_position(tauri::LogicalPosition::new(x, y));
    }
    let _ = w.show();
    let _ = w.set_focus();
}

/// 菜单栏显示 CPU / 内存（可在托盘菜单中关闭）。
fn start_tray_stats(app: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        let enabled = app
            .state::<collections::CollectionsState>()
            .store
            .lock()
            .map(|s| s.tray_stats)
            .unwrap_or(false);
        if let Some(tray) = app.tray_by_id("main") {
            if enabled {
                let mon = app.state::<monitor::Monitor>();
                let (cpu, mem) = {
                    let mut sys = mon.0.lock().unwrap();
                    sys.refresh_cpu_usage();
                    sys.refresh_memory();
                    let total = sys.total_memory().max(1);
                    (sys.global_cpu_usage(), sys.used_memory() as f64 / total as f64 * 100.0)
                };
                let _ = tray.set_title(Some(format!("{:.0}% · {:.0}%", cpu, mem)));
            } else {
                let _ = tray.set_title(None::<&str>);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    });
}
