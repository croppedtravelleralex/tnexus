"use client";

import { Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { conversationsApi } from "@/lib/api";
import type { Conversation } from "@/lib/conversations";
import { cn } from "@/lib/utils";

type Props = {
  activeId: string | null;
  onSelect: (c: Conversation) => void;
  onNew: () => void;
  refreshKey: number;
};

export function ConversationPanel({ activeId, onSelect, onNew, refreshKey }: Props) {
  const [items, setItems] = useState<Conversation[]>([]);

  useEffect(() => {
    void conversationsApi.list().then(setItems).catch(() => undefined);
  }, [refreshKey]);

  return (
    <div className="panel-card h-full border-r border-zinc-200">
      <div className="panel-header flex items-center justify-between text-zinc-900">
        <span>对话记录</span>
        <Button variant="outline" size="sm" onClick={onNew}>
          <Plus className="h-3.5 w-3.5" />
          新增对话
        </Button>
      </div>
      <div className="panel-body scrollbar-hide space-y-2">
        {items.length === 0 ? (
          <p className="text-sm text-zinc-400">点击「新增对话」开始创作</p>
        ) : (
          items.map((c) => (
            <button
              key={c.id}
              type="button"
              onClick={() => onSelect(c)}
              className={cn(
                "w-full rounded-lg border px-3 py-2.5 text-left transition-colors",
                activeId === c.id
                  ? "border-zinc-900 bg-zinc-50"
                  : "border-zinc-200 bg-white hover:bg-zinc-50"
              )}
            >
              <p className="line-clamp-2 text-sm font-medium text-zinc-800">{c.title}</p>
              <p className="mt-1 text-xs text-zinc-400">
                {new Date(c.updated_at).toLocaleString("zh-CN", {
                  month: "short",
                  day: "numeric",
                  hour: "2-digit",
                  minute: "2-digit",
                })}
              </p>
            </button>
          ))
        )}
      </div>
    </div>
  );
}
