import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { api, CItem, Collection, errMsg, fmtBytes, fmtDate, iconUrl, Volume } from "../api";

/** 悬浮在 Mac 桌面上的单个集合窗口。 */
export default function CollectionWindow({ id }: { id: string }) {
  const [col, setCol] = useState<Collection | null>(null);
  const [items, setItems] = useState<CItem[]>([]);
  const [vols, setVols] = useState<Volume[]>([]);
  const [renaming, setRenaming] = useState(location.hash.includes("rename=1"));
  const [dropHover, setDropHover] = useState(false);
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [toast, setToast] = useState("");
  /** 背景透明度 0–100（拖动滑块时实时更新，不等整份数据刷新） */
  const [transp, setTransp] = useState(0);
  const kindRef = useRef<string>("");

  const load = useCallback(async () => {
    const c = await api.getCollection(id);
    if (!c) return;
    setCol(c);
    setTransp(c.transparency ?? 0);
    kindRef.current = c.kind;
    if (c.kind === "disk") setVols(await api.listVolumes());
    else setItems(await api.collectionItems(id));
  }, [id]);

  useEffect(() => {
    load();
    const uns = [
      listen("collections-changed", load),
      listen("desktop-changed", load),
      listen<string>("start-rename", (e) => e.payload === id && setRenaming(true)),
      listen<[string, number]>("collection-transparency", (e) => e.payload[0] === id && setTransp(e.payload[1])),
      getCurrentWebviewWindow().onDragDropEvent(async (e) => {
        const p = e.payload;
        if (p.type === "enter" || p.type === "over") setDropHover(true);
        else if (p.type === "leave") setDropHover(false);
        else if (p.type === "drop") {
          setDropHover(false);
          try {
            const n = await api.addToCollection(id, p.paths);
            if (n) flash(`已移入本集合（${n} 项）。桌面不再保留，右键文件 →「移回桌面」可还原。`);
          } catch (err) { flash(errMsg(err)); }
        }
      }),
    ];
    const t = setInterval(() => kindRef.current === "disk" || kindRef.current === "folder" ? load() : null, 10000);
    return () => { uns.forEach((u) => u.then((f) => f())); clearInterval(t); };
  }, [id, load]);

  const flash = (m: string) => { setToast(m); setTimeout(() => setToast(""), 3500); };

  if (!col) return <div className="cw-root" />;

  const rename = async (name: string) => {
    setRenaming(false);
    if (name.trim() && name !== col.name) await api.updateCollection({ id, name });
  };

  const onHeadMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0 || col.locked || (e.target as HTMLElement).closest(".cw-nodrag")) return;
    getCurrentWindow().startDragging();
  };

  const onItemMouseDown = (e: React.MouseEvent, it: CItem) => {
    if (e.button !== 0) return;
    const sx = e.clientX, sy = e.clientY;
    const paths = sel.has(it.path) ? [...sel] : [it.path];
    // 集合里的实体文件用「移动」拖出：拖到桌面/Finder 后集合中不再保留，避免出现两份；快捷引用仍按拷贝处理
    const owned = (col?.kind === "collection" || col?.kind === "dropbox") && paths.every((p) => !col?.refs.includes(p));
    const iconP = api.iconFile(it.path).catch(() => "");
    const move = async (ev: MouseEvent) => {
      if (Math.abs(ev.clientX - sx) + Math.abs(ev.clientY - sy) < 6) return;
      cleanup();
      const icon = await iconP;
      try {
        await startDrag({ item: paths, icon, mode: owned ? "move" : "copy" }, (r) => {
          if (owned && r.result === "Dropped") api.dragFinished(paths).catch(() => {});
        });
      } catch (err) { console.error(err); }
    };
    const cleanup = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", cleanup); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", cleanup);
  };

  const onItemClick = (e: React.MouseEvent, it: CItem) => {
    e.stopPropagation();
    const s = new Set(e.metaKey ? sel : []);
    e.metaKey && sel.has(it.path) ? s.delete(it.path) : s.add(it.path);
    setSel(s);
  };

  const onItemContext = (e: React.MouseEvent, it: CItem) => {
    e.preventDefault();
    e.stopPropagation();
    if (!sel.has(it.path)) setSel(new Set([it.path]));
    api.popupMenu(id, it.path);
  };

  const mode = col.view || "icon";
  const pos = col.title_pos || "top";
  const side = pos === "left" || pos === "right";
  // 列表 / 迷你样式使用固定的小图标
  const size = mode === "list" ? 20 : mode === "mini" ? 16 : col.icon_size || 48;
  // 标题栏尺寸跟随字号：与 Rust 端 header_h() 的算法保持一致
  const headBase = mode === "mini" ? 24 : 34;
  const ts = col.title_size || (mode === "mini" ? 11 : 13);
  const head = col.title_size ? Math.max(headBase, ts + (mode === "mini" ? 12 : 16)) : headBase;
  const collapseIcon = side ? (col.collapsed ? (pos === "left" ? "▸" : "◂") : (pos === "left" ? "◂" : "▸")) : col.collapsed ? (pos === "bottom" ? "▴" : "▸") : (pos === "bottom" ? "▴" : "▾");
  const empty = {
    collection: col.catch_all ? "未被其他集合认领的桌面文件会移到这里" : "把桌面文件拖到这里（会从桌面移入集合），或右键设置自动归类规则",
    folder: col.folder ? "这个文件夹是空的，或没有访问权限" : "右键 → 选择文件夹…",
    dropbox: "拖入文件暂存（桌面文件会移入这里），需要时再拖出使用",
    disk: "",
  }[col.kind];

  return (
    <div
      className={`cw-root tp-${pos} v-${mode} ${col.locked ? "locked" : ""} ${dropHover ? "drop" : ""} ${col.collapsed ? "collapsed" : ""}`}
      style={{
        ["--c" as string]: col.color,
        ["--a" as string]: String(1 - transp / 100),
        ["--ts" as string]: `${ts}px`,
        ["--hh" as string]: `${head}px`,
        ["--hw" as string]: `${head}px`,
      }}
      onContextMenu={(e) => { e.preventDefault(); api.popupMenu(id); }}
      onClick={() => setSel(new Set())}
    >
      <header
        className="cw-head"
        onMouseDown={onHeadMouseDown}
        title={col.locked ? "已锁定位置，右键 →「锁定位置」可解锁" : col.folder ?? ""}
      >
        <span className="cw-dot" />
        {renaming ? (
          <input
            className="cw-rename cw-nodrag"
            autoFocus
            defaultValue={col.name}
            onFocus={(e) => e.target.select()}
            onBlur={(e) => rename(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              if (e.key === "Escape") setRenaming(false);
            }}
          />
        ) : (
          <span className="cw-title cw-nodrag" onDoubleClick={() => !col.locked && setRenaming(true)}>{col.name}</span>
        )}
        <span className="cw-count">{col.kind === "disk" ? vols.length : items.length}</span>
        <button className="cw-btn cw-nodrag" title={col.collapsed ? "展开" : "折叠"} onClick={(e) => { e.stopPropagation(); api.setCollapsed(id, !col.collapsed); }}>
          {collapseIcon}
        </button>
        <button className="cw-btn cw-nodrag" title="菜单" onClick={(e) => { e.stopPropagation(); api.popupMenu(id); }}>⋯</button>
      </header>

      {!col.collapsed && (
        <div className="cw-body">
          {col.kind === "disk" ? (
            <div className="cw-disks">
              {vols.map((v) => {
                const pct = v.total ? ((v.total - v.available) / v.total) * 100 : 0;
                return (
                  <div key={v.mount} className="cw-disk" onDoubleClick={() => api.openPath(v.mount)} title="双击打开">
                    <img src={iconUrl(v.mount, 40)} width={36} height={36} draggable={false} />
                    <div className="grow">
                      <div className="cw-disk-name">{v.name}</div>
                      <div className="cw-bar"><div style={{ width: `${pct}%`, background: pct > 90 ? "#ff453a" : pct > 75 ? "#ff9f0a" : "var(--c)" }} /></div>
                      <div className="cw-sub">可用 {fmtBytes(v.available)} / 共 {fmtBytes(v.total)}</div>
                    </div>
                  </div>
                );
              })}
            </div>
          ) : items.length ? (
            <div
              className={mode === "icon" ? "cw-grid" : mode === "list" ? "cw-list" : "cw-mini"}
              style={mode === "icon" ? { gridTemplateColumns: `repeat(auto-fill, minmax(${size + 30}px, 1fr))` } : undefined}
            >
              {items.map((it) => (
                <div
                  key={it.path}
                  className={`cw-item ${sel.has(it.path) ? "sel" : ""}`}
                  title={`${it.name}${it.size ? `\n${fmtBytes(it.size)}` : ""}${it.is_ref ? "\n（快捷引用：" + it.path + "）" : ""}`}
                  onMouseDown={(e) => onItemMouseDown(e, it)}
                  onClick={(e) => onItemClick(e, it)}
                  onDoubleClick={() => api.openPath(it.path)}
                  onContextMenu={(e) => onItemContext(e, it)}
                >
                  <div className="cw-icon" style={{ width: size, height: size }}>
                    <img src={iconUrl(it.path, size, it.modified)} width={size} height={size} draggable={false} loading="lazy" />
                    {it.is_ref && col.kind === "collection" && <span className="cw-alias">↗</span>}
                  </div>
                  <div className="cw-name">{it.name}</div>
                  {mode === "list" && (
                    <>
                      <div className="cw-meta">{it.is_dir ? "文件夹" : fmtBytes(it.size)}</div>
                      <div className="cw-meta date">{fmtDate(it.modified)}</div>
                    </>
                  )}
                </div>
              ))}
            </div>
          ) : (
            <div className="cw-empty">{empty}</div>
          )}
        </div>
      )}
      {toast && <div className="cw-toast">{toast}</div>}
    </div>
  );
}
