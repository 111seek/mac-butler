import { useEffect, useRef, useState } from "react";
import { api, AppMem, errMsg, fmtBytes, iconUrl, Overview, ProcInfo } from "../api";
import Sparkline, { pushHistory } from "../components/Sparkline";

export default function Dashboard() {
  const [ov, setOv] = useState<Overview | null>(null);
  const [procs, setProcs] = useState<ProcInfo[]>([]);
  const [appMem, setAppMem] = useState<AppMem[]>([]);
  const memHist = useRef(new Map<string, number[]>());
  const [sortBy, setSortBy] = useState<"cpu" | "memory">("memory");
  const [msg, setMsg] = useState("");

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const [o, p, m] = await Promise.all([api.systemOverview(), api.topProcesses(sortBy), api.appMemory(6)]);
        if (alive) {
          setOv(o); setProcs(p); setAppMem(m);
          pushHistory(memHist.current, m);
        }
      } catch (e) { if (alive) setMsg(errMsg(e)); }
    };
    tick();
    const t = setInterval(tick, 2000);
    return () => { alive = false; clearInterval(t); };
  }, [sortBy]);

  const kill = async (p: ProcInfo) => {
    if (!(await api.confirm(`结束进程「${p.name}」？`, `PID ${p.pid}，未保存的数据可能丢失。`, "结束进程"))) return;
    try { await api.killProcess(p.pid); setMsg(`已结束 ${p.name}`); } catch (e) { setMsg(errMsg(e)); }
  };

  if (!ov) return <div className="page"><h1>系统概览</h1><p className="muted">正在采集…</p></div>;
  const memPct = (ov.mem_used / ov.mem_total) * 100;
  const disk = ov.disks.find((d) => d.mount === "/") ?? ov.disks[0];
  const diskPct = disk ? ((disk.total - disk.available) / disk.total) * 100 : 0;
  const score = Math.round(100 - (ov.cpu_usage * 0.3 + memPct * 0.3 + diskPct * 0.4) / 2);

  return (
    <div className="page">
      <div className="section-head">
        <h1>系统概览</h1>
        <button className="btn small ghost" onClick={async () => { try { await api.createWidget("overview"); setMsg("已把系统概览放到桌面"); } catch (e) { setMsg(errMsg(e)); } }}>放到桌面</button>
      </div>
      <p className="muted">{ov.host_name} · {ov.os_version} · {ov.cpu_brand} · 已运行 {fmtUptime(ov.uptime)}</p>

      <div className="cards">
        <Gauge title="健康分" value={score} unit="" pct={score} />
        <Gauge title="CPU" value={ov.cpu_usage.toFixed(0)} unit="%" pct={ov.cpu_usage} sub={`${ov.cpu_count} 核`} />
        <Gauge title="内存" value={memPct.toFixed(0)} unit="%" pct={memPct} sub={`${fmtBytes(ov.mem_used)} / ${fmtBytes(ov.mem_total)}`} />
        {disk && <Gauge title="磁盘" value={diskPct.toFixed(0)} unit="%" pct={diskPct} sub={`可用 ${fmtBytes(disk.available)}`} />}
      </div>

      <div className="section-head">
        <h2>应用内存排行</h2>
        <button className="btn small ghost" onClick={async () => { try { await api.createWidget("memory"); setMsg("已把内存排行放到桌面"); } catch (e) { setMsg(errMsg(e)); } }}>放到桌面</button>
      </div>
      <div className="appmem">
        {appMem.map((a) => {
          return (
            <div key={a.path} className="appmem-row" title={`${a.path} · ${a.pids} 个进程`}>
              <img src={iconUrl(a.path, 32)} width={22} height={22} draggable={false} alt="" />
              <span className="appmem-name ellipsis">{a.name}</span>
              <div className="appmem-wave">
                <Sparkline values={memHist.current.get(a.path) ?? [a.memory]} max={appMem[0]?.memory} color="#3b6cff" height={24} />
              </div>
              <span className="muted small nowrap">{fmtBytes(a.memory)}</span>
            </div>
          );
        })}
        {!appMem.length && <div className="muted small">正在采集…</div>}
      </div>

      <div className="section-head">
        <h2>资源占用 Top 15</h2>
        <div className="seg">
          <button className={sortBy === "memory" ? "on" : ""} onClick={() => setSortBy("memory")}>按内存</button>
          <button className={sortBy === "cpu" ? "on" : ""} onClick={() => setSortBy("cpu")}>按 CPU</button>
        </div>
      </div>
      {msg && <div className="toast">{msg}</div>}
      <table className="table">
        <thead><tr><th>进程</th><th>PID</th><th>CPU</th><th>内存</th><th /></tr></thead>
        <tbody>
          {procs.map((p) => (
            <tr key={p.pid}>
              <td>{p.name}</td><td className="muted">{p.pid}</td>
              <td>{p.cpu.toFixed(1)}%</td><td>{fmtBytes(p.memory)}</td>
              <td><button className="btn small ghost" onClick={() => kill(p)}>结束</button></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Gauge({ title, value, unit, pct, sub }: { title: string; value: string | number; unit: string; pct: number; sub?: string }) {
  const level = title === "健康分" ? (pct > 80 ? "ok" : pct > 60 ? "mid" : "bad") : pct < 60 ? "ok" : pct < 85 ? "mid" : "bad";
  return (
    <div className="card">
      <div className="card-title">{title}</div>
      <div className="card-value">{value}<small>{unit}</small></div>
      <div className="bar"><div className={`bar-fill ${level}`} style={{ width: `${Math.min(pct, 100)}%` }} /></div>
      {sub && <div className="muted small">{sub}</div>}
    </div>
  );
}

function fmtUptime(s: number) {
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  return d ? `${d} 天 ${h} 小时` : h ? `${h} 小时 ${m} 分` : `${m} 分钟`;
}
