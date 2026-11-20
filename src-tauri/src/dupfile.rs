//! 重复文件查找：按「大小 → 首块哈希 → 全文件哈希」三级过滤，避免对每个文件都做全量哈希。

use crate::util::{self, home};
use serde::Serialize;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::fs::{self, File};
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct DupFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
}

#[derive(Serialize)]
pub struct DupGroup {
    pub size: u64,
    /// 该组里除保留一份外，可释放的空间
    pub wasted: u64,
    pub files: Vec<DupFile>,
}

const SKIP_DIRS: &[&str] = &["Library", "node_modules", ".git", ".Trash", ".cache", "Applications", ".npm", "Pods", "target", ".venv", "venv"];

fn walk(root: &Path, out: &mut Vec<DupFile>, budget: &mut usize) {
    let Ok(rd) = fs::read_dir(root) else { return };
    for e in rd.flatten() {
        if *budget == 0 {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            walk(&p, out, budget);
        } else if ft.is_file() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.len() == 0 {
                continue;
            }
            *budget -= 1;
            out.push(DupFile {
                path: p.to_string_lossy().into_owned(),
                name,
                size: meta.len(),
                modified: util::modified_secs(&meta),
            });
        }
    }
}

/// 读取前 `limit` 字节并哈希；`0` 表示整份文件。
fn hash_file(path: &str, limit: u64) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let mut h = DefaultHasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut read = 0u64;
    loop {
        let want = if limit == 0 { buf.len() } else { ((limit - read) as usize).min(buf.len()) };
        if want == 0 {
            break;
        }
        let n = f.read(&mut buf[..want]).ok()?;
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
        read += n as u64;
        if limit > 0 && read >= limit {
            break;
        }
    }
    Some(h.finish())
}

/// `root` 必须是用户目录下的普通文件夹（不能是 ~/Library 等）。
#[tauri::command]
pub async fn find_duplicates(root: String, min_kb: u64, max_files: usize) -> Result<Vec<DupGroup>, String> {
    let h = home();
    // 允许传「Desktop」「Downloads」这类相对名称
    let base = if root.starts_with('/') { PathBuf::from(&root) } else { h.join(&root) };
    let canon = base.canonicalize().map_err(|e| format!("路径无效：{e}"))?;
    if !canon.starts_with(&h) || canon.starts_with(h.join("Library")) || canon == h {
        return Err("只能扫描个人文件夹（如桌面、下载、文稿）".into());
    }
    let min = min_kb.max(1) * 1024;
    let mut files = vec![];
    let mut budget = max_files.clamp(1000, 400_000);
    walk(&canon, &mut files, &mut budget);

    // 1) 按大小分组
    let mut by_size: HashMap<u64, Vec<DupFile>> = HashMap::new();
    for f in files {
        if f.size >= min {
            by_size.entry(f.size).or_default().push(f);
        }
    }

    let mut groups: Vec<DupGroup> = vec![];
    for (size, list) in by_size {
        if list.len() < 2 {
            continue;
        }
        // 2) 首块哈希（最多 1MB）
        let head = size.min(1024 * 1024);
        let mut by_head: HashMap<u64, Vec<DupFile>> = HashMap::new();
        for f in list {
            if let Some(k) = hash_file(&f.path, head) {
                by_head.entry(k).or_default().push(f);
            }
        }
        for (_, cands) in by_head {
            if cands.len() < 2 {
                continue;
            }
            // 3) 全文件哈希（首块已经覆盖整个小文件时可以直接复用）
            let mut by_full: HashMap<u64, Vec<DupFile>> = HashMap::new();
            for f in cands {
                let k = if size <= head { hash_file(&f.path, head) } else { hash_file(&f.path, 0) };
                if let Some(k) = k {
                    by_full.entry(k).or_default().push(f);
                }
            }
            for (_, mut fs_) in by_full {
                if fs_.len() < 2 {
                    continue;
                }
                fs_.sort_by_key(|f| f.path.clone());
                let wasted = size * (fs_.len() as u64 - 1);
                groups.push(DupGroup { size, wasted, files: fs_ });
            }
        }
    }
    groups.sort_by(|a, b| b.wasted.cmp(&a.wasted));
    groups.truncate(200);
    Ok(groups)
}

/// 把指定的重复文件移到废纸篓（只允许用户目录下、非 Library 的文件）。
#[tauri::command]
pub async fn trash_duplicates(paths: Vec<String>) -> Result<(usize, u64), String> {
    let h = home();
    let lib = h.join("Library");
    let mut n = 0;
    let mut freed = 0u64;
    let mut errors = vec![];
    for p in paths {
        let path = Path::new(&p);
        let ok = path.canonicalize().map(|c| c.starts_with(&h) && !c.starts_with(&lib)).unwrap_or(false);
        if !ok {
            continue;
        }
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        match trash::delete(path) {
            Ok(_) => {
                n += 1;
                freed += size;
            }
            Err(e) => errors.push(format!("{}：{e}", path.display())),
        }
    }
    if n == 0 && !errors.is_empty() {
        return Err(errors.join("；"));
    }
    Ok((n, freed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_content() {
        let dir = std::env::temp_dir().join(format!("mb-dup-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let c = dir.join("c.txt");
        fs::write(&a, b"hello duplicate world").unwrap();
        fs::write(&b, b"hello duplicate world").unwrap();
        fs::write(&c, b"hello different world!").unwrap();
        assert_eq!(hash_file(a.to_str().unwrap(), 0), hash_file(b.to_str().unwrap(), 0));
        assert_ne!(hash_file(a.to_str().unwrap(), 0), hash_file(c.to_str().unwrap(), 0));
        // 小文件用首块哈希即可判定
        assert_eq!(hash_file(a.to_str().unwrap(), 1024), hash_file(b.to_str().unwrap(), 1024));
        let _ = fs::remove_dir_all(&dir);
    }
}
