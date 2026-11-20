//! 目录变化监听：直接调用 FSEvents，桌面和集合目录一有变化立即通知，
//! 替代原来每 3 秒遍历目录的轮询。

use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;
use tauri::AppHandle;

type CFAllocatorRef = *const c_void;
type CFArrayRef = *const c_void;
type CFStringRef = *const c_void;
type FSEventStreamRef = *const c_void;
type DispatchQueue = *mut c_void;

const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
/// kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer
const FLAGS: u32 = 0x0000_0010 | 0x0000_0002;

#[repr(C)]
struct FSEventStreamContext {
    version: isize,
    info: *mut c_void,
    retain: Option<extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<extern "C" fn(*const c_void)>,
    copy_description: Option<extern "C" fn(*const c_void) -> *const c_void>,
}

type FSEventStreamCallback =
    extern "C" fn(FSEventStreamRef, *mut c_void, usize, *mut c_void, *const u32, *const u64);

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn FSEventStreamCreate(
        allocator: CFAllocatorRef,
        callback: FSEventStreamCallback,
        context: *mut FSEventStreamContext,
        paths_to_watch: CFArrayRef,
        since_when: u64,
        latency: f64,
        flags: u32,
    ) -> FSEventStreamRef;
    fn FSEventStreamSetDispatchQueue(stream: FSEventStreamRef, queue: DispatchQueue);
    fn FSEventStreamStart(stream: FSEventStreamRef) -> u8;
    fn FSEventStreamStop(stream: FSEventStreamRef);
    fn FSEventStreamInvalidate(stream: FSEventStreamRef);
    fn FSEventStreamRelease(stream: FSEventStreamRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(alloc: CFAllocatorRef, s: *const c_char, encoding: u32) -> CFStringRef;
    fn CFArrayCreate(
        alloc: CFAllocatorRef,
        values: *const *const c_void,
        num_values: isize,
        callbacks: *const c_void,
    ) -> CFArrayRef;
    fn CFRelease(cf: *const c_void);
    static kCFTypeArrayCallBacks: u8;
}

extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> DispatchQueue;
}

static APP: OnceLock<AppHandle> = OnceLock::new();
/// 当前的事件流指针（0 表示没有）。重建监听时需要先停掉旧的。
static STREAM: std::sync::Mutex<usize> = std::sync::Mutex::new(0);

extern "C" fn on_event(
    _stream: FSEventStreamRef,
    _info: *mut c_void,
    _num_events: usize,
    _paths: *mut c_void,
    _flags: *const u32,
    _ids: *const u64,
) {
    if let Some(app) = APP.get() {
        crate::collections::fs_changed(app);
    }
}

/// 开始监听（重复调用无副作用）。
pub fn start(app: AppHandle, dirs: &[std::path::PathBuf]) {
    let _ = APP.set(app);
    restart(dirs);
}

/// 重建监听：文件夹合集改变后需要把新目录加入监听列表。
pub fn restart(dirs: &[std::path::PathBuf]) {
    {
        let mut cur = STREAM.lock().unwrap();
        if *cur != 0 {
            let stream = *cur as FSEventStreamRef;
            unsafe {
                FSEventStreamStop(stream);
                FSEventStreamInvalidate(stream);
                FSEventStreamRelease(stream);
            }
            *cur = 0;
        }
    }
    let dirs: Vec<std::path::PathBuf> = dirs
        .iter()
        .filter(|d| {
            let _ = std::fs::create_dir_all(d);
            d.is_dir()
        })
        .cloned()
        .collect();
    if dirs.is_empty() {
        return;
    }
    std::thread::Builder::new()
        .name("fsevents".into())
        .spawn(move || unsafe {
            let cstrings: Vec<CString> = dirs
                .iter()
                .map(|d| CString::new(d.to_string_lossy().as_bytes()).unwrap_or_default())
                .collect();
            let cf_strings: Vec<CFStringRef> = cstrings
                .iter()
                .map(|c| CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8))
                .collect();
            let array = CFArrayCreate(
                std::ptr::null(),
                cf_strings.as_ptr() as *const *const c_void,
                cf_strings.len() as isize,
                &kCFTypeArrayCallBacks as *const u8 as *const c_void,
            );
            let mut ctx = FSEventStreamContext {
                version: 0,
                info: std::ptr::null_mut(),
                retain: None,
                release: None,
                copy_description: None,
            };
            let stream = FSEventStreamCreate(
                std::ptr::null(),
                on_event,
                &mut ctx,
                array,
                0,
                0.3,
                FLAGS,
            );
            CFRelease(array as *const c_void);
            for s in cf_strings {
                CFRelease(s as *const c_void);
            }
            if stream.is_null() {
                eprintln!("[watcher] FSEvents 创建失败，将使用兜底轮询");
                return;
            }
            let label = CString::new("com.macbutler.fsevents").unwrap();
            let queue = dispatch_queue_create(label.as_ptr(), std::ptr::null_mut());
            FSEventStreamSetDispatchQueue(stream, queue);
            if FSEventStreamStart(stream) == 0 {
                eprintln!("[watcher] FSEvents 启动失败，将使用兜底轮询");
                FSEventStreamInvalidate(stream);
                FSEventStreamRelease(stream);
                return;
            }
            *STREAM.lock().unwrap() = stream as usize;
            // 线程到此结束；事件由 dispatch queue 回调。
        })
        .ok();
}

#[allow(dead_code)]
pub fn stop() {
    let mut cur = STREAM.lock().unwrap();
    if *cur != 0 {
        let stream = *cur as FSEventStreamRef;
        unsafe {
            FSEventStreamStop(stream);
            FSEventStreamInvalidate(stream);
            FSEventStreamRelease(stream);
        }
        *cur = 0;
    }
}
