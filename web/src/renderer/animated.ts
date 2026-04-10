/**
 * Animated layout renderer for the showcase modal.
 *
 * Renders all entities at alpha=0, then reveals them in batches
 * with a small stagger delay for a "popping in" effect.
 */

import type { Container } from "pixi.js";
import type { LayoutResult } from "../engine";
import { renderLayout } from "./entities";

/**
 * Render a layout with staggered entity animation.
 *
 * Uses the shared renderLayout() to draw everything, then sets all
 * entity graphics to alpha=0 and fades them in over time.
 *
 * @param layout    The layout result to render
 * @param container The PixiJS container to render into
 * @param badge     DOM element to update with entity count progress
 * @param onComplete Called when the animation finishes
 */
export function renderLayoutAnimated(
  layout: LayoutResult,
  container: Container,
  badge: HTMLElement,
  onComplete: () => void,
): void {
  // Render everything normally (no hover/click callbacks in the preview)
  renderLayout(layout, container);

  // Collect all direct children that are entity graphics
  const children = container.children.slice();

  // Hide everything
  for (const child of children) {
    child.alpha = 0;
  }

  const total = children.length;
  const entityTotal = layout.entities.length;

  // Batch size: aim for ~1.5s total animation
  // For small layouts (< 50 entities): reveal 2-3 at a time
  // For medium layouts (50-200): reveal 5-8 at a time
  // For large layouts (200+): reveal 10-20 at a time
  let batchSize: number;
  if (entityTotal < 50) {
    batchSize = 2;
  } else if (entityTotal < 200) {
    batchSize = 6;
  } else {
    batchSize = Math.max(10, Math.ceil(entityTotal / 30));
  }

  // Delay between batches: ~15-30ms for a snappy but visible animation
  const batchDelay = entityTotal < 50 ? 30 : entityTotal < 200 ? 20 : 12;

  let idx = 0;

  function revealBatch(): void {
    const end = Math.min(idx + batchSize, total);
    for (let i = idx; i < end; i++) {
      children[i].alpha = 1;
    }
    idx = end;
    badge.textContent = `${Math.min(idx, entityTotal)} / ${entityTotal}`;

    if (idx < total) {
      setTimeout(revealBatch, batchDelay);
    } else {
      badge.textContent = `${entityTotal} entities`;
      onComplete();
    }
  }

  // Start the animation after a small initial pause so the user sees the empty canvas
  setTimeout(revealBatch, 150);
}
