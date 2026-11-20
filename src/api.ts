// 前端与 Rust 后端之间的类型化调用层。字段名与 Rust 结构体保持一致（snake_case）。
import { invoke } from "@tauri-apps/api/core";

export interface DiskInfo { name: string; mount: string; total: number; available: number }
export interface Overview {
  cpu_usage: number; cpu_count: number; cpu_brand: string;
  mem_total: number; mem_used: number; swap_total: number; swap_used: number;
  disks: DiskInfo[]; uptime: number; os_version: string; host_name: string;
}
export interface ProcInfo { pid: number; name: string; cpu: number; memory: number }
export interface JunkItem { path: string; name: string; size: number }
export interface JunkGroup {
  key: string; title: string; description: string; size: number;
  needs_permission: boolean; recommended: boolean; items: JunkItem[];
}
export interface CleanResult { cleaned: number; freed: number; errors: string[] }
export interface LargeFile { path: string; name: string; size: number; modified: number }
export interface StartupItem {
  label: string; program: string; source: string; path: string;
  enabled: boolean; editable: boolean; kind: "launchd" | "login_item";
}
export interface PermissionStatus { desktop: boolean; full_disk: boolean }
export interface AppEntry { name: string; path: string; bundle_id: string; version: string; size: number; running: boolean }
export interface Residue { path: string; size: number; kind: string }
export interface DupFile { path: string; name: string; size: number; modified: number }
export interface DupGroup { size: number; wasted: number; files: DupFile[] }
export interface BrowserInfo { key: string; name: string; process: string; running: boolean; size: number; paths: string[] }
export interface MoveOp { id: number; time: number; from: string; to: string; name: string; reason: string }
export interface AppMem { name: string; path: string; memory: number; pids: number }
export interface SweepItem { path: string; name: string; target: string; target_id: string; is_dir: boolean }
export type WidgetKind = "overview" | "memory";
export type WidgetSize = "s" | "m" | "l";
export interface Widget {
  id: string; kind: WidgetKind; x: number; y: number; w: number; h: number; size: WidgetSize; visible: boolean;
}

export const api = {
  checkPermissions: () => invoke<PermissionStatus>("check_permissions"),
  openPrivacySettings: (pane: "full_disk" | "automation" | "files") => invoke<void>("open_privacy_settings", { pane }),

  openPath: (path: string) => invoke<void>("open_path", { path }),
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),

  systemOverview: () => invoke<Overview>("system_overview"),
  topProcesses: (sortBy: "cpu" | "memory", limit = 15) => invoke<ProcInfo[]>("top_processes", { sortBy, limit }),
  appMemory: (limit = 8) => invoke<AppMem[]>("app_memory", { limit }),
  killProcess: (pid: number) => invoke<void>("kill_process", { pid }),

  scanJunk: () => invoke<JunkGroup[]>("scan_junk"),
  cleanItems: (paths: string[]) => invoke<CleanResult>("clean_items", { paths }),
  emptyTrash: () => invoke<void>("empty_trash"),
  scanLargeFiles: (minMb = 500, limit = 50) => invoke<LargeFile[]>("scan_large_files", { minMb, limit }),
  trashUserFile: (path: string) => invoke<void>("trash_user_file", { path }),

  listStartupItems: () => invoke<StartupItem[]>("list_startup_items"),
  setLaunchAgentEnabled: (path: string, enabled: boolean) => invoke<void>("set_launch_agent_enabled", { path, enabled }),
  removeLoginItem: (name: string) => invoke<void>("remove_login_item", { name }),

  listApps: () => invoke<AppEntry[]>("list_apps"),
  appResidues: (bundleId: string, appName: string) => invoke<Residue[]>("app_residues", { bundleId, appName }),
  uninstallApp: (appPath: string, residuePaths: string[]) => invoke<[number, number]>("uninstall_app", { appPath, residuePaths }),
  findDuplicates: (root: string, minKb = 1024, maxFiles = 200_000) => invoke<DupGroup[]>("find_duplicates", { root, minKb, maxFiles }),
  trashDuplicates: (paths: string[]) => invoke<[number, number]>("trash_duplicates", { paths }),
  scanBrowsers: () => invoke<BrowserInfo[]>("scan_browsers"),
  cleanBrowserCache: (key: string) => invoke<number>("clean_browser_cache", { key }),
  cacheSize: () => invoke<number>("cache_size"),
  pruneIconCache: () => invoke<number>("prune_icon_cache"),

  /** 原生确认框（WKWebView 不支持 window.confirm，会直接返回 false） */
  confirm: (title: string, message = "", ok = "确定") => invoke<boolean>("confirm_dialog", { title, message, ok }),
  pickFolder: () => invoke<string | null>("pick_folder"),

  listCollections: () => invoke<Collection[]>("list_collections"),
  getCollection: (id: string) => invoke<Collection | null>("get_collection", { id }),
  collectionItems: (id: string) => invoke<CItem[]>("collection_items", { id }),
  unassignedItems: () => invoke<CItem[]>("unassigned_items"),
  updateCollection: (patch: Partial<Collection> & { id: string }) => invoke<void>("update_collection", { patch }),
  createCollection: (kind: CollectionKind, folder?: string) => invoke<Collection>("create_collection", { kind, folder }),
  deleteCollection: (id: string) => invoke<void>("delete_collection", { id }),
  addToCollection: (id: string, paths: string[]) => invoke<number>("add_to_collection", { id, paths }),
  removeFromCollection: (id: string, path: string) => invoke<void>("remove_from_collection", { id, path }),
  dragFinished: (paths: string[]) => invoke<void>("drag_finished", { paths }),
  setTransparency: (id: string, value: number) => invoke<void>("set_transparency", { id, value }),
  recentMoves: () => invoke<MoveOp[]>("recent_moves"),
  undoMove: (id: number) => invoke<void>("undo_move", { id }),
  clearMoveHistory: () => invoke<void>("clear_move_history"),
  sweepPreview: () => invoke<SweepItem[]>("sweep_preview"),
  runSweep: () => invoke<number>("run_sweep"),
  setAutoMove: (enabled: boolean) => invoke<void>("set_auto_move", { enabled }),
  returnCollectionFiles: (id: string) => invoke<number>("return_collection_files", { id }),

  listWidgets: () => invoke<Widget[]>("list_widgets"),
  getWidget: (id: string) => invoke<Widget | null>("get_widget", { id }),
  createWidget: (kind: WidgetKind) => invoke<Widget>("create_widget", { kind }),
  updateWidget: (patch: Partial<Widget> & { id: string }) => invoke<void>("update_widget", { patch }),
  deleteWidget: (id: string) => invoke<void>("delete_widget", { id }),
  popupWidgetMenu: (id: string) => invoke<void>("popup_widget_menu", { id }),
  setCollapsed: (id: string, collapsed: boolean) => invoke<void>("set_collapsed", { id, collapsed }),
  getPrefs: () => invoke<Prefs>("get_prefs"),
  setPrefs: (p: { onTop?: boolean; hiddenAll?: boolean; snap?: boolean; snapGap?: number; trayStats?: boolean; autostart?: boolean }) => invoke<void>("set_prefs", p),
  setAutostart: (enabled: boolean) => invoke<void>("set_autostart", { enabled }),
  setFinderIcons: (show: boolean) => invoke<void>("set_finder_icons", { show }),
  listVolumes: () => invoke<Volume[]>("list_volumes"),
  popupMenu: (id: string, file?: string) => invoke<void>("popup_collection_menu", { id, file }),
  iconFile: (path: string) => invoke<string>("icon_file", { path }),
  openMain: (focus?: string) => invoke<void>("open_main", { focus }),
};

