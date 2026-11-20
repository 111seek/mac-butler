//! 系统监控（对标电脑管家「性能加速」）：CPU / 内存 / 磁盘 / 进程。

use serde::Serialize;
use std::sync::Mutex;
use sysinfo::{Disks, Pid, ProcessesToUpdate, System};

/// 全局共享的 System 实例：CPU 使用率需要两次采样的差值，所以必须复用。
pub struct Monitor(pub Mutex<System>);

impl Monitor {
    pub fn new() -> Self {
        let mut s = System::new_all().expect("初始化 sysinfo 失败");
        s.refresh_cpu_usage();
        Monitor(Mutex::new(s))
    }
}

#[derive(Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub available: u64,
}

#[derive(Serialize)]
pub struct Overview {
    pub cpu_usage: f32,
    pub cpu_count: usize,
    pub cpu_brand: String,
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub disks: Vec<DiskInfo>,
    pub uptime: u64,
    pub os_version: String,
    pub host_name: String,
}

#[derive(Serialize)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
}

#[tauri::command]
pub fn system_overview(state: tauri::State<'_, Monitor>) -> Overview {
    let mut sys = state.0.lock().unwrap();
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    let disk_list = Disks::new_with_refreshed_list().ok();
    let mut disks: Vec<DiskInfo> = disk_list
        .as_ref()
        .map(|d| &d[..])
        .unwrap_or_default()
        .iter()
        // macOS 上 / 与 /System/Volumes/Data 共享同一个 APFS 容器，只展示根卷与外接盘
        .filter(|d| {
            let m = d.mount_point().to_string_lossy();
            m == "/" || m.starts_with("/Volumes/")
        })
        .map(|d| DiskInfo {
            name: d.name().to_string_lossy().into_owned(),
            mount: d.mount_point().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
        })
        .collect();
    disks.dedup_by(|a, b| a.mount == b.mount);
    Overview {
        cpu_usage: sys.global_cpu_usage(),
        cpu_count: sys.cpus().len(),
        cpu_brand: sys.cpus().first().map(|c| c.brand().to_string()).unwrap_or_default(),
        mem_total: sys.total_memory(),
        mem_used: sys.used_memory(),
        swap_total: sys.total_swap(),
        swap_used: sys.used_swap(),
        disks,
        uptime: System::uptime().unwrap_or(0),
        os_version: System::long_os_version().unwrap_or_default(),
        host_name: System::host_name().unwrap_or_default(),
    }
}

/// 按 CPU 或内存排序返回前 N 个进程。
#[tauri::command]
pub fn top_processes(state: tauri::State<'_, Monitor>, sort_by: String, limit: usize) -> Vec<ProcInfo> {
    let mut sys = state.0.lock().unwrap();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut list: Vec<ProcInfo> = sys
        .processes()
        .values()
        .map(|p| ProcInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            cpu: p.cpu_usage(),
            memory: p.memory(),
        })
        .collect();
    if sort_by == "cpu" {
        list.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    } else {
        list.sort_by(|a, b| b.memory.cmp(&a.memory));
    }
    list.truncate(limit);
    list
}

/// 结束进程（只能结束当前用户有权限的进程；系统进程会返回失败）。
#[tauri::command]
pub fn kill_process(state: tauri::State<'_, Monitor>, pid: u32) -> Result<(), String> {
    if pid == std::process::id() {
        return Err("不能结束本程序自身".into());
    }
    let sys = state.0.lock().unwrap();
    let p = sys.process(Pid::from_u32(pid)).ok_or("进程不存在")?;
    if p.kill() {
        Ok(())
    } else {
        Err("结束进程失败（可能是系统进程或权限不足）".into())
    }
}

/// 按「应用」聚合内存占用：从可执行文件路径识别 .app 包，同一应用的多进程合并。
#[derive(Serialize)]
pub struct AppMem {
    pub name: String,
    pub path: String,
    pub memory: u64,
    pub pids: usize,
}

#[tauri::command]
pub fn app_memory(state: tauri::State<'_, Monitor>, limit: usize) -> Vec<AppMem> {
    let mut sys = state.0.lock().unwrap();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let mut agg: std::collections::HashMap<String, (u64, usize, String)> = std::collections::HashMap::new();
    for p in sys.processes().values() {
        let mem = p.memory();
        if mem == 0 {
            continue;
        }
        let Some(exe) = p.exe() else { continue };
        let path = exe.to_string_lossy();
        // /Applications/Foo.app/Contents/MacOS/Foo → /Applications/Foo.app
        let Some(idx) = path.find(".app/") else { continue };
        let bundle = &path[..idx + 4];
        let name = bundle.rsplit('/').next().unwrap_or(bundle).trim_end_matches(".app").to_string();
        let e = agg.entry(bundle.to_string()).or_insert((0, 0, name));
        e.0 += mem;
        e.1 += 1;
    }
    let mut list: Vec<AppMem> = agg
        .into_iter()
        .map(|(path, (memory, pids, name))| AppMem { name, path, memory, pids })
        .collect();
    list.sort_by(|a, b| b.memory.cmp(&a.memory));
    list.truncate(limit.clamp(1, 50));
    list
}
