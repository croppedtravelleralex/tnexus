"use client";

import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { GrokChatWorkbench } from "@/components/grok-chat/grok-chat-workbench";

export default function GrokChatPage() {
  return (
    <PageShell title="Grok 对话" fullBleed>
      <div className="flex h-[calc(100dvh-3rem)] min-h-0 flex-col px-4 pb-4 sm:px-6">
        <ElevatedCard className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <GrokChatWorkbench />
        </ElevatedCard>
      </div>
    </PageShell>
  );
}
