import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import CollectionWindow from "./pages/CollectionWindow";
import WidgetWindow from "./pages/WidgetWindow";
import Mini from "./pages/Mini";
import "./styles.css";

// 主窗口与桌面窗口共用同一份前端，通过 hash 路由区分：
// #/collection/<id> 集合    #/widget/<id> 小组件    #/mini 菜单栏迷你面板
const cw = location.hash.match(/^#\/collection\/([^?]+)/);
const wg = location.hash.match(/^#\/widget\/([^?]+)/);
const mini = location.hash.startsWith("#/mini");
if (cw || wg) document.documentElement.classList.add("cw");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {cw ? <CollectionWindow id={cw[1]} /> : wg ? <WidgetWindow id={wg[1]} /> : mini ? <Mini /> : <App />}
  </React.StrictMode>,
);
