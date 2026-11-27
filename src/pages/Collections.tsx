import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CItem, Collection, CollectionKind, COLORS, errMsg, fmtBytes, fmtDate, KIND_LABEL, MoveOp, Prefs, SweepItem, TITLE_POS, TITLE_SIZES, VIEW_MODE, Widget, WidgetKind, WidgetSize } from "../api";

/** 主窗口里的「桌面集合」管理页：集合本身显示在 Mac 桌面上，这里负责自定义。 */
export default function Collections({ focusId }: { focusId: string | null }) {
  const [cols, setCols] = useState<Collection[]>([]);
  const [prefs, setPrefs] = useState<Prefs | null>(null);
  const [unassigned, setUnassigned] = useState<CItem[]>([]);
  const [moves, setMoves] = useState<MoveOp[]>([]);
  const [cache, setCache] = useState(0);
  const [widgets, setWidgets] = useState<Widget[]>([]);
  const [preview, setPreview] = useState<SweepItem[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [showMoves, setShowMoves] = useState(() => localStorage.getItem("mb.showMoves") === "1");
  const [showCols, setShowCols] = useState(() => localStorage.getItem("mb.showCols") !== "0");
  const foldMoves = () => setShowMoves((v) => { localStorage.setItem("mb.showMoves", v ? "0" : "1"); return !v; });
  const foldCols = () => setShowCols((v) => { localStorage.setItem("mb.showCols", v ? "0" : "1"); return !v; });
  const [msg, setMsg] = useState("");

  const load = async () => {
    try {
      const [c, p, u, m, sz, wg] = await Promise.all([
        api.listCollections(), api.getPrefs(), api.unassignedItems(), api.recentMoves(), api.cacheSize(), api.listWidgets(),
      ]);
      setCols(c); setPrefs(p); setUnassigned(u); setMoves(m); setCache(sz); setWidgets(wg);
    } catch (e) { setMsg(errMsg(e)); }
  };

  useEffect(() => {
    load();
    const uns = [listen("collections-changed", load), listen("desktop-changed", load)];
    return () => uns.forEach((u) => u.then((f) => f()));
  }, []);

  useEffect(() => {
    if (!focusId) return;
    const el = document.getElementById(`col-card-${focusId}`);
    el?.scrollIntoView({ behavior: "smooth", block: "center" });
    el?.classList.add("flash");
    setTimeout(() => el?.classList.remove("flash"), 1500);
  }, [focusId, cols.length]);

  const create = async (kind: CollectionKind) => {
    try {
      let folder: string | undefined;
      if (kind === "folder") {
        const f = await api.pickFolder();
        if (!f) return;
        folder = f;
      }
      const c = await api.createCollection(kind, folder);
      setMsg(`已在桌面上创建「${c.name}」，可直接拖动到想要的位置`);
    } catch (e) { setMsg(errMsg(e)); }
  };

  const patch = (p: Partial<Collection> & { id: string }) => api.updateCollection(p).catch((e) => setMsg(errMsg(e)));

  const toggleFinder = async () => {
    if (!prefs) return;
    const show = !prefs.finder_icons;
    if (!show && !(await api.confirm("隐藏系统桌面图标？", "隐藏后桌面只显示集合；未放入集合的文件仍在「桌面」文件夹中。Finder 会重启一次，随时可以恢复。", "隐藏"))) return;
    try { await api.setFinderIcons(show); load(); } catch (e) { setMsg(errMsg(e)); }
  };

  const remove = async (c: Collection) => {
    if (!(await api.confirm(`删除集合「${c.name}」？`, "只删除这个集合，其中的文件不会被删除，会全部移回桌面。", "删除"))) return;
    try { await api.deleteCollection(c.id); } catch (e) { setMsg(errMsg(e)); }
  };

  return (
    <div className="page">
      <div className="section-head">
        <div>
          <h1>桌面集合</h1>
          <p className="muted">集合直接悬浮在 Mac 桌面上（桌面图标之上、所有窗口之下）。在集合上<b>右键</b>可新建、重命名、换颜色、调整透明度和图标大小；拖动标题栏移动，拖动边缘调整大小。</p>
        </div>
      </div>

      <div className="create-row">
        {([
          ["collection", "▦", "集合", "桌面文档集"],
          ["disk", "⛁", "硬盘面板", "电脑上的硬盘"],
          ["folder", "▤", "文件夹合集", "显示所选文件夹里的文件"],
          ["dropbox", "⇩", "拖拽释放框", "临时文件容器"],
        ] as const).map(([k, icon, title, sub]) => (
          <button key={k} className="create-card" onClick={() => create(k)}>
            <span className="create-icon">{icon}</span>
            <span><strong>{title}</strong><br /><span className="muted small">{sub}</span></span>
          </button>
        ))}
      </div>

      {prefs && (
        <div className="list prefs">
          <PrefRow title="隐藏系统桌面图标" desc="只显示集合，桌面更整洁（会重启一次 Finder，可随时恢复）" on={!prefs.finder_icons} onChange={toggleFinder} />
          <PrefRow title="将集合置于前方" desc="临时把集合浮在所有窗口之上，方便拖放文件" on={prefs.on_top} onChange={() => api.setPrefs({ onTop: !prefs.on_top })} />
          <PrefRow title="隐藏集合" desc="暂时隐藏桌面上的全部集合" on={prefs.hidden_all} onChange={() => api.setPrefs({ hiddenAll: !prefs.hidden_all })} />
          <div className="group-head static">
            <div className="grow">
              <strong>自动归类</strong>
              <div className="muted small">桌面新文件按规则移入集合；开启前先预演一次</div>
            </div>
            <label className="switch">
              <input
                type="checkbox"
                checked={prefs.auto_move}
                onChange={async () => {
                  if (prefs.auto_move) { await api.setAutoMove(false); load(); return; }
                  setBusy(true);
                  try { setPreview(await api.sweepPreview()); } catch (e) { setMsg(errMsg(e)); }
                  setBusy(false);
                }}
              />
              <span />
            </label>
          </div>
          <PrefRow
            title="在桌面显示「桌面集合」快捷方式"
            desc="网页上传、其他应用的选文件框里，可以直接从「桌面 → 桌面集合」进入集合文件夹"
            on={prefs.desktop_link}
            onChange={async () => {
              try { await api.setPrefs({ desktopLink: !prefs.desktop_link }); load(); }
              catch (e) { setMsg(errMsg(e)); }
            }}
          />
          <PrefRow title="在菜单栏显示 CPU / 内存" desc="点击菜单栏图标可弹出迷你面板" on={prefs.tray_stats} onChange={() => api.setPrefs({ trayStats: !prefs.tray_stats })} />
          <PrefRow title="登录时自动启动" desc="通过 LaunchAgent 在登录后自动打开，桌面集合随时可用" on={prefs.autostart} onChange={async () => {
            try { await api.setPrefs({ autostart: !prefs.autostart }); load(); } catch (e) { setMsg(errMsg(e)); }
          }} />
          <PrefRow title="集合互相吸附" desc="拖动集合靠近其他集合或屏幕边缘时自动贴齐，松手后不会互相重叠" on={prefs.snap} onChange={() => api.setPrefs({ snap: !prefs.snap })} />
          <div className="group-head static">
            <div className="grow"><strong>图标缓存</strong><div className="muted small">桌面文件图标与拖拽预览，超过 14 天未使用会自动清理</div></div>
            <span className="muted small">{fmtBytes(cache)}</span>
            <button
              className="btn small ghost"
              onClick={async () => { await api.pruneIconCache(); setCache(await api.cacheSize()); setMsg("图标缓存已清理"); }}
            >清理</button>
          </div>
          {prefs.snap && (
            <div className="group-head static">
              <div className="grow"><strong>吸附间隙</strong><div className="muted small">吸附后两个集合之间留出的距离</div></div>
              <div className="transp">
                <input
                  type="range" min={0} max={24} step={1} value={prefs.snap_gap}
                  onChange={(e) => { const v = Number(e.target.value); setPrefs({ ...prefs, snap_gap: v }); api.setPrefs({ snapGap: v }); }}
                />
                <span className="transp-val">{prefs.snap_gap}px</span>
              </div>
            </div>
          )}
        </div>
      )}
      {msg && <div className="toast">{msg}</div>}

      {preview && (
        <div className="modal-mask" onClick={() => setPreview(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <h3>自动归类预演</h3>
            {preview.length ? (
              <>
                <p className="muted small">以下 {preview.length} 个桌面文件会被移到对应集合（桌面不再保留），可随时在「最近移动」中撤销。</p>
                <div className="modal-list">
                  {preview.slice(0, 100).map((i) => (
                    <div key={i.path} className="item-row">
                      <span className="grow ellipsis" title={i.path}>{i.name}</span>
                      <span className="muted small">→ {i.target}</span>
                    </div>
                  ))}
                  {preview.length > 100 && <div className="muted small">… 另有 {preview.length - 100} 个文件</div>}
                </div>
              </>
            ) : (
              <p className="muted small">桌面目前没有需要归类的文件。开启后，新出现在桌面的文件会自动移入对应集合。</p>
            )}
            <div className="modal-actions">
              <button className="btn ghost" onClick={() => setPreview(null)}>取消</button>
              <button
                className="btn primary"
                disabled={busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    if (preview.length) await api.runSweep();
                    await api.setAutoMove(true);
                    setMsg(preview.length ? `已整理 ${preview.length} 个文件，自动归类已开启` : "自动归类已开启");
                    setPreview(null);
                    load();
                  } catch (e) { setMsg(errMsg(e)); }
                  setBusy(false);
                }}
              >{preview.length ? "开始整理并开启" : "仅开启"}</button>
            </div>
          </div>
        </div>
      )}

      {moves.length > 0 && (
        <>
          <div className="fold-head" onClick={foldMoves}>
            <h2>最近移动（{moves.length}）</h2>
            <span className="fold-chev">{showMoves ? "▾" : "▸"}</span>
          </div>
          {showMoves && <div className="list">
            {moves.slice(0, 6).map((m) => (
              <div key={m.id} className="group-head static">
                <div className="grow">
                  <div className="ellipsis"><strong>{m.name}</strong> <span className="muted small">{m.reason}</span></div>
                  <div className="muted small ellipsis" title={m.to}>{fmtDate(m.time)} · {m.to.replace(/^\/Users\/[^/]+\//, "~/")}</div>
                </div>
                <button
                  className="btn small ghost"
                  onClick={async () => {
                    try { await api.undoMove(m.id); setMsg(`已撤销：${m.name} 已移回原处`); load(); }
                    catch (e) { setMsg(errMsg(e)); }
                  }}
                >撤销</button>
              </div>
            ))}
            <div className="group-head static">
              <div className="grow muted small">撤销会把文件移回移动前的位置；已删除的记录不影响文件。</div>
              <button className="link" onClick={async () => { await api.clearMoveHistory(); load(); }}>清除记录</button>
            </div>
          </div>}
        </>
      )}

      <div className="fold-head" onClick={foldCols}>
        <h2>我的集合（{cols.length}）</h2>
        <span className="fold-chev">{showCols ? "▾" : "▸"}</span>
      </div>
      {showCols && (<>
      <p className="muted small">放进集合的文件实际存放在「桌面集合」文件夹里，桌面上不再保留。右键集合可打开存放目录，或在卡片里一键全部移回桌面。</p>
      <div className="col-cards">
        {cols.map((c) => (
          <div key={c.id} id={`col-card-${c.id}`} className="col-card" style={{ ["--c" as string]: c.color }}>
            <div className="col-card-head">
              <input className="name-input" defaultValue={c.name} key={c.name} onBlur={(e) => e.target.value !== c.name && patch({ id: c.id, name: e.target.value })} />
              <span className="tag plain">{KIND_LABEL[c.kind]}</span>
              <label className="switch" title={c.visible ? "在桌面上显示" : "已隐藏"}>
                <input type="checkbox" checked={c.visible} onChange={() => patch({ id: c.id, visible: !c.visible })} />
                <span />
              </label>
            </div>
            <div className="transp">
              <span className="muted small">透明度</span>
              <input
                type="range" min={0} max={100} step={1} value={c.transparency ?? 0}
                onChange={(e) => {
                  const v = Number(e.target.value);
                  setCols((cs) => cs.map((x) => (x.id === c.id ? { ...x, transparency: v } : x)));
                  api.setTransparency(c.id, v);
                }}
              />
              <span className="transp-val">{c.transparency ?? 0}%</span>
            </div>
            <div className="seg-row">
              <span className="muted small">标题</span>
              <div className="seg">
                {TITLE_POS.map(([k, label]) => (
                  <button key={k} className={(c.title_pos || "") === k ? "on" : ""} onClick={() => patch({ id: c.id, title_pos: k })}>{label}</button>
                ))}
              </div>
            </div>
            {c.kind !== "disk" && (<>
              <div className="seg-row">
                <span className="muted small">字号</span>
                <select
                  className="mini-select" value={c.title_size || 0}
                  onChange={(e) => patch({ id: c.id, title_size: Number(e.target.value) })}
                >
                  <option value={0}>默认</option>
                  {TITLE_SIZES.map((n) => <option key={n} value={n}>{n} px</option>)}
                </select>
              </div>
              <div className="seg-row">
                <span className="muted small">显示</span>
                <div className="seg">
                  {VIEW_MODE.map(([k, label]) => (
                    <button key={k} className={(c.view || "") === k ? "on" : ""} onClick={() => patch({ id: c.id, view: k })}>{label}</button>
                  ))}
                </div>
              </div>
            </>)}
            <div className="swatches">
              {COLORS.map((hex) => (
                <button key={hex} className={c.color === hex ? "sw on" : "sw"} style={{ background: hex }} onClick={() => patch({ id: c.id, color: hex })} />
              ))}
            </div>

            {c.kind === "collection" && (
              <div className="rules">
                <Field label="扩展名" placeholder="如 pdf, docx, xlsx" value={c.exts.join(", ")} onSave={(v) => patch({ id: c.id, exts: split(v) })} />
                <Field label="文件名包含" placeholder="如 合同, 发票, 截屏" value={c.keywords.join(", ")} onSave={(v) => patch({ id: c.id, keywords: split(v) })} />
                <div className="checks">
                  <label><input type="checkbox" checked={c.dirs} onChange={() => patch({ id: c.id, dirs: !c.dirs })} /> 收纳桌面文件夹</label>
                  <label><input type="checkbox" checked={c.catch_all} onChange={() => patch({ id: c.id, catch_all: !c.catch_all })} /> 兜底集合（收纳其余所有文件）</label>
                </div>
                {c.refs.length > 0 && <div className="muted small">另含 {c.refs.length} 个非桌面文件的快捷引用</div>}
              </div>
            )}
            {c.kind === "folder" && (
              <div className="rules">
                <div className="muted small ellipsis" title={c.folder ?? ""}>{c.folder}</div>
                <button className="btn small ghost" onClick={async () => { const f = await api.pickFolder(); if (f) patch({ id: c.id, folder: f }); }}>选择文件夹…</button>
              </div>
            )}
            {c.kind === "dropbox" && <div className="rules muted small">暂存 {c.refs.length} 个文件引用</div>}

            {c.dir && (
              <div className="dir-row">
                <span className="muted small ellipsis" title={c.dir}>{c.dir.replace(/^\/Users\/[^/]+\//, "~/")}</span>
                <button className="link" onClick={() => api.openPath(c.dir!)}>打开</button>
                <button
                  className="link danger"
                  onClick={async () => {
                    if (!(await api.confirm(`把「${c.name}」里的文件全部移回桌面？`, "文件会回到桌面，集合本身保留。也可以只把个别文件拖出。", "全部移回"))) return;
                    try {
                      const n = await api.returnCollectionFiles(c.id);
                      setMsg(`已把 ${n} 项移回桌面`);
                      load();
                    } catch (e) { setMsg(errMsg(e)); }
                  }}
                >全部移回桌面</button>
              </div>
            )}

            <label className="lock-row" title="锁定后不能拖动 / 缩放 / 双击改名，避免误触">
              <input type="checkbox" checked={c.locked} onChange={() => patch({ id: c.id, locked: !c.locked })} />
              锁定位置（防止误拖动）
            </label>

            <div className="col-card-foot">
              <button className="link" onClick={() => patch({ id: c.id, visible: true })}>在桌面显示</button>
              <button className="link danger" onClick={() => remove(c)}>删除集合</button>
            </div>
          </div>
        ))}
      </div>
      </>)}

      <h2>桌面小组件（{widgets.length}）</h2>
      <div className="create-row">
        {([
          ["overview", "◎", "系统概览", "CPU / 内存 / 磁盘"],
          ["memory", "▤", "内存排行", "占用最高的应用"],
        ] as [WidgetKind, string, string, string][]).map(([k, icon, title, sub]) => (
          <button key={k} className="create-card" onClick={async () => { await api.createWidget(k); load(); }}>
            <span className="create-icon">{icon}</span>
            <span><strong>{title}</strong><br /><span className="muted small">{sub}</span></span>
          </button>
        ))}
      </div>
      {widgets.length > 0 && (
        <div className="list">
          {widgets.map((w) => (
            <div key={w.id} className="group-head static">
              <div className="grow">
                <strong>{w.kind === "memory" ? "内存排行" : "系统概览"}</strong>
                <div className="muted small">拖动标题栏移动，拖动边缘缩放；右键可改大小或删除</div>
              </div>
              <div className="seg">
                {([["s", "小"], ["m", "中"], ["l", "大"]] as [WidgetSize, string][]).map(([k, label]) => (
                  <button key={k} className={w.size === k ? "on" : ""} onClick={async () => { await api.updateWidget({ id: w.id, size: k }); load(); }}>{label}</button>
                ))}
              </div>
              <label className="switch" title={w.visible ? "显示中" : "已隐藏"}>
                <input type="checkbox" checked={w.visible} onChange={async () => { await api.updateWidget({ id: w.id, visible: !w.visible }); load(); }} />
                <span />
              </label>
              <button className="link danger" onClick={async () => { await api.deleteWidget(w.id); load(); }}>删除</button>
            </div>
          ))}
        </div>
      )}

      {unassigned.length > 0 && (
        <>
          <h2>未归入任何集合的桌面文件（{unassigned.length}）</h2>
          <p className="muted small">隐藏系统桌面图标后，这些文件将不可见。可设置一个「兜底集合」，或把它们拖进某个集合。</p>
          <div className="chips">{unassigned.slice(0, 40).map((i) => <span key={i.path} className="chip" title={i.path}>{i.name}</span>)}</div>
        </>
      )}
    </div>
  );
}

const split = (v: string) => v.split(/[,，、\s]+/).map((s) => s.trim()).filter(Boolean);

function Field({ label, value, placeholder, onSave }: { label: string; value: string; placeholder: string; onSave: (v: string) => void }) {
  return (
    <label className="field">
      <span className="muted small">{label}</span>
      <input key={value} defaultValue={value} placeholder={placeholder} onBlur={(e) => e.target.value !== value && onSave(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()} />
    </label>
  );
}

function PrefRow({ title, desc, on, onChange }: { title: string; desc: string; on: boolean; onChange: () => void }) {
  return (
    <div className="group-head static">
      <div className="grow"><strong>{title}</strong><div className="muted small">{desc}</div></div>
      <label className="switch"><input type="checkbox" checked={on} onChange={onChange} /><span /></label>
    </div>
  );
}
