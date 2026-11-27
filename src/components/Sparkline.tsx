import { useId } from "react";
import { AppMem } from "../api";

/** 平滑曲线：用 Catmull-Rom 转三次贝塞尔，避免折线锯齿。 */
function smooth(points: [number, number][]): string {
  if (!points.length) return "";
  if (points.length === 1) return `M ${points[0][0]},${points[0][1]}`;
  let d = `M ${points[0][0]},${points[0][1]}`;
  for (let i = 0; i < points.length - 1; i++) {
    const p0 = points[i - 1] ?? points[i];
    const p1 = points[i];
    const p2 = points[i + 1];
    const p3 = points[i + 2] ?? p2;
    const c1x = p1[0] + (p2[0] - p0[0]) / 6;
    const c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6;
    const c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C ${c1x},${c1y} ${c2x},${c2y} ${p2[0]},${p2[1]}`;
  }
  return d;
}

export default function Sparkline({
  values, height = 26, color = "#3b6cff", max,
}: { values: number[]; height?: number; color?: string; max?: number }) {
  const gid = useId().replace(/:/g, "");
  const data = values.length > 1 ? values : values.length ? [values[0], values[0]] : [0, 0];
  const hi = Math.max(max ?? Math.max(...data), 1);
  const w = 100;
  const step = w / (data.length - 1);
  const pts: [number, number][] = data.map((v, i) => [i * step, height - 2 - (v / hi) * (height - 5)]);
  const line = smooth(pts);
  const area = `${line} L ${w},${height} L 0,${height} Z`;
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${height}`} preserveAspectRatio="none" role="img" aria-hidden="true">
      <defs>
        <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.34" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={area} fill={`url(#${gid})`} />
      <path d={line} fill="none" stroke={color} strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
    </svg>
  );
}

/** 记录每个应用最近 N 次的内存采样，供波浪图使用。 */
export function pushHistory(map: Map<string, number[]>, apps: AppMem[], max = 40) {
  for (const a of apps) {
    const arr = map.get(a.path) ?? [];
    arr.push(a.memory);
    if (arr.length > max) arr.shift();
    map.set(a.path, arr);
  }
}
