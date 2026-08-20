import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";
import { App } from "./app/App";
import { tauriClient, TauriClientProvider } from "./lib/tauri-client";
import "./styles/app.css";

const router = createBrowserRouter([{ path: "*", element: <App /> }]);

async function startApplication() {
  // The test-only plugin must stay out of release bundles and is enabled by VITE_WDIO.
  if (import.meta.env.VITE_WDIO === "1") await import("@wdio/tauri-plugin");
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <TauriClientProvider client={tauriClient}>
        <RouterProvider router={router} />
      </TauriClientProvider>
    </StrictMode>,
  );
}

void startApplication();
