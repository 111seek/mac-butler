import { useState } from "react";
import { api, AppEntry, BrowserInfo, DupGroup, errMsg, fmtBytes, fmtDate, iconUrl } from "../api";

/** 卸载器 / 重复文件 / 浏览器缓存。所有删除都进废纸篓，可恢复。 */
export default function Tools() {
  const [tab, setTab] = useState<"apps" | "dups" | "browsers">("apps");
  return (
    <div className="page">
      <div className="section-head">
        <h1>卸载与查重</h1>
        <div className="seg">
          <button className={tab === "apps" ? "on" : ""} onClick={() => setTab("apps")}>应用卸载</button>
          <button className={tab === "dups" ? "on" : ""} onClick={() => setTab("dups")}>重复文件</button>
          <button className={tab === "browsers" ? "on" : ""} onClick={() => setTab("browsers")}>浏览器缓存</button>
        </div>
      </div>
      {tab === "apps" ? <Apps /> : tab === "dups" ? <Dups /> : <Browsers />}
    </div>
  );
}

function Apps() {
  const [apps, setApps] = useState<AppEntry[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [q, setQ] = useState("");

  const scan = async () => {
    setBusy(true); setMsg("");
    try { setApps(await api.listApps()); } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  const uninstall = async (app: AppEntry) => {
    if (app.running && !(await api.confirm(`「${app.name}」正在运行`, "建议先退出应用再卸载，否则可能残留正在写入的文件。仍要继续吗？", "继续"))) return;
    setBusy(true);
    try {
      const residues = await api.appResidues(app.bundle_id, app.name);
      const total = residues.reduce((a, r) => a + r.size, 0);
      const detail = residues.slice(0, 8).map((r) => `· ${r.kind}：${r.path.replace(/^.*\/Library\//, "~/Library/")}`).join("\n");
      const more = residues.length > 8 ? `\n… 另有 ${residues.length - 8} 项` : "";
      const ok = await api.confirm(
        `卸载「${app.name}」？`,
        `应用本体 ${fmtBytes(app.size)}，可清理的关联数据 ${residues.length} 项（${fmtBytes(total)}）。\n${detail}${more}\n\n全部移到废纸篓，可从废纸篓恢复。`,
        "卸载",
      );
      if (!ok) { setBusy(false); return; }
      const [n, freed] = await api.uninstallApp(app.path, residues.map((r) => r.path));
      setMsg(`已卸载「${app.name}」：${n} 项，释放 ${fmtBytes(freed)}（已移到废纸篓）`);
      await scan();
    } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  if (!apps) {
    return (
      <div className="hero">
        <p className="muted">扫描「应用程序」文件夹，卸载时可一并清理缓存、偏好设置、沙盒容器等残留数据</p>
        <button className="btn primary big" onClick={scan} disabled={busy}>{busy ? "扫描中…" : "开始扫描"}</button>
        {msg && <div className="toast">{msg}</div>}
      </div>
    );
  }

  const list = apps.filter((a) => !q || a.name.toLowerCase().includes(q.toLowerCase()) || a.bundle_id.toLowerCase().includes(q.toLowerCase()));
  const total = apps.reduce((a, x) => a + x.size, 0);
  return (
    <>
      <div className="summary">
        <div>
          {apps.length} 个应用，共 {fmtBytes(total)}
          <input className="search" placeholder="搜索名称或 Bundle ID" value={q} onChange={(e) => setQ(e.target.value)} />
        </div>
        <button className="btn ghost" onClick={scan} disabled={busy}>重新扫描</button>
      </div>
      {msg && <div className="toast">{msg}</div>}
      <table className="table">
        <thead><tr><th>应用</th><th>版本</th><th>大小</th><th /></tr></thead>
        <tbody>
          {list.map((a) => (
            <tr key={a.path}>
              <td>
                <div className="app-cell">
                  <img src={iconUrl(a.path, 32)} width={28} height={28} draggable={false} alt="" />
                  <div className="grow">
                    <div className="ellipsis">{a.name} {a.running && <span className="tag">运行中</span>}</div>
                    <div className="muted small ellipsis">{a.bundle_id || "无 Bundle ID"}</div>
                  </div>
                </div>
              </td>
              <td className="muted">{a.version || "—"}</td>
              <td className="nowrap">{fmtBytes(a.size)}</td>
              <td className="nowrap">
                <button className="btn small ghost" onClick={() => api.revealInFinder(a.path)}>显示</button>
                <button className="btn small danger" onClick={() => uninstall(a)} disabled={busy}>卸载</button>
              </td>
            </tr>
          ))}
          {!list.length && <tr><td colSpan={4} className="muted">没有匹配的应用</td></tr>}
        </tbody>
      </table>
    </>
  );
}

const ROOTS: [string, string][] = [
  ["Desktop", "桌面"],
  ["Downloads", "下载"],
  ["Documents", "文稿"],
  ["Pictures", "图片"],
  ["Movies", "影片"],
];

function Dups() {
  const [root, setRoot] = useState("Downloads");
  const [minKb, setMinKb] = useState(1024);
  const [groups, setGroups] = useState<DupGroup[] | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const scan = async () => {
    setBusy(true); setMsg("");
    try {
      const g = await api.findDuplicates(root, minKb);
      setGroups(g);
      setChecked(new Set());
    } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  const pick = async () => {
    const f = await api.pickFolder();
    if (f) { setRoot(f); setGroups(null); }
  };

  const toggle = (p: string) => {
    const s = new Set(checked);
    s.has(p) ? s.delete(p) : s.add(p);
    setChecked(s);
  };

  const remove = async () => {
    if (!checked.size) return;
    const freed = groups?.flatMap((g) => g.files).filter((f) => checked.has(f.path)).reduce((a, f) => a + f.size, 0) ?? 0;
    if (!(await api.confirm(`删除 ${checked.size} 个重复文件？`, `共 ${fmtBytes(freed)}，将移到废纸篓，可以恢复。每组都会保留一份。`, "删除"))) return;
    setBusy(true);
    try {
      const [n, bytes] = await api.trashDuplicates([...checked]);
      setMsg(`已删除 ${n} 个文件，释放 ${fmtBytes(bytes)}`);
      await scan();
    } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  const wasted = groups?.reduce((a, g) => a + g.wasted, 0) ?? 0;

  return (
    <>
      <div className="summary">
        <div className="dup-controls">
          <select className="mini-select" value={ROOTS.some(([k]) => k === root) ? root : ""} onChange={(e) => { if (e.target.value) { setRoot(e.target.value); setGroups(null); } }}>
            {ROOTS.map(([k, label]) => <option key={k} value={k}>{label}</option>)}
            {!ROOTS.some(([k]) => k === root) && <option value="">自定义文件夹</option>}
          </select>
          <button className="btn small ghost" onClick={pick}>选择文件夹…</button>
          <span className="muted small">最小</span>
          <select className="mini-select" value={minKb} onChange={(e) => setMinKb(Number(e.target.value))}>
            {[[100, "100 KB"], [1024, "1 MB"], [10 * 1024, "10 MB"], [100 * 1024, "100 MB"]].map(([v, l]) => <option key={v} value={v}>{l}</option>)}
          </select>
          <button className="btn primary small" onClick={scan} disabled={busy}>{busy ? "扫描中…" : "扫描"}</button>
        </div>
        {groups && (
          <div className="actions">
            <span className="muted">可释放 {fmtBytes(wasted)}</span>
            <button className="btn danger" onClick={remove} disabled={busy || !checked.size}>删除所选（{checked.size}）</button>
          </div>
        )}
      </div>
      {msg && <div className="toast">{msg}</div>}
      {groups && (
        <div className="list">
          {groups.map((g, gi) => (
            <div key={gi} className="group">
              <div className="group-head static">
                <div className="grow">
                  <strong>{g.files[0].name}</strong> <span className="muted small">×{g.files.length}</span>
                  <div className="muted small">每个 {fmtBytes(g.size)}，可释放 {fmtBytes(g.wasted)}</div>
                </div>
                <button className="link" onClick={() => setChecked(new Set([...checked, ...g.files.slice(1).map((f) => f.path)]))}>选中多余</button>
              </div>
              <div className="group-items">
                {g.files.map((f, i) => (
                  <label key={f.path} className="item-row">
                    <input type="checkbox" checked={checked.has(f.path)} onChange={() => toggle(f.path)} />
                    <span className="grow ellipsis" title={f.path}>{f.path}{i === 0 && <span className="tag plain">保留</span>}</span>
                    <span className="muted small nowrap">{fmtDate(f.modified)}</span>
                  </label>
                ))}
              </div>
            </div>
          ))}
          {!groups.length && <div className="group-head static muted">没有找到重复文件</div>}
        </div>
      )}
    </>
  );
}

function Browsers() {
  const [list, setList] = useState<BrowserInfo[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const scan = async () => {
    setBusy(true); setMsg("");
    try { setList(await api.scanBrowsers()); } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  const clean = async (b: BrowserInfo) => {
    if (!(await api.confirm(`清理 ${b.name} 缓存？`, `约 ${fmtBytes(b.size)}，会移到废纸篓。登录状态和书签不受影响。`, "清理"))) return;
    setBusy(true);
    try {
      const freed = await api.cleanBrowserCache(b.key);
      setMsg(`${b.name}：已清理 ${fmtBytes(freed)}`);
      await scan();
    } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };

  if (!list) {
    return (
      <div className="hero">
        <p className="muted">扫描 Chrome、Edge、Safari、Firefox、Arc 的缓存目录；浏览器运行中时会提示先退出</p>
        <button className="btn primary big" onClick={scan} disabled={busy}>{busy ? "扫描中…" : "开始扫描"}</button>
        {msg && <div className="toast">{msg}</div>}
      </div>
    );
  }

  const total = list.reduce((a, b) => a + b.size, 0);
  return (
    <>
      <div className="summary">
        <div>发现 <strong>{fmtBytes(total)}</strong> 浏览器缓存</div>
        <button className="btn ghost" onClick={scan} disabled={busy}>重新扫描</button>
      </div>
      {msg && <div className="toast">{msg}</div>}
      <div className="list">
        {list.map((b) => (
          <div key={b.key} className="group-head static">
            <div className="grow">
              <div><strong>{b.name}</strong> {b.running && <span className="tag">运行中</span>}</div>
              <div className="muted small ellipsis" title={b.paths.join("\n")}>{b.paths.length} 个缓存目录</div>
            </div>
            <span>{fmtBytes(b.size)}</span>
            <button
              className="btn small ghost"
              onClick={() => clean(b)}
              disabled={busy || b.running}
              title={b.running ? "请先退出浏览器" : ""}
            >清理</button>
          </div>
        ))}
        {!list.length && <div className="group-head static muted">没有发现可清理的浏览器缓存</div>}
      </div>
    </>
  );
}
