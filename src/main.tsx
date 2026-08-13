import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import { App } from "./app/App";
import { tauriClient, TauriClientProvider } from "./lib/tauri-client";
import "./styles/app.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <TauriClientProvider client={tauriClient}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </TauriClientProvider>
  </StrictMode>,
);
