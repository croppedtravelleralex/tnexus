"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, ImageIcon, LoaderCircle, RefreshCw, Search, Tag, Trash2 } from "lucide-react";
import { DateRangeFilter } from "@/components/date-range-filter";
import { ImageLightbox } from "@/components/image-lightbox";
import { LazyImageThumb } from "@/components/lazy-image-thumb";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { imagesApi, apiAssetUrl, type ManagedImage } from "@/lib/api";
import { fetchWithCache, invalidateCache } from "@/lib/api-cache";
import { formatImageDateTime } from "@/lib/account-display";
import { formatDuration } from "@/lib/format-duration";

const PAGE_SIZE = 24;

function thumbSrc(item: ManagedImage) {
  const inline = b64Fallback(item);
  if (inline) return inline;
  if (item.thumb_api_url) return apiAssetUrl(item.thumb_api_url);
  if (item.thumbnail_url) return apiAssetUrl(item.thumbnail_url);
  if (item.url) return apiAssetUrl(item.url);
  return undefined;
}

function thumbFallback(item: ManagedImage) {
  return b64Fallback(item) || apiAssetUrl(item.thumbnail_url) || apiAssetUrl(item.url) || undefined;
}

function fullSrc(item: ManagedImage) {
  if (item.rel) return apiAssetUrl(`/api/images/original/${item.rel}`);
  return apiAssetUrl(item.url) || apiAssetUrl(item.thumbnail_url) || "";
}

function b64Fallback(item: ManagedImage) {
  const raw = item.preview_b64 || item.b64_json;
  if (!raw) return undefined;
  if (raw.startsWith("data:")) return raw;
  return `data:image/png;base64,${raw}`;
}

function imageKey(item: ManagedImage) {
  return item.rel;
}

