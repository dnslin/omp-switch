import { LayoutGrid, Server, Settings, Users } from "lucide-react";
import type { ReactNode } from "react";
import { Link, NavLink } from "react-router";

import { NavigationItem } from "../components/ui";
import type { ShellStatus } from "./omp-presentation";

const pages = [
  { to: "/overview", label: "概览", icon: LayoutGrid },
  { to: "/providers", label: "Providers", icon: Server },
  { to: "/roles", label: "角色", icon: Users },
  { to: "/settings", label: "设置", icon: Settings },
] as const;

export function MainShell({ children, status }: { children: ReactNode; status?: ShellStatus }) {
  const footer = status ?? { title: "尚未检测 OMP", path: "配置目录不可用", status: "请先完成 OMP 检测", tone: "warning" as const };
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
        <section className="page-content">{children}</section>
      </main>
    </div>
  );
}
