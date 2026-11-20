import { useEffect, useState } from "react";
import { api, PermissionStatus } from "./api";
import Dashboard from "./pages/Dashboard";
import Collections from "./pages/Collections";
import { listen } from "@tauri-apps/api/event";
import Cleaner from "./pages/Cleaner";
import Startup from "./pages/Startup";
import Tools from "./pages/Tools";

const PAGES = [
  { key: "dashboard", label: "系统概览", icon: "◎" },
  { key: "desktop", label: "桌面集合", icon: "▦" },
  { key: "cleaner", label: "垃圾清理", icon: "✦" },
  { key: "startup", label: "启动项", icon: "⏻" },
  { key: "tools", label: "卸载与查重", icon: "⌫" },
] as const;
type PageKey = (typeof PAGES)[number]["key"];

export default function App() {
  const [page, setPage] = useState<PageKey>("dashboard");
  const [perm, setPerm] = useState<PermissionStatus | null>(null);
  const [focusId, setFocusId] = useState<string | null>(null);

  // 集合右键「自动归类规则…」会跳到这里并定位到对应集合
  useEffect(() => {
    const un = listen<string>("focus-collection", (e) => { setPage("desktop"); setFocusId(e.payload); });
    return () => { un.then((f) => f()); };
  }, []);

  useEffect(() => {
    api.checkPermissions().then(setPerm).catch(() => setPerm(null));
  }, [page]);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="drag-region" data-tauri-drag-region />
        <div className="brand">Mac 管家</div>
        <nav>
          {PAGES.map((p) => (
            <button key={p.key} className={page === p.key ? "nav active" : "nav"} onClick={() => setPage(p.key)}>
              <span className="nav-icon">{p.icon}</span>
              {p.label}
            </button>
          ))}
        </nav>
        <div className="perm-box">
          <div>权限状态</div>
          <PermRow ok={perm?.desktop} label="桌面访问" onFix={() => api.openPrivacySettings("files")} />
          <PermRow ok={perm?.full_disk} label="完全磁盘访问" onFix={() => api.openPrivacySettings("full_disk")} />
        </div>
      </aside>
      <main className="content">
        <div className="drag-region" data-tauri-drag-region />
        {page === "dashboard" && <Dashboard />}
        {page === "desktop" && <Collections focusId={focusId} />}
        {page === "cleaner" && <Cleaner />}
        {page === "startup" && <Startup />}
        {page === "tools" && <Tools />}
      </main>
    </div>
  );
}

function PermRow({ ok, label, onFix }: { ok?: boolean; label: string; onFix: () => void }) {
  return (
    <div className="perm-row">
      <span className={ok ? "dot ok" : "dot warn"} />
      <span>{label}</span>
      {!ok && <button className="link" onClick={onFix}>去授权</button>}
    </div>
  );
}
