import type { JobDetail } from "@/lib/api";
import { fetchWithCache } from "@/lib/api-cache";
import { jobsApi } from "@/lib/api";

const JOB_CACHE_MS = 5 * 60_000;

export async function getJobDetailCached(
  jobId: string,
  options?: { force?: boolean },
): Promise<{ data: JobDetail; fromCache: boolean }> {
  return fetchWithCache(
    `job:${jobId}`,
    () => jobsApi.get(jobId),
    JOB_CACHE_MS,
    options,
  );
}

export function invalidateJobDetail(jobId: string) {
  // api-cache has no single-key delete; prefix invalidate jobs
  import("@/lib/api-cache").then(({ invalidateCache }) => invalidateCache("job:"));
}
