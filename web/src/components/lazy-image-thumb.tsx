"use client";

import { useEffect, useRef, useState } from "react";
import { ImageIcon } from "lucide-react";

type Props = {
  src?: string;
  fallbackSrc?: string;
  alt?: string;
  className?: string;
  onClick?: () => void;
};

export function LazyImageThumb({ src, fallbackSrc, alt = "", className, onClick }: Props) {
  const [activeSrc, setActiveSrc] = useState<string | undefined>(undefined);
  const [failed, setFailed] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setFailed(false);
    setActiveSrc(undefined);
    const el = ref.current;
    if (!el) return;
    const primary = src || fallbackSrc;
    if (!primary) return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setActiveSrc(primary);
          observer.disconnect();
        }
      },
      { rootMargin: "120px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [src, fallbackSrc]);

  const handleError = () => {
    if (fallbackSrc && activeSrc !== fallbackSrc) {
      setActiveSrc(fallbackSrc);
      return;
    }
    setFailed(true);
  };

  const inner = activeSrc && !failed ? (
    // eslint-disable-next-line @next/next/no-img-element
    <img
      src={activeSrc}
      alt={alt}
      loading="lazy"
      decoding="async"
      className={className}
      onError={handleError}
    />
  ) : (
    <div className={`flex items-center justify-center bg-[var(--neo-surface-muted)] ${className ?? ""}`}>
      <ImageIcon className="size-6 text-[var(--neo-muted)]" />
    </div>
  );

  if (onClick) {
    return (
      <button type="button" ref={ref as never} className="block h-full w-full" onClick={onClick}>
        {inner}
      </button>
    );
  }

  return (
    <div ref={ref} className="h-full w-full">
      {inner}
    </div>
  );
}
