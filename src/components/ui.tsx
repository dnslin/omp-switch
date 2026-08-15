import { LayoutGrid, Search, type LucideIcon } from "lucide-react";
import { type InputHTMLAttributes, type PropsWithChildren, type ReactNode } from "react";
import { Button } from "./ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "./ui/dialog";
import { Input as ShadcnInput } from "./ui/input";
import { Select as ShadcnSelect, SelectTrigger, SelectValue } from "./ui/select";

export { Button };


export function SearchInput({ className = "", ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <label className={`input-shell ${className}`}>
      <Search aria-hidden="true" size={18} />
      <ShadcnInput className="border-0 bg-transparent px-0 shadow-none focus-visible:ring-0" {...props} />
    </label>
  );
}

export function Select({ label = "请选择" }: { label?: string }) {
  return (
    <ShadcnSelect>
      <SelectTrigger aria-label={label}>
        <SelectValue placeholder={label} />
      </SelectTrigger>
    </ShadcnSelect>
  );
}

export function StatusIndicator({
  tone = "success",
  children,
}: PropsWithChildren<{ tone?: "success" | "neutral" | "warning" | "danger" }>) {
  return (
    <span className={`status-indicator status-indicator--${tone}`}>
      <span className="status-dot" aria-hidden="true" />
      {children}
    </span>
  );
}

export function StatusTag({ children }: PropsWithChildren) {
  return <span className="status-tag">{children}</span>;
}

export function NavigationItem({
  icon: Icon = LayoutGrid,
  active = false,
  children,
}: PropsWithChildren<{ icon?: LucideIcon; active?: boolean }>) {
  return (
    <span className={`navigation-item ${active ? "navigation-item--active" : ""}`}>
      <span className="navigation-active-bar" aria-hidden="true" />
      <Icon aria-hidden="true" size={22} />
      <span>{children}</span>
    </span>
  );
}

export function Card({ title, children }: PropsWithChildren<{ title?: string }>) {
  return (
    <section className="card">
      {title ? <h2 className="card-title">{title}</h2> : null}
      {children}
    </section>
  );
}

export function PageTitle({ title, description }: { title: string; description: string }) {
  return (
    <header className="page-title">
      <h1>{title}</h1>
      <p>{description}</p>
    </header>
  );
}

export function ConfirmDialog({
  title,
  children,
  onCancel,
  onConfirm,
}: PropsWithChildren<{ title: string; onCancel(): void; onConfirm(): void }>) {
  return (
    <Dialog open onOpenChange={(open) => { if (!open) onCancel(); }}>
      <DialogContent aria-describedby="confirm-dialog-description">
        <DialogTitle>{title}</DialogTitle>
        <DialogDescription id="confirm-dialog-description" className="mt-5 text-base text-[var(--color-text-secondary)]">
          {children}
        </DialogDescription>
        <footer className="mt-5 flex justify-end gap-3">
          <Button variant="secondary" onClick={onCancel}>取消</Button>
          <Button onClick={onConfirm}>确认</Button>
        </footer>
      </DialogContent>
    </Dialog>
  );
}

export function KeyValueRow({ label, value }: { label: ReactNode; value: ReactNode }) {
  return (
    <div className="key-value-row">
      <div>{label}</div>
      <div>{value}</div>
    </div>
  );
}
