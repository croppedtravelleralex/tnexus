"use client";

import { GRADIENT_STORAGE_KEY, GRADIENT_THEMES, type GradientThemeId } from "@/lib/gradient-themes";
import { useEffect, useState } from "react";

export function GradientPicker() {
  const [theme, setTheme] = useState<GradientThemeId>("moss");

  useEffect(() => {
    const saved = localStorage.getItem(GRADIENT_STORAGE_KEY) as GradientThemeId | null;
    if (saved && GRADIENT_THEMES.some((t) => t.id === saved)) {
      setTheme(saved);
      document.documentElement.dataset.gradientTheme = saved;
    } else {
      document.documentElement.dataset.gradientTheme = "moss";
    }
  }, []);

  const select = (id: GradientThemeId) => {
    setTheme(id);
    localStorage.setItem(GRADIENT_STORAGE_KEY, id);
    document.documentElement.dataset.gradientTheme = id;
  };

  return (
    <div className="space-y-2">
      <p className="text-xs font-medium uppercase tracking-wider text-ink-400">视觉渐变</p>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        {GRADIENT_THEMES.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => select(t.id)}
            className={`group rounded-xl border p-2 text-left transition-all ${
              theme === t.id
                ? "border-ink-400/60 ring-1 ring-ink-400/30"
                : "border-ink-700/50 hover:border-ink-500/40"
            }`}
          >
            <div
              className="mb-2 h-8 rounded-lg border border-ink-700/30"
              style={{ background: t.preview }}
            />
            <div className="text-xs font-medium text-ink-100">{t.label}</div>
            <div className="mt-0.5 text-[10px] leading-tight text-ink-400">{t.description}</div>
          </button>
        ))}
      </div>
    </div>
  );
}
