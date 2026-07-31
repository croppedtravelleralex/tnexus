"use client";

import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { ChatWorkbench } from "@/components/chat/chat-workbench";

export default function ChatPage() {
  return (
    <PageShell title="对话" fullBleed>
      <div className="flex h-[calc(100dvh-3rem)] min-h-0 flex-col px-4 pb-4 sm:px-6">
        <ElevatedCard className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <ChatWorkbench />
        </ElevatedCard>
      </div>
    </PageShell>
  );
}
