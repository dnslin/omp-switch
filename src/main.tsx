import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, RouterProvider } from "react-router";
import { App } from "./app/App";
import { tauriClient, TauriClientProvider } from "./lib/tauri-client";
import "./styles/app.css";

const router = createBrowserRouter([{ path: "*", element: <App /> }]);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <TauriClientProvider client={tauriClient}>
      <RouterProvider router={router} />
    </TauriClientProvider>
  </StrictMode>,
);