export type CollectionKind = "collection" | "folder" | "dropbox" | "disk";
export interface Collection {
  id: string; name: string; kind: CollectionKind; color: string;
  x: number; y: number; w: number; h: number; visible: boolean; collapsed: boolean;
  exts: string[]; keywords: string[]; dirs: boolean; catch_all: boolean;
  folder: string | null; refs: string[]; dir: string | null; icon_size: number; sort: string; transparency: number;
  title_pos: TitlePos; view: ViewMode; title_size: number; locked: boolean;
}
export interface CItem { name: string; path: string; is_dir: boolean; size: number; modified: number; ext: string; is_ref: boolean }
export interface Volume { name: string; mount: string; total: number; available: number }
export interface Prefs {
  on_top: boolean; hidden_all: boolean; finder_icons: boolean; snap: boolean; snap_gap: number;
  tray_stats: boolean; autostart: boolean; auto_move: boolean;
}
export const TITLE_SIZES = [12, 13, 14, 15, 16, 18, 20, 24];
export type TitlePos = "" | "bottom" | "left" | "right" | "inside";
export type ViewMode = "" | "list" | "mini";
export const TITLE_POS: [TitlePos, string][] = [["", "顶部"], ["bottom", "底部"], ["left", "左侧"], ["right", "右侧"], ["inside", "合集内"]];
export const VIEW_MODE: [ViewMode, string][] = [["", "图标"], ["list", "列表"], ["mini", "迷你"]];

export const COLORS = ["#3b82f6", "#8b5cf6", "#ec4899", "#ef4444", "#f59e0b", "#10b981", "#06b6d4", "#64748b"];
export const KIND_LABEL: Record<CollectionKind, string> = { collection: "集合", folder: "文件夹视图", dropbox: "拖拽释放框", disk: "硬盘面板" };

/** Finder 同款文件图标 / 图片缩略图 */
export function iconUrl(path: string, size: number, m = 0): string {
  return `icon://localhost/${encodeURIComponent(path)}?s=${size}&m=${m}`;
}

export function fmtBytes(n: number): string {
  if (!n) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(Math.floor(Math.log(n) / Math.log(1024)), u.length - 1);
  return `${(n / 1024 ** i).toFixed(i >= 3 ? 2 : i === 0 ? 0 : 1)} ${u[i]}`;
}

export function fmtDate(secs: number): string {
  return secs ? new Date(secs * 1000).toLocaleDateString("zh-CN") : "-";
}

export function errMsg(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
