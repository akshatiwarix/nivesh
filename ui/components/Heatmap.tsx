import { type Component, createEffect, onCleanup, onMount } from "solid-js";
import { heatColor, layoutFor, type Rect, type Tile, tileText } from "./heatmapLayout";
import "./Heatmap.css";

export interface HeatmapProps {
  tiles: readonly Tile[];
  /** Keyboard cursor: the tile drawn with a frame. */
  selected?: number;
  label: string;
  onSelect?: (index: number) => void;
  onOpen?: (index: number) => void;
}

const css = (name: string, fallback: string) =>
  getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;

/**
 * A heatmap of tiles coloured by % change, drawn on one canvas (no DOM node per
 * tile, so 100 tiles cost nothing to update). It fills its container.
 */
export const Heatmap: Component<HeatmapProps> = (props) => {
  let canvas: HTMLCanvasElement | undefined;
  let box: HTMLDivElement | undefined;
  let size = { w: 0, h: 0 };
  // The rects the last draw used, so clicks hit-test the same layout.
  let rects: readonly Rect[] = [];

  const draw = () => {
    const ctx = canvas?.getContext?.("2d");
    if (!canvas || !ctx || size.w === 0 || size.h === 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(size.w * dpr);
    canvas.height = Math.round(size.h * dpr);
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, size.w, size.h);
    rects = layoutFor(props.tiles, size.w, size.h);
    const font = css("--font-mono", "monospace");
    const fg = css("--c-fg", "#d6d6d6");
    const frame = css("--c-amber", "#ffb000");
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    props.tiles.forEach((tile, i) => {
      const r = rects[i];
      if (!r) return;
      ctx.fillStyle = heatColor(tile.pct);
      ctx.fillRect(r.x, r.y, r.w, r.h);
      ctx.fillStyle = fg;
      const cx = r.x + r.w / 2;
      const cy = r.y + r.h / 2;
      if (r.w >= 52 && r.h >= 30) {
        ctx.font = `600 11px ${font}`;
        ctx.fillText(tile.label, cx, cy - 7, r.w - 4);
        ctx.font = `10px ${font}`;
        ctx.fillText(tileText(tile.pct), cx, cy + 7, r.w - 4);
      } else if (r.w >= 28 && r.h >= 14) {
        ctx.font = `10px ${font}`;
        ctx.fillText(tile.label, cx, cy, r.w - 2);
      }
      if (i === props.selected) {
        ctx.strokeStyle = frame;
        ctx.lineWidth = 2;
        ctx.strokeRect(r.x + 1, r.y + 1, r.w - 2, r.h - 2);
      }
    });
  };

  onMount(() => {
    if (!box) return;
    const measure = () => {
      size = { w: Math.floor(box?.clientWidth ?? 0), h: Math.floor(box?.clientHeight ?? 0) };
      draw();
    };
    measure();
    if (typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(measure);
      ro.observe(box);
      onCleanup(() => ro.disconnect());
    }
  });

  // Redraw on any change to the tiles or the cursor.
  createEffect(() => {
    for (const t of props.tiles) {
      void t.pct;
      void t.weight;
    }
    void props.selected;
    draw();
  });

  const hit = (e: MouseEvent) => {
    const rect = canvas?.getBoundingClientRect();
    if (!rect) return -1;
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    for (let i = 0; i < rects.length; i++) {
      const r = rects[i];
      if (r && x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) return i;
    }
    return -1;
  };

  return (
    <div class="heatmap" ref={box}>
      <canvas
        ref={canvas}
        class="heatmap__canvas"
        role="img"
        aria-label={props.label}
        onMouseDown={(e) => {
          const i = hit(e);
          if (i >= 0) props.onSelect?.(i);
        }}
        onDblClick={(e) => {
          const i = hit(e);
          if (i >= 0) props.onOpen?.(i);
        }}
      />
    </div>
  );
};