export default function ImageManagerPage() {
  const [items, setItems] = useState<ManagedImage[]>([]);
  const [startDate, setStartDate] = useState("");
  const [endDate, setEndDate] = useState("");
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState("");
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [page, setPage] = useState(1);
  const [deleteByDateOpen, setDeleteByDateOpen] = useState(false);
  const [deleteStartDate, setDeleteStartDate] = useState("");
  const [deleteEndDate, setDeleteEndDate] = useState("");
  const [allTags, setAllTags] = useState<string[]>([]);
  const [filterTags, setFilterTags] = useState<string[]>([]);
  const [lightboxOpen, setLightboxOpen] = useState(false);
  const [lightboxIndex, setLightboxIndex] = useState(0);

  const loadImages = useCallback(
    async (options?: { force?: boolean; background?: boolean }) => {
      if (!options?.background) setLoading(true);
      setError("");
      const cacheKey = `images:${startDate}:${endDate}`;
      try {
        const [listResult, tagsResult] = await Promise.all([
          fetchWithCache(
            cacheKey,
            () =>
              imagesApi.list({
                start_date: startDate || undefined,
                end_date: endDate || undefined,
              }),
            30_000,
            { force: options?.force },
          ),
          imagesApi.tags().catch(() => ({ tags: [] as string[] })),
        ]);
        const data = listResult.data;
        setItems(data.items);
        setAllTags(tagsResult.tags);
        setSelectedPaths((current) => current.filter((p) => data.items.some((item) => imageKey(item) === p)));
        setPage(1);
      } catch (err) {
        setError(err instanceof Error ? err.message : "加载图片失败");
        setItems([]);
      } finally {
        setLoading(false);
      }
    },
    [startDate, endDate],
  );

  useEffect(() => {
    void loadImages({ background: items.length > 0 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadImages]);

  const filtered = useMemo(() => {
    let list = items;
    if (filterTags.length > 0) {
      list = list.filter((item) => filterTags.every((t) => (item.tags ?? []).includes(t)));
    }
    const q = search.trim().toLowerCase();
    if (!q) return list;
    return list.filter(
      (item) =>
        item.name.toLowerCase().includes(q) ||
        (item.prompt ?? "").toLowerCase().includes(q) ||
        (item.tags ?? []).some((t) => t.toLowerCase().includes(q)),
    );
  }, [items, search, filterTags]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const safePage = Math.min(page, pageCount);
  const currentRows = filtered.slice((safePage - 1) * PAGE_SIZE, safePage * PAGE_SIZE);
  const selectedSet = useMemo(() => new Set(selectedPaths), [selectedPaths]);
  const allSelected = filtered.length > 0 && filtered.every((item) => selectedSet.has(imageKey(item)));
  const currentPageSelected = currentRows.length > 0 && currentRows.every((item) => selectedSet.has(imageKey(item)));

  const togglePaths = (paths: string[], checked: boolean) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      for (const p of paths) {
        if (checked) next.add(p);
        else next.delete(p);
      }
      return [...next];
    });
  };

  const onDeleteSelected = async () => {
    if (selectedPaths.length === 0) return;
    if (!window.confirm(`确定删除选中的 ${selectedPaths.length} 张图片？`)) return;
    setDeleting(true);
    try {
      await imagesApi.delete({ paths: selectedPaths });
      setSelectedPaths([]);
      invalidateCache("images:");
      await loadImages({ force: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除失败");
    } finally {
      setDeleting(false);
    }
  };

  const onDeleteByDate = async () => {
    if (!deleteStartDate && !deleteEndDate) return;
    if (!window.confirm(`确定删除 ${deleteStartDate || "…"} 至 ${deleteEndDate || "…"} 范围内的图片？`)) return;
    setDeleting(true);
    try {
      await imagesApi.delete({
        start_date: deleteStartDate || undefined,
        end_date: deleteEndDate || undefined,
        all_matching: true,
      });
      setDeleteByDateOpen(false);
      setDeleteStartDate("");
      setDeleteEndDate("");
      invalidateCache("images:");
      await loadImages({ force: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "按日期删除失败");
    } finally {
      setDeleting(false);
    }
  };

  const lightboxImages = filtered.map((item) => ({
    id: imageKey(item),
    src: fullSrc(item) || b64Fallback(item) || "",
    dimensions: item.width && item.height ? `${item.width}×${item.height}` : undefined,
  }));

  const onEditTags = async (img: ManagedImage) => {
    const raw = window.prompt("标签（逗号分隔）", (img.tags ?? []).join(", "));
    if (raw === null) return;
    const tags = raw
      .split(/[,，]/)
      .map((t) => t.trim())
      .filter(Boolean);
    try {
      const result = await imagesApi.setTags(imageKey(img), tags);
      setItems((prev) => prev.map((i) => (imageKey(i) === imageKey(img) ? { ...i, tags: result.tags } : i)));
      const tagData = await imagesApi.tags();
      setAllTags(tagData.tags);
      invalidateCache("images:");
    } catch (err) {
      setError(err instanceof Error ? err.message : "设置标签失败");
    }
  };

  return (
    <PageShell
      title="图片管理"
      actions={
        <Button size="sm" variant="toolbar" className="h-8 gap-1.5" onClick={() => { invalidateCache("images:"); void loadImages({ force: true }); }} disabled={loading}>
          {loading ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          刷新
        </Button>
      }
    >
      {error ? (
        <ElevatedCard className="mb-4 border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</ElevatedCard>
      ) : null}

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <DateRangeFilter
          startDate={startDate}
          endDate={endDate}
          onChange={(s, e) => {
            setStartDate(s);
            setEndDate(e);
          }}
        />
        <Button size="sm" className="h-8 gap-1" onClick={() => { invalidateCache("images:"); void loadImages({ force: true }); }} disabled={loading}>
          <Search className="size-3.5" />
          查询
        </Button>
        <div className="relative min-w-[180px] flex-1">
          <Search className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-[var(--neo-muted)]" />
          <Input
            placeholder="搜索提示词 / 标签"
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setPage(1);
            }}
            className="h-8 pl-8 text-sm"
          />
        </div>
        <Badge variant="muted">共 {filtered.length} 张</Badge>
        <Button
          size="sm"
          variant="toolbar"
          className="h-8 text-rose-600"
          disabled={selectedPaths.length === 0 || deleting}
          onClick={() => void onDeleteSelected()}
        >
          <Trash2 className="mr-1 size-3.5" />
          删除所选 ({selectedPaths.length})
        </Button>
        <Button size="sm" variant="toolbar" className="h-8 text-rose-600" onClick={() => setDeleteByDateOpen(true)}>
          按日期删除
        </Button>
      </div>

      {allTags.length > 0 ? (
        <div className="mb-3 flex flex-wrap items-center gap-2">
          <Tag className="size-3.5 text-[var(--neo-muted)]" />
          {allTags.map((tag) => {
            const active = filterTags.includes(tag);
            return (
              <button
                key={tag}
                type="button"
                onClick={() =>
                  setFilterTags((prev) => (active ? prev.filter((t) => t !== tag) : [...prev, tag]))
                }
                className={
                  active
                    ? "rounded-full bg-[var(--neo-primary)] px-2 py-0.5 text-[11px] text-white"
                    : "rounded-full bg-[var(--neo-surface-muted)] px-2 py-0.5 text-[11px] text-[var(--neo-muted)]"
                }
              >
                {tag}
              </button>
            );
          })}
          {filterTags.length > 0 ? (
            <button type="button" className="text-xs text-[var(--neo-muted)] underline" onClick={() => setFilterTags([])}>
              清除标签筛选
            </button>
          ) : null}
        </div>
      ) : null}

      <div className="mb-3 flex flex-wrap items-center gap-3 text-sm text-[var(--neo-muted)]">
        <label className="flex items-center gap-2">
          <input
            type="checkbox"
            checked={currentPageSelected}
            onChange={(e) => togglePaths(currentRows.map(imageKey), e.target.checked)}
          />
          本页全选
        </label>
        <label className="flex items-center gap-2">
          <input type="checkbox" checked={allSelected} onChange={(e) => togglePaths(filtered.map(imageKey), e.target.checked)} />
          全选结果
        </label>
      </div>

      {loading && items.length === 0 ? (
        <div className="py-16 text-center text-sm text-[var(--neo-muted)]">加载中…</div>
      ) : filtered.length === 0 ? (
        <div className="py-16 text-center text-sm text-[var(--neo-muted)]">没有找到图片</div>
      ) : (
        <div className="grid gap-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-6 xl:grid-cols-8">
          {currentRows.map((img) => {
            const key = imageKey(img);
            const checked = selectedSet.has(key);
            const thumb = thumbSrc(img);
            const fallback = thumbFallback(img);
            const hasPreview = Boolean(thumb || fallback);
            return (
              <ElevatedCard key={key} className="overflow-hidden">
                <div className="relative aspect-square bg-[var(--neo-surface-muted)]">
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={(e) => togglePaths([key], e.target.checked)}
                    className="absolute left-1.5 top-1.5 z-10 h-3.5 w-3.5 rounded"
                  />
                  {hasPreview ? (
                    <LazyImageThumb
                      src={thumb}
                      fallbackSrc={fallback}
                      className="h-full w-full object-cover"
                      onClick={() => {
                        const idx = filtered.findIndex((i) => imageKey(i) === key);
                        setLightboxIndex(Math.max(0, idx));
                        setLightboxOpen(true);
                      }}
                    />
                  ) : (
                    <div className="flex h-full items-center justify-center">
                      <ImageIcon className="size-8 text-[var(--neo-muted)]" />
                    </div>
                  )}
                </div>
                <div className="space-y-0.5 p-2">
                  <p className="truncate text-[11px] font-medium text-[var(--neo-ink)]">{img.prompt || img.name}</p>
                  <p className="truncate text-[10px] text-[var(--neo-muted)]">
                    {img.source === "api" ? (
                      <Badge variant="info" className="mr-1 h-4 px-1 text-[9px]">
                        API
                      </Badge>
                    ) : null}
                    {img.date}
                    {formatImageDateTime(img.created_at) ? ` · ${formatImageDateTime(img.created_at)}` : ""}
                    {img.duration_ms != null && img.duration_ms > 0 ? (
                      <span className="text-stone-400"> · {formatDuration(img.duration_ms)}</span>
                    ) : null}
                  </p>
                  {(img.tags ?? []).length > 0 ? (
                    <div className="flex flex-wrap gap-0.5">
                      {(img.tags ?? []).slice(0, 3).map((tag) => (
                        <span key={tag} className="rounded bg-[var(--neo-surface-muted)] px-1 py-0.5 text-[9px] text-[var(--neo-muted)]">
                          {tag}
                        </span>
                      ))}
                    </div>
                  ) : null}
                  <button type="button" className="text-[10px] text-[var(--neo-primary-deep)]" onClick={() => void onEditTags(img)}>
                    标签
                  </button>
                </div>
              </ElevatedCard>
            );
          })}
        </div>
      )}

      {filtered.length > 0 ? (
        <div className="mt-4 flex items-center justify-end gap-2 text-sm text-[var(--neo-muted)]">
          <span>
            第 {safePage} / {pageCount} 页
          </span>
          <Button size="sm" variant="toolbar" className="h-8 w-8 p-0" disabled={safePage <= 1} onClick={() => setPage((p) => Math.max(1, p - 1))}>
            <ChevronLeft className="size-4" />
          </Button>
          <Button
            size="sm"
            variant="toolbar"
            className="h-8 w-8 p-0"
            disabled={safePage >= pageCount}
            onClick={() => setPage((p) => Math.min(pageCount, p + 1))}
          >
            <ChevronRight className="size-4" />
          </Button>
        </div>
      ) : null}

      <ImageLightbox
        images={lightboxImages}
        currentIndex={lightboxIndex}
        open={lightboxOpen}
        onOpenChange={setLightboxOpen}
        onIndexChange={setLightboxIndex}
      />

      {deleteByDateOpen ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={() => setDeleteByDateOpen(false)}>
          <div className="neo-card w-full max-w-md p-5" onClick={(e) => e.stopPropagation()}>
            <h3 className="text-base font-semibold text-[var(--neo-ink)]">按日期范围删除</h3>
            <p className="mt-1 text-sm text-[var(--neo-muted)]">将删除该日期范围内所有图片记录（不可恢复）。</p>
            <div className="mt-4">
              <DateRangeFilter
                startDate={deleteStartDate}
                endDate={deleteEndDate}
                onChange={(s, e) => {
                  setDeleteStartDate(s);
                  setDeleteEndDate(e);
                }}
              />
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="toolbar" onClick={() => setDeleteByDateOpen(false)}>
                取消
              </Button>
              <Button className="bg-rose-600 hover:brightness-105" disabled={deleting} onClick={() => void onDeleteByDate()}>
                {deleting ? "删除中…" : "确认删除"}
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </PageShell>
  );
}
