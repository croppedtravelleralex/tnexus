"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { SiteHeader } from "@/components/site-header";
import { Card } from "@/components/ui/card";
import { jobsApi, type JobRecord } from "@/lib/api";
import { useAuth } from "@/lib/auth";

export default function HistoryPage() {
  const { user, loading } = useAuth();
  const router = useRouter();
  const [jobs, setJobs] = useState<JobRecord[]>([]);

  useEffect(() => {
    if (!loading && !user) router.replace("/login");
  }, [loading, user, router]);

  useEffect(() => {
    if (!user) return;
    void jobsApi.list().then(setJobs).catch(() => undefined);
  }, [user]);

  if (loading || !user) return null;

  return (
    <div className="min-h-screen">
      <SiteHeader />
      <main className="mx-auto max-w-4xl space-y-4 px-6 py-8">
        <h1 className="text-2xl font-semibold text-ink-100">历史任务</h1>
        {jobs.map((job) => (
          <Link key={job.id} href={`/history/detail?id=${job.id}`}>
            <Card className="transition hover:border-ink-400/50">
              <div className="flex items-center justify-between gap-4">
                <div>
                  <p className="line-clamp-1 text-ink-100">{job.input_prompt}</p>
                  <p className="mt-1 text-xs text-ink-400">
                    {job.mode} · {job.workflow_path} · {job.status}
                  </p>
                </div>
                <span className="text-xs text-ink-400">
                  {new Date(job.created_at).toLocaleString()}
                </span>
              </div>
            </Card>
          </Link>
        ))}
        {jobs.length === 0 && <p className="text-ink-400">暂无任务</p>}
      </main>
    </div>
  );
}
