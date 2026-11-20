import { useState } from "react";
import { api, errMsg, fmtBytes, fmtDate, JunkGroup, LargeFile } from "../api";

/** 垃圾清理 + 大文件查找。清理一律移到废纸篓，可恢复。 */
export default function Cleaner() {
  const [tab, setTab] = useState<"junk" | "large">("junk");
  return (
    <div className="page">
      <div className="section-head">
        <h1>垃圾清理</h1>
        <div className="seg">
          <button className={tab === "junk" ? "on" : ""} onClick={() => setTab("junk")}>系统垃圾</button>
          <button className={tab === "large" ? "on" : ""} onClick={() => setTab("large")}>大文件</button>
        </div>
      </div>
      {tab === "junk" ? <Junk /> : <Large />}
    </div>
  );
}

function Junk() {
  const [groups, setGroups] = useState<JunkGroup[] | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [expanded, setExpanded] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const scan = async () => {
    setBusy(true); setMsg("");
    try {
      const g = await api.scanJunk();
      setGroups(g);
      setChecked(new Set(g.filter((x) => x.recommended && x.key !== "trash").flatMap((x) => x.items.map((i) => i.path))));
    } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  const toggleGroup = (g: JunkGroup, on: boolean) => {
    const s = new Set(checked);
    g.items.forEach((i) => (on ? s.add(i.path) : s.delete(i.path)));
    setChecked(s);
  };
  const toggleItem = (p: string) => {
    const s = new Set(checked);
    s.has(p) ? s.delete(p) : s.add(p);
    setChecked(s);
  };

  const selectedSize = groups?.flatMap((g) => g.items).filter((i) => checked.has(i.path)).reduce((a, i) => a + i.size, 0) ?? 0;

  const clean = async () => {
    if (!checked.size) return;
    if (!(await api.confirm(`清理 ${checked.size} 个项目？`, `共 ${fmtBytes(selectedSize)}，将移到废纸篓，可以恢复。`, "清理"))) return;
    setBusy(true);
    try {
      const r = await api.cleanItems([...checked]);
      setMsg(`已清理 ${r.cleaned} 项，释放 ${fmtBytes(r.freed)}（已移到废纸篓）${r.errors.length ? `，${r.errors.length} 项失败（可能被占用）` : ""}`);
      await scan();
    } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  const emptyTrash = async () => {
    if (!(await api.confirm("清倒废纸篓？", "清倒后文件将无法恢复。", "清倒"))) return;
    try { await api.emptyTrash(); setMsg("废纸篓已清空"); scan(); } catch (e) { setMsg(errMsg(e)); }
  };

  if (!groups) {
    return (
      <div className="hero">
        <p className="muted">扫描应用缓存、日志、开发工具缓存、下载中的安装包等可清理项目</p>
        <button className="btn primary big" onClick={scan} disabled={busy}>{busy ? "扫描中…" : "开始扫描"}</button>
        {msg && <div className="toast">{msg}</div>}
      </div>
    );
  }

  const total = groups.reduce((a, g) => a + g.size, 0);
  return (
    <>
      <div className="summary">
        <div>发现 <strong>{fmtBytes(total)}</strong> 可清理，已选 <strong>{fmtBytes(selectedSize)}</strong></div>
        <div className="actions">
          <button className="btn ghost" onClick={scan} disabled={busy}>重新扫描</button>
          <button className="btn primary" onClick={clean} disabled={busy || !checked.size}>清理所选</button>
        </div>
      </div>
      {msg && <div className="toast">{msg}</div>}
      <div className="list">
        {groups.map((g) => {
          const all = g.items.length > 0 && g.items.every((i) => checked.has(i.path));
          return (
            <div key={g.key} className="group">
              <div className="group-head" onClick={() => setExpanded(expanded === g.key ? null : g.key)}>
                {g.key !== "trash" && (
                  <input type="checkbox" checked={all} disabled={!g.items.length}
                    onClick={(e) => e.stopPropagation()} onChange={(e) => toggleGroup(g, e.target.checked)} />
                )}
                <div className="grow">
                  <div><strong>{g.title}</strong> {!g.recommended && g.key !== "trash" && <span className="tag">需确认</span>}</div>
                  <div className="muted small">{g.description}</div>
                </div>
                {g.needs_permission ? (
                  <button className="link" onClick={(e) => { e.stopPropagation(); api.openPrivacySettings("full_disk"); }}>需完全磁盘访问权限</button>
                ) : g.key === "trash" ? (
                  <button className="btn small ghost" onClick={(e) => { e.stopPropagation(); emptyTrash(); }}>{fmtBytes(g.size)} · 清倒</button>
                ) : (
                  <span>{fmtBytes(g.size)}</span>
                )}
              </div>
              {expanded === g.key && g.items.length > 0 && (
                <div className="group-items">
                  {g.items.map((i) => (
                    <label key={i.path} className="item-row">
                      <input type="checkbox" checked={checked.has(i.path)} onChange={() => toggleItem(i.path)} />
                      <span className="grow ellipsis" title={i.path}>{i.name}</span>
                      <span className="muted">{fmtBytes(i.size)}</span>
                    </label>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </>
  );
}

function Large() {
  const [files, setFiles] = useState<LargeFile[] | null>(null);
  const [minMb, setMinMb] = useState(500);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const scan = async () => {
    setBusy(true); setMsg("");
    try { setFiles(await api.scanLargeFiles(minMb, 100)); } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };
  const remove = async (f: LargeFile) => {
    if (!(await api.confirm(`将「${f.name}」移到废纸篓？`, `大小 ${fmtBytes(f.size)}，可以在废纸篓中恢复。`, "移到废纸篓"))) return;
    try { await api.trashUserFile(f.path); setFiles(files!.filter((x) => x.path !== f.path)); setMsg(`已移到废纸篓：${f.name}`); }
    catch (e) { setMsg(errMsg(e)); }
  };

  return (
    <>
      <div className="summary">
        <div>
          在用户目录中查找大于
          <select value={minMb} onChange={(e) => setMinMb(Number(e.target.value))}>
            {[100, 500, 1024, 4096].map((v) => <option key={v} value={v}>{v >= 1024 ? `${v / 1024} GB` : `${v} MB`}</option>)}
          </select>
          的文件（跳过 ~/Library 与隐藏目录）
        </div>
        <button className="btn primary" onClick={scan} disabled={busy}>{busy ? "扫描中…" : "扫描"}</button>
      </div>
      {msg && <div className="toast">{msg}</div>}
      {files && (
        <table className="table">
          <thead><tr><th>文件</th><th>大小</th><th>修改时间</th><th /></tr></thead>
          <tbody>
            {files.map((f) => (
              <tr key={f.path}>
                <td className="ellipsis" title={f.path}>{f.name}<div className="muted small ellipsis">{f.path}</div></td>
                <td>{fmtBytes(f.size)}</td>
                <td className="muted">{fmtDate(f.modified)}</td>
                <td className="nowrap">
                  <button className="btn small ghost" onClick={() => api.revealInFinder(f.path)}>显示</button>
                  <button className="btn small danger" onClick={() => remove(f)}>删除</button>
                </td>
              </tr>
            ))}
            {!files.length && <tr><td colSpan={4} className="muted">没有找到符合条件的大文件</td></tr>}
          </tbody>
        </table>
      )}
    </>
  );
}
