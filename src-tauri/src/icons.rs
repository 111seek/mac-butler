//! 文件图标 / 图片缩略图：用 NSWorkspace 取 Finder 同款图标，渲染成 PNG。
//! 通过自定义协议 `icon://localhost/<urlencoded path>?s=<size>` 提供给前端。

use crate::util;
use objc2::rc::{autoreleasepool, Retained};
use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage, NSWorkspace};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "heic", "webp", "bmp", "tiff", "tif"];

fn cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    static C: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 返回指定边长（逻辑像素）的 PNG，内部按 2x 渲染以适配 Retina。
pub fn icon_png(path: &str, size: u32) -> Option<Vec<u8>> {
    let size = size.clamp(16, 256);
    let meta = std::fs::metadata(path).ok();
    let mtime = meta.as_ref().map(util::modified_secs).unwrap_or(0);
    let key = format!("{path}|{size}|{mtime}");
    if let Some(b) = cache().lock().unwrap().get(&key) {
        return Some(b.clone());
    }
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let thumb = IMAGE_EXTS.contains(&ext.as_str()) && meta.as_ref().is_some_and(|m| m.is_file() && m.len() < 40 * 1024 * 1024);
    let bytes = autoreleasepool(|_| render(path, size, thumb))?;
    let mut c = cache().lock().unwrap();
    if c.len() > 3000 {
        c.clear();
    }
    c.insert(key, bytes.clone());
    Some(bytes)
}

fn render(path: &str, size: u32, thumb: bool) -> Option<Vec<u8>> {
    let ns_path = NSString::from_str(path);
    let img: Retained<NSImage> = if thumb {
        NSImage::initWithContentsOfFile(NSImage::alloc(), &ns_path)
            .unwrap_or_else(|| NSWorkspace::sharedWorkspace().iconForFile(&ns_path))
    } else {
        NSWorkspace::sharedWorkspace().iconForFile(&ns_path)
    };
    let px = (size * 2) as f64;
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            px as isize,
            px as isize,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )
    }?;
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    let s = img.size();
    if s.width > 0.0 && s.height > 0.0 {
        let k = (px / s.width).min(px / s.height);
        let (w, h) = (s.width * k, s.height * k);
        img.drawInRect(NSRect::new(NSPoint::new((px - w) / 2.0, (px - h) / 2.0), NSSize::new(w, h)));
    }
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

/// 拖拽时需要一个 PNG 文件作为拖拽预览图。
#[tauri::command]
pub async fn icon_file(path: String) -> Result<String, String> {
    let bytes = icon_png(&path, 64).ok_or("无法生成图标")?;
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    let dir: PathBuf = util::home().join("Library/Caches/mac-butler/drag");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{:x}.png", h.finish()));
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    Ok(file.to_string_lossy().into_owned())
}

pub fn cache_root() -> PathBuf {
    util::home().join("Library/Caches/mac-butler")
}

/// 删除超过 `days` 天未更新的缓存文件（图标、拖拽预览图），并清理空目录。
/// 返回删除的文件数与释放的字节数。
pub fn prune_cache(days: u64) -> (usize, u64) {
    use std::os::unix::fs::MetadataExt;
    let root = cache_root();
    // days == 0 表示立即清理全部磁盘缓存
    let cutoff = if days == 0 {
        std::time::SystemTime::now() + std::time::Duration::from_secs(1)
    } else {
        let Some(t) = std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(days * 86400)) else { return (0, 0) };
        t
    };
    let mut files = 0;
    let mut freed = 0u64;
    let mut dirs = vec![];
    let mut stack = vec![root.clone()];
    while let Some(p) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&p) else { continue };
        for e in rd.flatten() {
            let path = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            if meta.is_dir() {
                dirs.push(path.clone());
                stack.push(path);
            } else if meta.modified().map(|t| t < cutoff).unwrap_or(false) && std::fs::remove_file(&path).is_ok() {
                files += 1;
                freed += meta.blocks() * 512;
            }
        }
    }
    // 由深到浅删除空目录
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        if d != root && std::fs::read_dir(&d).map(|r| r.count() == 0).unwrap_or(false) {
            let _ = std::fs::remove_dir(&d);
        }
    }
    (files, freed)
}

/// 当前缓存占用（供设置页展示）。
#[tauri::command]
pub async fn cache_size() -> u64 {
    util::disk_usage(&cache_root())
}

#[tauri::command]
pub async fn prune_icon_cache() -> Result<u64, String> {
    cache().lock().unwrap().clear();
    let (_, freed) = prune_cache(0);
    Ok(freed)
}
