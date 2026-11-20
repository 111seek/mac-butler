import { useEffect, useState } from "react";
import { api, errMsg, StartupItem } from "../api";

/** 启动项管理：登录项 + LaunchAgents/Daemons。系统级项目只读。 */
export default function Startup() {
  const [items, setItems] = useState<StartupItem[]>([]);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  const load = async () => {
    setBusy(true);
    try { setItems(await api.listStartupItems()); } catch (e) { setMsg(errMsg(e)); }
    setBusy(false);
  };
  useEffect(() => { load(); }, []);

  const toggle = async (it: StartupItem) => {
    try {
      await api.setLaunchAgentEnabled(it.path, !it.enabled);
      setMsg(`${it.enabled ? "已禁用" : "已启用"}：${it.label}`);
      load();
    } catch (e) { setMsg(errMsg(e)); }
  };
  const removeLogin = async (it: StartupItem) => {
    if (!(await api.confirm(`移除登录项「${it.label}」？`, "不会删除应用本身。", "移除"))) return;
    try { await api.removeLoginItem(it.label); setMsg(`已移除登录项：${it.label}`); load(); } catch (e) { setMsg(errMsg(e)); }
  };

  const sources = [...new Set(items.map((i) => i.source))];
  const hasLogin = items.some((i) => i.kind === "login_item");

  return (
    <div className="page">
      <div className="section-head">
        <div>
          <h1>启动项</h1>
          <p className="muted">共 {items.length} 项，已启用 {items.filter((i) => i.enabled).length} 项。禁用不必要的启动项可加快开机速度。</p>
        </div>
        <button className="btn ghost" onClick={load} disabled={busy}>{busy ? "读取中…" : "刷新"}</button>
      </div>
      {msg && <div className="toast">{msg}</div>}
      {!hasLogin && !busy && (
        <div className="toast warn">
          未读取到登录项：首次使用需允许「Mac 管家」控制「System Events」。
          <button className="link" onClick={() => api.openPrivacySettings("automation")}>打开自动化设置</button>
        </div>
      )}
      {sources.map((s) => (
        <div key={s}>
          <h2>{s}</h2>
          <div className="list">
            {items.filter((i) => i.source === s).map((it) => (
              <div key={it.path + it.label} className="group-head static">
                <div className="grow">
                  <div><strong>{it.label}</strong> {!it.editable && <span className="tag">需管理员</span>}</div>
                  <div className="muted small ellipsis" title={it.program}>{it.program || it.path}</div>
                </div>
                {it.kind === "login_item" ? (
                  <button className="btn small ghost" onClick={() => removeLogin(it)}>移除</button>
                ) : (
                  <label className={it.editable ? "switch" : "switch disabled"}>
                    <input type="checkbox" checked={it.enabled} disabled={!it.editable} onChange={() => toggle(it)} />
                    <span />
                  </label>
                )}
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}
