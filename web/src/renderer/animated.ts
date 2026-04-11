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

  const children = container.children.slice();
  const totalChildren = children.length;

  // Hide everything
  for (const child of children) {
    child.alpha = 0;
  }

  const entityTotal = layout.entities.length;

  // Batch size in terms of entities: aim for ~1.5s total animation
  let entitiesPerBatch: number;
  if (entityTotal < 50) {
    entitiesPerBatch = 2;
  } else if (entityTotal < 200) {
    entitiesPerBatch = 6;
  } else {
    entitiesPerBatch = Math.max(10, Math.ceil(entityTotal / 30));
  }

  // Map entity batches to child-index batches so each reveal completes whole entities
  const childrenPerEntity = totalChildren / entityTotal;
  const childrenPerBatch = Math.max(1, Math.round(entitiesPerBatch * childrenPerEntity));

  // Delay between batches: ~15-30ms for a snappy but visible animation
  const batchDelay = entityTotal < 50 ? 30 : entityTotal < 200 ? 20 : 12;

  let childIdx = 0;
  let entitiesShown = 0;

  function revealBatch(): void {
    const end = Math.min(childIdx + childrenPerBatch, totalChildren);
    for (let i = childIdx; i < end; i++) {
      children[i].alpha = 1;
    }
    childIdx = end;
    entitiesShown = Math.min(Math.round(childIdx / childrenPerEntity), entityTotal);
    badge.textContent = `${entitiesShown} / ${entityTotal}`;

    if (childIdx < totalChildren) {
      setTimeout(revealBatch, batchDelay);
    } else {
      badge.textContent = `${entityTotal} entities`;
      onComplete();
    }
  }

  // Start the animation after a small initial pause so the user sees the empty canvas
  setTimeout(revealBatch, 150);
}
