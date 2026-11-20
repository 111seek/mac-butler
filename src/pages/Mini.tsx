import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, AppMem, fmtBytes, iconUrl, Overview } from "../api";

/** 菜单栏图标点击后弹出的迷你面板。 */
export default function Mini() {
  const [ov, setOv] = useState<Overview | null>(null);
  const [apps, setApps] = useState<AppMem[]>([]);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const [o, a] = await Promise.all([api.systemOverview(), api.appMemory(4)]);
        if (alive) { setOv(o); setApps(a); }
      } catch { /* 忽略单次采集失败 */ }
    };
    tick();
    const t = setInterval(tick, 2000);
    return () => { alive = false; clearInterval(t); };
  }, []);

  const memPct = ov ? (ov.mem_used / ov.mem_total) * 100 : 0;
  const disk = ov?.disks.find((d) => d.mount === "/") ?? ov?.disks[0];
  const diskPct = disk ? ((disk.total - disk.available) / disk.total) * 100 : 0;
  const hide = () => getCurrentWindow().hide();

  return (
    <div className="mini-root">
      <div className="mini-head">
        <strong>Mac 管家</strong>
        <span className="grow" />
        <button className="mini-btn" title="打开主窗口" onClick={() => { api.openMain(); hide(); }}>打开</button>
        <button className="mini-btn" title="隐藏" onClick={hide}>×</button>
      </div>
      <div className="mini-metrics">
        <Cell label="CPU" pct={ov?.cpu_usage ?? 0} text={ov ? `${ov.cpu_usage.toFixed(0)}%` : "–"} />
        <Cell label="内存" pct={memPct} text={ov ? `${memPct.toFixed(0)}%` : "–"} />
        <Cell label="磁盘" pct={diskPct} text={disk ? `${diskPct.toFixed(0)}%` : "–"} />
      </div>
      <div className="mini-list">
        {apps.map((a) => (
          <div key={a.path} className="mini-app" title={`${a.name} · ${a.pids} 个进程`}>
            <img src={iconUrl(a.path, 24)} width={16} height={16} draggable={false} alt="" />
            <span className="grow ellipsis">{a.name}</span>
            <span className="muted small">{fmtBytes(a.memory)}</span>
          </div>
        ))}
        {!apps.length && <div className="muted small">正在采集…</div>}
      </div>
    </div>
  );
}

function Cell({ label, pct, text }: { label: string; pct: number; text: string }) {
  const tone = pct > 90 ? "bad" : pct > 75 ? "mid" : "ok";
  return (
    <div className="mini-cell">
      <div className="mini-cell-line"><span className="muted small">{label}</span><strong>{text}</strong></div>
      <div className="wg-bar"><div className={tone} style={{ width: `${Math.min(pct, 100)}%` }} /></div>
    </div>
  );
}
