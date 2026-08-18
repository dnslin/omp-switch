import { LayoutGrid, Server, Settings, Users } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { Link, NavLink, useLocation } from "react-router";

import { NavigationItem } from "../components/ui";
import { asAppError, useTauriClient } from "../lib/tauri-client";
import type { ShellStatus } from "./omp-presentation";
import { startupShellStatus } from "./omp-presentation";

const pages = [
  { to: "/overview", label: "概览", icon: LayoutGrid },
  { to: "/providers", label: "Providers", icon: Server },
  { to: "/roles", label: "角色", icon: Users },
  { to: "/settings", label: "设置", icon: Settings },
] as const;

export function MainShell({ children, status, contentClassName }: { children: ReactNode; status?: ShellStatus; contentClassName?: string }) {
  const client = useTauriClient();
  const location = useLocation();
  const [detectedStatus, setDetectedStatus] = useState<ShellStatus | null>(null);

  useEffect(() => {
    if (status) return;
    let active = true;
    setDetectedStatus(null);
    void client.getStartupState().then((state) => {
      if (active) setDetectedStatus(startupShellStatus(state));
    }).catch((cause: unknown) => {
      if (!active) return;
      const error = asAppError(cause, "OMP 状态不可用");
      setDetectedStatus({ title: error.message, path: "配置目录不可用", status: error.action, tone: "warning" });
    });
    return () => { active = false; };
  }, [client, location.pathname, status]);

  const footer = status ?? detectedStatus ?? { title: "正在检测 OMP", path: "配置目录检测中", status: "请稍候", tone: "warning" as const };
  return (
    <div className="app-frame app-frame--shell">
      <main className="shell-main">
        <aside className="sidebar">
          <nav className="sidebar-nav" aria-label="主导航">
            {pages.map(({ to, label, icon }) => (
              <NavLink key={to} to={to}>
                {({ isActive }) => <NavigationItem active={isActive} icon={icon}>{label}</NavigationItem>}
              </NavLink>
            ))}
          </nav>
          <Link className={`sidebar-footer sidebar-footer--${footer.tone}`} to="/settings#omp-settings" aria-label={`${footer.title}，${footer.path}，${footer.status}`}>
            <strong><span className="status-dot" aria-hidden="true" />{footer.title}</strong>
            <code>{footer.path}</code>
            <span className="sidebar-footer__status">{footer.status}</span>
          </Link>
        </aside>
        <section className={contentClassName ? `page-content ${contentClassName}` : "page-content"}>{children}</section>
      </main>
    </div>
  );
}
