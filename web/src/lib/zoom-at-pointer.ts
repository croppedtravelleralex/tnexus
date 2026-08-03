export type PanOffset = { x: number; y: number };

/** Zoom toward the pointer while keeping the point under the cursor fixed on screen. */
export function zoomAtPointer(
  clientX: number,
  clientY: number,
  viewport: HTMLElement,
  scale: number,
  offset: PanOffset,
  factor: number,
  minScale: number,
  maxScale: number,
): { scale: number; offset: PanOffset } | null {
  const newScale = Math.min(maxScale, Math.max(minScale, scale * factor));
  if (newScale === scale) return null;

  const rect = viewport.getBoundingClientRect();
  const centerX = rect.width / 2;
  const centerY = rect.height / 2;
  const mouseX = clientX - rect.left;
  const mouseY = clientY - rect.top;
  const ratio = newScale / scale;
  const dx = mouseX - centerX - offset.x;
  const dy = mouseY - centerY - offset.y;

  return {
    scale: newScale,
    offset: {
      x: offset.x - dx * (ratio - 1),
      y: offset.y - dy * (ratio - 1),
    },
  };
}

export function wheelZoomFactor(deltaY: number, strength = 1.12): number {
  return deltaY > 0 ? 1 / strength : strength;
}
