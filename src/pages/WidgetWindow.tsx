import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, AppMem, fmtBytes, iconUrl, Overview, Widget } from "../api";
import Sparkline, { pushHistory } from "../components/Sparkline";

/** 桌面小组件：系统概览 / 内存排行。 */
export default function WidgetWindow({ id }: { id: string }) {
  const [w, setW] = useState<Widget | null>(null);
  const [ov, setOv] = useState<Overview | null>(null);
  const [apps, setApps] = useState<AppMem[]>([]);
  const memHist = useRef(new Map<string, number[]>());
  const [compact, setCompact] = useState(false);

  const load = useCallback(async () => {
    const ww = await api.getWidget(id);
    if (ww) setW(ww);
  }, [id]);

  useEffect(() => {
    load();
    const un = listen("widgets-changed", load);
    return () => { un.then((f) => f()); };
  }, [load]);

  useEffect(() => {
    if (!w) return;
    let alive = true;
    const tick = async () => {
      try {
        if (w.kind === "memory") {
          const a = await api.appMemory(compact ? 5 : 8);
          if (alive) { setApps(a); pushHistory(memHist.current, a); }
        } else {
          const o = await api.systemOverview();
          if (alive) setOv(o);
        }
      } catch { /* 采集失败时保留上一次数据 */ }
    };
    tick();
    const t = setInterval(tick, 2500);
    return () => { alive = false; clearInterval(t); };
  }, [w?.kind, compact]);

  useEffect(() => {
    const onResize = () => setCompact(window.innerWidth < 300 || window.innerHeight < 190);
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  if (!w) return <div className="wg-root" />;

  const memPct = ov ? (ov.mem_used / ov.mem_total) * 100 : 0;
  const disk = ov?.disks.find((d) => d.mount === "/") ?? ov?.disks[0];
  const diskPct = disk ? ((disk.total - disk.available) / disk.total) * 100 : 0;
  const max = apps[0]?.memory || 1;

  return (
    <div
      className={`wg-root ${compact ? "compact" : ""}`}
      onContextMenu={(e) => { e.preventDefault(); api.popupWidgetMenu(id); }}
    >
      <header className="wg-head" onMouseDown={(e) => { if (e.button === 0 && !(e.target as HTMLElement).closest(".wg-btn")) getCurrentWindow().startDragging(); }}>
        <span className={`wg-dot ${w.kind}`} />
        <span className="wg-title">{w.kind === "memory" ? "内存排行" : "系统概览"}</span>
        <span className="wg-sub">{w.kind === "memory" ? `${apps.length} 个应用` : `CPU ${ov ? ov.cpu_usage.toFixed(0) : "–"}%`}</span>
        <button className="wg-btn" title="菜单" onClick={(e) => { e.stopPropagation(); api.popupWidgetMenu(id); }}>⋯</button>
      </header>

      {w.kind === "overview" ? (
        <div className="wg-body">
          <Metric label="CPU" value={ov ? ov.cpu_usage.toFixed(0) : "–"} unit="%" pct={ov?.cpu_usage ?? 0} />
          <Metric label="内存" value={ov ? memPct.toFixed(0) : "–"} unit="%" pct={memPct}
            sub={ov ? `${fmtBytes(ov.mem_used)} / ${fmtBytes(ov.mem_total)}` : undefined} />
          <Metric label="磁盘" value={disk ? diskPct.toFixed(0) : "–"} unit="%" pct={diskPct}
            sub={disk ? `可用 ${fmtBytes(disk.available)}` : undefined} />
          {!compact && ov && <div className="wg-foot">已运行 {uptime(ov.uptime)} · {ov.cpu_count} 核</div>}
        </div>
      ) : (
        <div className="wg-body mem">
          {apps.map((a) => (
            <div key={a.path} className="wg-app" title={`${a.name} · ${a.pids} 个进程`}>
              <img src={iconUrl(a.path, 28)} width={18} height={18} draggable={false} alt="" />
              <div className="grow">
                <div className="wg-app-line">
                  <span className="ellipsis">{a.name}</span>
                  <span className="wg-val">{fmtBytes(a.memory)}</span>
                </div>
                <div className="wg-wave"><Sparkline values={memHist.current.get(a.path) ?? [a.memory]} max={max} color="#ec4899" height={14} /></div>
              </div>
            </div>
          ))}
          {!apps.length && <div className="wg-empty">正在采集…</div>}
        </div>
      )}
    </div>
  );
}

function Metric({ label, value, unit, pct, sub }: { label: string; value: string; unit: string; pct: number; sub?: string }) {
  const tone = pct > 90 ? "bad" : pct > 75 ? "mid" : "ok";
  return (
    <div className="wg-metric">
      <div className="wg-metric-line">
        <span className="wg-label">{label}</span>
        <span className="wg-num">{value}<small>{unit}</small></span>
      </div>
      <div className="wg-bar"><div className={tone} style={{ width: `${Math.min(pct, 100)}%` }} /></div>
      {sub && <div className="wg-hint">{sub}</div>}
    </div>
  );
}

function uptime(secs: number) {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return d > 0 ? `${d} 天 ${h} 小时` : h > 0 ? `${h} 小时 ${m} 分` : `${m} 分钟`;
}
