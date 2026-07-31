"use client";

import { useSearchParams, useRouter } from "next/navigation";
import { useEffect, useState, Suspense } from "react";
import { SiteHeader } from "@/components/site-header";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { jobsApi, type JobDetail } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { Download } from "lucide-react";

function DetailInner() {
  const id = useSearchParams().get("id");
  const { user, bootstrapping } = useAuth();
  const router = useRouter();
  const [job, setJob] = useState<JobDetail | null>(null);

  useEffect(() => {
    if (!bootstrapping && !user) router.replace("/login");
  }, [bootstrapping, user, router]);

  useEffect(() => {
    if (!user || !id) return;
    void jobsApi.get(id).then(setJob).catch(() => undefined);
  }, [user, id]);

  if (bootstrapping || !user) return null;

  return (
    <main className="mx-auto max-w-4xl space-y-4 px-6 py-8">
      {job && (
        <>
          <h1 className="text-2xl font-semibold text-ink-100">任务详情</h1>
          <Card>
            <p className="text-ink-100">{job.input_prompt}</p>
            <p className="mt-2 text-sm text-ink-400">{job.status}</p>
          </Card>
          <div className="grid gap-4 sm:grid-cols-2">
            {job.results.map((r) => (
              <Card key={r.id}>
                {r.preview_url && (
                  // eslint-disable-next-line @next/next/no-img-element
                  <img src={r.preview_url} alt="" className="mb-3 rounded-lg" />
                )}
                <div className="flex justify-between">
                  <span className="text-sm text-ink-400">{r.provider}</span>
                  {r.download_url && (
                    <a href={r.download_url} target="_blank" rel="noreferrer">
                      <Button size="sm" variant="ghost"><Download className="h-4 w-4" />下载</Button>
                    </a>
                  )}
                </div>
              </Card>
            ))}
          </div>
        </>
      )}
    </main>
  );
}

export default function HistoryDetailPage() {
  return (
    <div className="min-h-screen">
      <SiteHeader />
      <Suspense>
        <DetailInner />
      </Suspense>
    </div>
  );
}
