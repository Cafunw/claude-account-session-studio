import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

async function boot() {
  // Plain `vite` in a browser has no Tauri backend: answer with fabricated data. DEV is false in builds, so this drops out.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) (await import("./devMock")).installDevMock();
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}

boot();
