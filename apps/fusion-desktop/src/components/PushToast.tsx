import React, { useEffect, useState } from "react";
import { CheckCircle2, AlertCircle, HelpCircle, ShieldAlert, X } from "lucide-react";
import type { DesktopNotificationEventType } from "../lib/desktop-notifications";

export interface PushToastMessage {
  id: string;
  type: DesktopNotificationEventType;
  title: string;
  body: string;
  timestamp: number;
}

export interface PushToastProps {
  toasts: PushToastMessage[];
  onDismiss: (id: string) => void;
}

export function PushToast({ toasts, onDismiss }: PushToastProps) {
  if (toasts.length === 0) return null;

  return (
    <div
      data-testid="push-toast-container"
      className="fixed top-4 right-4 z-50 flex flex-col gap-2 max-w-sm w-full pointer-events-none"
    >
      {toasts.map((toast) => (
        <ToastItem key={toast.id} toast={toast} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function ToastItem({
  toast,
  onDismiss,
}: {
  toast: PushToastMessage;
  onDismiss: (id: string) => void;
}) {
  const [isVisible, setIsVisible] = useState(true);

  useEffect(() => {
    const timer = setTimeout(() => {
      setIsVisible(false);
      setTimeout(() => onDismiss(toast.id), 200);
    }, 5000);
    return () => clearTimeout(timer);
  }, [toast.id, onDismiss]);

  const getIcon = () => {
    switch (toast.type) {
      case "taskCompletion":
        return <CheckCircle2 className="w-4 h-4 text-emerald-600" />;
      case "approvalNeeded":
        return <ShieldAlert className="w-4 h-4 text-amber-600" />;
      case "questionAsked":
        return <HelpCircle className="w-4 h-4 text-[#5100cd]" />;
      case "sessionError":
        return <AlertCircle className="w-4 h-4 text-rose-600" />;
    }
  };

  const getBadgeClass = () => {
    switch (toast.type) {
      case "taskCompletion":
        return "bg-emerald-50 border-emerald-200/80";
      case "approvalNeeded":
        return "bg-amber-50 border-amber-200/80";
      case "questionAsked":
        return "bg-purple-50 border-purple-200/80";
      case "sessionError":
        return "bg-rose-50 border-rose-200/80";
    }
  };

  return (
    <div
      data-testid="push-toast-item"
      className={`pointer-events-auto rounded-xl border border-zinc-200/90 bg-white/95 backdrop-blur-md p-3.5 shadow-xl transition-all duration-200 flex items-start gap-3 ${
        isVisible ? "opacity-100 translate-y-0" : "opacity-0 -translate-y-2"
      }`}
    >
      <div
        className={`w-7 h-7 rounded-lg flex items-center justify-center shrink-0 border ${getBadgeClass()}`}
      >
        {getIcon()}
      </div>

      <div className="min-w-0 flex-1 pt-0.5">
        <h4 className="text-xs font-semibold text-zinc-900 leading-tight">
          {toast.title}
        </h4>
        <p className="text-[11px] text-zinc-500 mt-1 leading-relaxed line-clamp-2">
          {toast.body}
        </p>
      </div>

      <button
        type="button"
        onClick={() => {
          setIsVisible(false);
          setTimeout(() => onDismiss(toast.id), 200);
        }}
        className="p-1 rounded text-zinc-400 hover:text-zinc-700 hover:bg-zinc-100 transition-colors cursor-pointer shrink-0 -mr-1 -mt-1"
        aria-label="Dismiss notification"
      >
        <X className="w-3.5 h-3.5" />
      </button>
    </div>
  );
}

export default PushToast;
