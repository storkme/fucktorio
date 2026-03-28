"""Shared rendering themes for the visualizer and showcase.

Each theme is a JavaScript object with draw* methods. These are emitted as
inline JS into self-contained HTML files.

Both `visualize.py` and `showcase.py` import THEME_JS to get the rendering
code for all themes + the dispatch helpers.

Themes:
  - schematic: clean, colorful, diagrammatic (the original style)
  - factorio: dark, industrial, game-realistic
"""

from __future__ import annotations

# ---------------------------------------------------------------------------
# Shared JS utilities (direction helpers, adjacency, classification)
# ---------------------------------------------------------------------------

UTILS_JS = r"""
// Direction helpers: dir 0=N, 4=E, 8=S, 12=W
function dirAngle(d) { return (d / 4) * Math.PI * 0.5; }
function dirDx(d) { return d === 4 ? 1 : d === 12 ? -1 : 0; }
function dirDy(d) { return d === 8 ? 1 : d === 0 ? -1 : 0; }

function isBelt(name) {
  return name === 'transport-belt' || name === 'fast-transport-belt'
    || name === 'express-transport-belt';
}
function isInserter(name) {
  return name === 'inserter' || name === 'fast-inserter'
    || name === 'long-handed-inserter';
}
function isPipe(name) { return name === 'pipe' || name === 'pipe-to-ground'; }

function pipeNeighbors(t) {
  const dirs = [[0,-1],[1,0],[0,1],[-1,0]];
  const result = [];
  for (const [dx,dy] of dirs) {
    const nb = tileMap[(t.x+dx)+','+(t.y+dy)];
    if (nb && isPipe(nb.entity)) result.push({dx, dy});
  }
  return result;
}

function beltTurnInfo(t) {
  // Detect 90-degree belt turns. A turn occurs when a perpendicular belt feeds
  // into this tile AND there is NO straight feeder from behind.
  // If both exist, it's a sideload junction (belt stays straight).
  const d = t.dir || 0;
  const dirs = [[0,-1],[1,0],[0,1],[-1,0]];
  let hasStraightFeeder = false;
  let perpFeeder = null;
  for (const [dx,dy] of dirs) {
    const nb = tileMap[(t.x+dx)+','+(t.y+dy)];
    if (!nb || !isBelt(nb.entity)) continue;
    const nd = nb.dir || 0;
    // Does this neighbor's direction point at our tile?
    if (nb.x + dirDx(nd) !== t.x || nb.y + dirDy(nd) !== t.y) continue;
    if (nd === d) {
      hasStraightFeeder = true;
    } else {
      const cross = dirDx(nd) * dirDy(d) - dirDy(nd) * dirDx(d);
      if (cross !== 0) perpFeeder = { fromDir: nd, turn: cross > 0 ? 'cw' : 'ccw' };
    }
  }
  if (perpFeeder && !hasStraightFeeder) return perpFeeder;
  return null;
}

function darkenColor(hex, factor) {
  const r = parseInt(hex.slice(1, 3), 16);
  const g = parseInt(hex.slice(3, 5), 16);
  const b = parseInt(hex.slice(5, 7), 16);
  return 'rgb(' + Math.round(r * factor) + ',' + Math.round(g * factor) + ',' + Math.round(b * factor) + ')';
}
"""

# ---------------------------------------------------------------------------
# Schematic theme (original style)
# ---------------------------------------------------------------------------

SCHEMATIC_THEME_JS = r"""
const schematic = {
  background: '#0a0a1a',
  gridLine: 'rgba(255,255,255,0.04)',

  drawBelt(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const baseColors = {
      'transport-belt': '#a89030',
      'fast-transport-belt': '#b03030',
      'express-transport-belt': '#3070b0',
    };
    const chevColors = {
      'transport-belt': '#e0d070',
      'fast-transport-belt': '#ff6060',
      'express-transport-belt': '#70b0f0',
    };
    const base = baseColors[t.entity] || '#a89030';
    const chev = chevColors[t.entity] || '#e0d070';

    const cx = px + w / 2;
    const cy = py + w / 2;
    const turn = beltTurnInfo(t);

    // Belt track background — curved for turns, rectangular for straight
    ctx.fillStyle = base;
    if (turn) {
      const angle = dirAngle(t.dir || 0);
      const sign = turn.turn === 'cw' ? 1 : -1;
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(angle);
      const crnX = sign * w / 2;
      const crnY = -w / 2;
      const ccw = turn.turn === 'cw';
      const sa = ccw ? Math.PI : 0;
      const ea = Math.PI * 0.5;
      ctx.beginPath();
      ctx.arc(crnX, crnY, w, sa, ea, ccw);
      ctx.arc(crnX, crnY, 0.01, ea, sa, !ccw);
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    } else {
      ctx.fillRect(px, py, w, w);
    }

    if (scale >= 4) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(px, py, w, w);
      ctx.clip();

      if (turn) {
        const angle = dirAngle(t.dir || 0);
        ctx.translate(cx, cy);
        ctx.rotate(angle);

        const sign = turn.turn === 'cw' ? 1 : -1;
        const cornerX = sign * w / 2;
        const cornerY = -w / 2;
        const r = w;

        // Outer rail arc
        ctx.strokeStyle = 'rgba(0,0,0,0.3)';
        ctx.lineWidth = Math.max(1, s * 0.08);
        const startAngle = turn.turn === 'cw' ? Math.PI : 0;
        const endAngle = Math.PI * 0.5;
        ctx.beginPath();
        ctx.arc(cornerX, cornerY, r + w * 0.02, startAngle, endAngle, turn.turn === 'cw');
        ctx.stroke();

        // Chevron arcs along the curve
        ctx.strokeStyle = chev;
        ctx.lineWidth = Math.max(1, s * 0.12);
        ctx.lineCap = 'round';
        for (let i = 0; i < 3; i++) {
          const frac = (i + 0.5) / 3;
          const a = turn.turn === 'cw'
            ? Math.PI - frac * Math.PI * 0.5
            : frac * Math.PI * 0.5;
          const chevR = w * 0.5;
          const mx = cornerX + chevR * Math.cos(a);
          const my = cornerY + chevR * Math.sin(a);
          const tangentA = turn.turn === 'cw' ? a - Math.PI * 0.5 : a + Math.PI * 0.5;
          const chevSize = w * 0.2;
          const perpX = Math.cos(tangentA);
          const perpY = Math.sin(tangentA);
          const normX = -perpY;
          const normY = perpX;
          ctx.beginPath();
          ctx.moveTo(mx - normX * chevSize + perpX * chevSize * 0.5, my - normY * chevSize + perpY * chevSize * 0.5);
          ctx.lineTo(mx + normX * chevSize, my + normY * chevSize);
          ctx.lineTo(mx - normX * chevSize - perpX * chevSize * 0.5, my - normY * chevSize - perpY * chevSize * 0.5);
          ctx.stroke();
        }
      } else {
        const angle = dirAngle(t.dir || 0);
        ctx.translate(cx, cy);
        ctx.rotate(angle);

        ctx.strokeStyle = chev;
        ctx.lineWidth = Math.max(1, s * 0.12);
        ctx.lineCap = 'round';
        const chevSize = w * 0.25;
        for (let i = -1; i <= 1; i++) {
          const oy = i * w * 0.3;
          ctx.beginPath();
          ctx.moveTo(-chevSize, oy + chevSize * 0.5);
          ctx.lineTo(0, oy - chevSize * 0.5);
          ctx.lineTo(chevSize, oy + chevSize * 0.5);
          ctx.stroke();
        }

        ctx.strokeStyle = 'rgba(0,0,0,0.3)';
        ctx.lineWidth = Math.max(1, s * 0.08);
        ctx.beginPath();
        ctx.moveTo(-w / 2, -w / 2);
        ctx.lineTo(-w / 2, w / 2);
        ctx.moveTo(w / 2, -w / 2);
        ctx.lineTo(w / 2, w / 2);
        ctx.stroke();
      }
      ctx.restore();
    }
  },

  drawPipe(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const cx = px + w / 2;
    const cy = py + w / 2;

    ctx.fillStyle = '#1a2a3a';
    ctx.fillRect(px, py, w, w);

    const neighbors = pipeNeighbors(t);
    const pipeWidth = Math.max(2, w * 0.4);

    ctx.strokeStyle = t.entity === 'pipe-to-ground' ? '#3a6090' : '#5a9ad0';
    ctx.lineWidth = pipeWidth;
    ctx.lineCap = 'round';

    if (neighbors.length === 0) {
      ctx.fillStyle = t.entity === 'pipe-to-ground' ? '#3a6090' : '#5a9ad0';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.6, 0, Math.PI * 2);
      ctx.fill();
    } else {
      for (const nb of neighbors) {
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        ctx.lineTo(cx + nb.dx * w / 2, cy + nb.dy * w / 2);
        ctx.stroke();
      }
    }

    if (neighbors.length >= 2) {
      ctx.fillStyle = t.entity === 'pipe-to-ground' ? '#3a6090' : '#5a9ad0';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.4, 0, Math.PI * 2);
      ctx.fill();
    }

    if (t.entity === 'pipe-to-ground') {
      ctx.fillStyle = '#0a1520';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.25, 0, Math.PI * 2);
      ctx.fill();
    }

    if (scale >= 8) {
      ctx.fillStyle = 'rgba(100,180,255,0.3)';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.2, 0, Math.PI * 2);
      ctx.fill();
    }
  },

  drawInserter(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const cx = px + w / 2;
    const cy = py + w / 2;

    ctx.fillStyle = '#2a3a2a';
    ctx.fillRect(px, py, w, w);

    const colors = {
      'inserter': '#7ab050',
      'fast-inserter': '#50a0e0',
      'long-handed-inserter': '#e06060',
    };
    const armColor = colors[t.entity] || '#7ab050';
    const angle = dirAngle(t.dir || 0);

    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(angle);

    ctx.fillStyle = '#444';
    ctx.beginPath();
    ctx.arc(0, w * 0.2, w * 0.15, 0, Math.PI * 2);
    ctx.fill();

    ctx.strokeStyle = armColor;
    ctx.lineWidth = Math.max(1.5, w * 0.12);
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(0, w * 0.2);
    ctx.lineTo(0, -w * 0.35);
    ctx.stroke();

    const clawY = -w * 0.35;
    const clawW = w * 0.18;
    ctx.beginPath();
    ctx.moveTo(-clawW, clawY - clawW * 0.6);
    ctx.lineTo(0, clawY);
    ctx.lineTo(clawW, clawY - clawW * 0.6);
    ctx.stroke();

    if (t.entity === 'long-handed-inserter' && scale >= 6) {
      ctx.strokeStyle = 'rgba(255,255,255,0.3)';
      ctx.lineWidth = Math.max(1, w * 0.06);
      ctx.setLineDash([w * 0.06, w * 0.06]);
      ctx.beginPath();
      ctx.moveTo(0, -w * 0.35);
      ctx.lineTo(0, -w * 0.5);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    ctx.restore();
  },

  drawMachine(ctx, px, py, pw, ph, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = pw - gap;
    const h = ph - gap;
    const cx = px + w / 2;
    const cy = py + h / 2;

    const r = scale >= 6 ? Math.min(scale * 0.3, 4) : 0;
    ctx.fillStyle = t.color;
    if (r > 0) {
      ctx.beginPath();
      ctx.moveTo(px + r, py);
      ctx.lineTo(px + w - r, py);
      ctx.quadraticCurveTo(px + w, py, px + w, py + r);
      ctx.lineTo(px + w, py + h - r);
      ctx.quadraticCurveTo(px + w, py + h, px + w - r, py + h);
      ctx.lineTo(px + r, py + h);
      ctx.quadraticCurveTo(px, py + h, px, py + h - r);
      ctx.lineTo(px, py + r);
      ctx.quadraticCurveTo(px, py, px + r, py);
      ctx.fill();
    } else {
      ctx.fillRect(px, py, w, h);
    }

    if (scale >= 6) {
      ctx.strokeStyle = 'rgba(255,255,255,0.15)';
      ctx.lineWidth = 1;
      const inset = Math.max(2, w * 0.08);
      ctx.strokeRect(px + inset, py + inset, w - inset * 2, h - inset * 2);
    }

    if (scale >= 8) {
      ctx.save();
      ctx.translate(cx, cy);
      const iconSize = Math.min(w, h) * 0.3;

      if (t.entity === 'chemical-plant') {
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineCap = 'round';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.15, -iconSize * 0.7);
        ctx.lineTo(-iconSize * 0.15, -iconSize * 0.2);
        ctx.lineTo(-iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.15, -iconSize * 0.2);
        ctx.lineTo(iconSize * 0.15, -iconSize * 0.7);
        ctx.stroke();
        ctx.fillStyle = 'rgba(100,200,255,0.25)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.35, iconSize * 0.2);
        ctx.lineTo(-iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, iconSize * 0.2);
        ctx.fill();
      } else if (t.entity === 'oil-refinery') {
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.fillStyle = 'rgba(255,255,255,0.1)';
        for (let i = 0; i < 3; i++) {
          const ty = -iconSize * 0.6 + i * iconSize * 0.45;
          const tw = iconSize * (0.5 + i * 0.15);
          ctx.fillRect(-tw / 2, ty, tw, iconSize * 0.35);
          ctx.strokeRect(-tw / 2, ty, tw, iconSize * 0.35);
        }
      } else {
        ctx.strokeStyle = 'rgba(255,255,255,0.45)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        const teeth = 6;
        const outerR = iconSize * 0.7;
        const innerR = iconSize * 0.45;
        ctx.beginPath();
        for (let i = 0; i < teeth; i++) {
          const a1 = (i / teeth) * Math.PI * 2;
          const a2 = ((i + 0.35) / teeth) * Math.PI * 2;
          const a3 = ((i + 0.5) / teeth) * Math.PI * 2;
          const a4 = ((i + 0.85) / teeth) * Math.PI * 2;
          if (i === 0) ctx.moveTo(Math.cos(a1) * outerR, Math.sin(a1) * outerR);
          ctx.lineTo(Math.cos(a2) * outerR, Math.sin(a2) * outerR);
          ctx.lineTo(Math.cos(a3) * innerR, Math.sin(a3) * innerR);
          ctx.lineTo(Math.cos(a4) * innerR, Math.sin(a4) * innerR);
          ctx.lineTo(Math.cos(((i + 1) / teeth) * Math.PI * 2) * outerR,
                      Math.sin(((i + 1) / teeth) * Math.PI * 2) * outerR);
        }
        ctx.closePath();
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(0, 0, innerR * 0.4, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.restore();
    }

    if (t.recipe && scale >= 14) {
      ctx.fillStyle = 'rgba(0,0,0,0.7)';
      ctx.font = 'bold ' + Math.max(8, scale * 0.5) + 'px sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'bottom';
      ctx.fillText(t.recipe.substring(0, 8), cx, py + h - Math.max(2, h * 0.05));
    }
  },

  drawPole(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const cx = px + w / 2;
    const cy = py + w / 2;

    ctx.fillStyle = '#2a2510';
    ctx.fillRect(px, py, w, w);

    const armW = Math.max(1.5, w * 0.2);
    const armLen = w * 0.38;
    ctx.fillStyle = '#c0a030';
    ctx.fillRect(cx - armW / 2, cy - armLen, armW, armLen * 2);
    ctx.fillRect(cx - armLen, cy - armW / 2, armLen * 2, armW);

    if (scale >= 8) {
      ctx.fillStyle = '#e0c040';
      ctx.beginPath();
      ctx.arc(cx, cy, armW * 0.6, 0, Math.PI * 2);
      ctx.fill();
    }

    if (scale >= 6) {
      ctx.strokeStyle = 'rgba(200,180,50,0.12)';
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.arc(cx, cy, 3.5 * scale, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }
  },

  drawSplitter(ctx, px, py, pw, ph, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = pw - gap;
    const h = ph - gap;
    ctx.fillStyle = '#a89030';
    ctx.fillRect(px, py, w, h);
    if (scale >= 6) {
      ctx.strokeStyle = '#706020';
      ctx.lineWidth = Math.max(1, Math.min(w, h) * 0.08);
      ctx.beginPath();
      if (t.w > t.h) {
        ctx.moveTo(px + w / 2, py);
        ctx.lineTo(px + w / 2, py + h);
      } else {
        ctx.moveTo(px, py + h / 2);
        ctx.lineTo(px + w, py + h / 2);
      }
      ctx.stroke();
    }
  },
};
"""

# ---------------------------------------------------------------------------
# Factorio theme (game-realistic)
# ---------------------------------------------------------------------------

FACTORIO_THEME_JS = r"""
const factorio = {
  background: '#2a2418',
  gridLine: 'rgba(0,0,0,0.15)',

  drawBelt(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const baseColors = {
      'transport-belt': '#6b5c28',
      'fast-transport-belt': '#6b2020',
      'express-transport-belt': '#1e4a6b',
    };
    const trackColors = {
      'transport-belt': '#8a7a3a',
      'fast-transport-belt': '#8a3535',
      'express-transport-belt': '#35658a',
    };
    const arrowColors = {
      'transport-belt': '#bba850',
      'fast-transport-belt': '#cc5555',
      'express-transport-belt': '#5590cc',
    };
    const base = baseColors[t.entity] || '#6b5c28';
    const track = trackColors[t.entity] || '#8a7a3a';
    const arrow = arrowColors[t.entity] || '#bba850';
    const frameColor = '#3a352a';

    const cx = px + w / 2;
    const cy = py + w / 2;
    const turn = beltTurnInfo(t);
    const frame = Math.max(1, w * 0.1);

    // Background — curved for turns, rectangular for straight
    if (turn) {
      const angle = dirAngle(t.dir || 0);
      const sign = turn.turn === 'cw' ? 1 : -1;
      // Outer frame arc
      ctx.fillStyle = frameColor;
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(angle);
      const crnX = sign * w / 2;
      const crnY = -w / 2;
      const ccw = turn.turn === 'cw';
      const sa = ccw ? Math.PI : 0;
      const ea = Math.PI * 0.5;
      ctx.beginPath();
      ctx.arc(crnX, crnY, w, sa, ea, ccw);
      ctx.arc(crnX, crnY, 0.01, ea, sa, !ccw);
      ctx.closePath();
      ctx.fill();
      // Inner belt surface
      ctx.fillStyle = base;
      ctx.beginPath();
      ctx.arc(crnX, crnY, w - frame, sa, ea, ccw);
      ctx.arc(crnX, crnY, frame, ea, sa, !ccw);
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    } else {
      ctx.fillStyle = frameColor;
      ctx.fillRect(px, py, w, w);
      ctx.fillStyle = base;
      ctx.fillRect(px + frame, py + frame, w - frame * 2, w - frame * 2);
    }

    if (scale >= 4) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(px, py, w, w);
      ctx.clip();

      if (turn) {
        const angle = dirAngle(t.dir || 0);
        ctx.translate(cx, cy);
        ctx.rotate(angle);

        const sign = turn.turn === 'cw' ? 1 : -1;
        const cornerX = sign * w / 2;
        const cornerY = -w / 2;

        // Frame edges (dark border arcs)
        ctx.strokeStyle = '#1a1810';
        ctx.lineWidth = Math.max(0.5, w * 0.06);
        const startAngle = turn.turn === 'cw' ? Math.PI : 0;
        const endAngle = Math.PI * 0.5;
        ctx.beginPath();
        ctx.arc(cornerX, cornerY, w, startAngle, endAngle, turn.turn === 'cw');
        ctx.stroke();

        // Direction arrow along curve
        ctx.strokeStyle = arrow;
        ctx.lineWidth = Math.max(1, s * 0.08);
        ctx.lineCap = 'round';
        const midFrac = 0.5;
        const a = turn.turn === 'cw'
          ? Math.PI - midFrac * Math.PI * 0.5
          : midFrac * Math.PI * 0.5;
        const chevR = w * 0.5;
        const mx = cornerX + chevR * Math.cos(a);
        const my = cornerY + chevR * Math.sin(a);
        const tangentA = turn.turn === 'cw' ? a - Math.PI * 0.5 : a + Math.PI * 0.5;
        const aSize = w * 0.15;
        const perpX = Math.cos(tangentA);
        const perpY = Math.sin(tangentA);
        const normX = -perpY;
        const normY = perpX;
        ctx.beginPath();
        ctx.moveTo(mx - normX * aSize + perpX * aSize * 0.4, my - normY * aSize + perpY * aSize * 0.4);
        ctx.lineTo(mx + normX * aSize, my + normY * aSize);
        ctx.lineTo(mx - normX * aSize - perpX * aSize * 0.4, my - normY * aSize - perpY * aSize * 0.4);
        ctx.stroke();
      } else {
        const angle = dirAngle(t.dir || 0);
        ctx.translate(cx, cy);
        ctx.rotate(angle);

        ctx.strokeStyle = 'rgba(0,0,0,0.25)';
        ctx.lineWidth = Math.max(0.5, w * 0.03);
        ctx.beginPath();
        ctx.moveTo(0, -w / 2);
        ctx.lineTo(0, w / 2);
        ctx.stroke();

        ctx.strokeStyle = track;
        ctx.lineWidth = Math.max(0.5, w * 0.04);
        const ridgeCount = 4;
        for (let i = 0; i < ridgeCount; i++) {
          const ry = -w * 0.4 + (i / (ridgeCount - 1)) * w * 0.8;
          ctx.beginPath();
          ctx.moveTo(-w * 0.35, ry);
          ctx.lineTo(-w * 0.05, ry);
          ctx.moveTo(w * 0.05, ry);
          ctx.lineTo(w * 0.35, ry);
          ctx.stroke();
        }

        ctx.strokeStyle = arrow;
        ctx.lineWidth = Math.max(1, s * 0.08);
        ctx.lineCap = 'round';
        const aSize = w * 0.15;
        ctx.beginPath();
        ctx.moveTo(-aSize, aSize * 0.4);
        ctx.lineTo(0, -aSize * 0.4);
        ctx.lineTo(aSize, aSize * 0.4);
        ctx.stroke();

        ctx.strokeStyle = '#1a1810';
        ctx.lineWidth = Math.max(0.5, w * 0.06);
        ctx.beginPath();
        ctx.moveTo(-w / 2, -w / 2);
        ctx.lineTo(-w / 2, w / 2);
        ctx.moveTo(w / 2, -w / 2);
        ctx.lineTo(w / 2, w / 2);
        ctx.stroke();
      }
      ctx.restore();
    }
  },

  drawPipe(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const cx = px + w / 2;
    const cy = py + w / 2;

    ctx.fillStyle = '#2a2418';
    ctx.fillRect(px, py, w, w);

    const neighbors = pipeNeighbors(t);
    const pipeWidth = Math.max(2, w * 0.45);
    const pipeBase = t.entity === 'pipe-to-ground' ? '#4a5560' : '#6a7580';
    const pipeHighlight = t.entity === 'pipe-to-ground' ? '#5a6570' : '#8a95a0';
    const pipeShadow = '#2a3038';

    ctx.lineCap = 'butt';

    if (neighbors.length === 0) {
      ctx.fillStyle = pipeBase;
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.5, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = pipeShadow;
      ctx.lineWidth = Math.max(1, pipeWidth * 0.1);
      ctx.stroke();
      ctx.fillStyle = pipeHighlight;
      ctx.beginPath();
      ctx.arc(cx - pipeWidth * 0.1, cy - pipeWidth * 0.1, pipeWidth * 0.2, 0, Math.PI * 2);
      ctx.fill();
    } else {
      for (const nb of neighbors) {
        ctx.strokeStyle = pipeShadow;
        ctx.lineWidth = pipeWidth + 2;
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        ctx.lineTo(cx + nb.dx * w / 2, cy + nb.dy * w / 2);
        ctx.stroke();
        ctx.strokeStyle = pipeBase;
        ctx.lineWidth = pipeWidth;
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        ctx.lineTo(cx + nb.dx * w / 2, cy + nb.dy * w / 2);
        ctx.stroke();
        ctx.strokeStyle = pipeHighlight;
        ctx.lineWidth = pipeWidth * 0.25;
        ctx.beginPath();
        const offX = nb.dy !== 0 ? -pipeWidth * 0.2 : 0;
        const offY = nb.dx !== 0 ? -pipeWidth * 0.2 : 0;
        ctx.moveTo(cx + offX, cy + offY);
        ctx.lineTo(cx + nb.dx * w / 2 + offX, cy + nb.dy * w / 2 + offY);
        ctx.stroke();
      }
    }

    if (neighbors.length >= 2) {
      ctx.fillStyle = pipeBase;
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.4, 0, Math.PI * 2);
      ctx.fill();
      if (scale >= 10) {
        ctx.fillStyle = '#505860';
        const boltR = pipeWidth * 0.06;
        for (let a = 0; a < 4; a++) {
          const ba = a * Math.PI / 2;
          const br = pipeWidth * 0.28;
          ctx.beginPath();
          ctx.arc(cx + Math.cos(ba) * br, cy + Math.sin(ba) * br, boltR, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    }

    if (t.entity === 'pipe-to-ground') {
      ctx.fillStyle = '#0a0e12';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.22, 0, Math.PI * 2);
      ctx.fill();
      if (scale >= 8) {
        ctx.strokeStyle = '#3a4048';
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(cx - pipeWidth * 0.15, cy);
        ctx.lineTo(cx + pipeWidth * 0.15, cy);
        ctx.moveTo(cx, cy - pipeWidth * 0.15);
        ctx.lineTo(cx, cy + pipeWidth * 0.15);
        ctx.stroke();
      }
    }

    if (scale >= 10 && t.entity === 'pipe') {
      ctx.fillStyle = 'rgba(80,140,200,0.15)';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.15, 0, Math.PI * 2);
      ctx.fill();
    }
  },

  drawInserter(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const cx = px + w / 2;
    const cy = py + w / 2;

    ctx.fillStyle = '#383830';
    ctx.fillRect(px, py, w, w);

    if (scale >= 6) {
      ctx.fillStyle = '#444438';
      const pad = w * 0.1;
      ctx.fillRect(px + pad, py + pad, w - pad * 2, w - pad * 2);
    }

    const colors = {
      'inserter': '#5a8a30',
      'fast-inserter': '#3070a0',
      'long-handed-inserter': '#a04040',
    };
    const highlightColors = {
      'inserter': '#7ab050',
      'fast-inserter': '#50a0e0',
      'long-handed-inserter': '#e06060',
    };
    const armColor = colors[t.entity] || '#5a8a30';
    const armHighlight = highlightColors[t.entity] || '#7ab050';
    const angle = dirAngle(t.dir || 0);

    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(angle);

    const baseSize = w * 0.22;
    ctx.fillStyle = '#333';
    ctx.fillRect(-baseSize, w * 0.08, baseSize * 2, baseSize * 1.2);
    ctx.fillStyle = '#444';
    ctx.fillRect(-baseSize + 1, w * 0.08 + 1, baseSize * 2 - 2, baseSize * 1.2 - 2);

    ctx.strokeStyle = armColor;
    ctx.lineWidth = Math.max(2, w * 0.14);
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(0, w * 0.15);
    ctx.lineTo(0, -w * 0.32);
    ctx.stroke();
    ctx.strokeStyle = armHighlight;
    ctx.lineWidth = Math.max(1, w * 0.05);
    ctx.beginPath();
    ctx.moveTo(w * 0.04, w * 0.15);
    ctx.lineTo(w * 0.04, -w * 0.32);
    ctx.stroke();

    const clawY = -w * 0.32;
    const clawW = w * 0.2;
    ctx.strokeStyle = '#666';
    ctx.lineWidth = Math.max(1.5, w * 0.1);
    ctx.beginPath();
    ctx.moveTo(-clawW, clawY - clawW * 0.5);
    ctx.lineTo(0, clawY + clawW * 0.1);
    ctx.lineTo(clawW, clawY - clawW * 0.5);
    ctx.stroke();

    if (scale >= 8) {
      ctx.fillStyle = '#555';
      ctx.beginPath();
      ctx.arc(0, w * 0.15, w * 0.05, 0, Math.PI * 2);
      ctx.fill();
      ctx.beginPath();
      ctx.arc(0, -w * 0.32, w * 0.04, 0, Math.PI * 2);
      ctx.fill();
    }

    if (t.entity === 'long-handed-inserter' && scale >= 6) {
      ctx.strokeStyle = armColor;
      ctx.lineWidth = Math.max(1.5, w * 0.1);
      ctx.beginPath();
      ctx.moveTo(0, -w * 0.32);
      ctx.lineTo(0, -w * 0.48);
      ctx.stroke();
      ctx.strokeStyle = '#666';
      ctx.lineWidth = Math.max(1, w * 0.08);
      const clawY2 = -w * 0.48;
      ctx.beginPath();
      ctx.moveTo(-clawW * 0.8, clawY2 - clawW * 0.4);
      ctx.lineTo(0, clawY2 + clawW * 0.05);
      ctx.lineTo(clawW * 0.8, clawY2 - clawW * 0.4);
      ctx.stroke();
    }

    ctx.restore();
  },

  drawMachine(ctx, px, py, pw, ph, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = pw - gap;
    const h = ph - gap;
    const cx = px + w / 2;
    const cy = py + h / 2;

    ctx.fillStyle = '#2a2a28';
    ctx.fillRect(px, py, w, h);

    const baseColor = t.color || '#888';
    ctx.fillStyle = darkenColor(baseColor, 0.45);
    const border = Math.max(2, w * 0.04);
    ctx.fillRect(px + border, py + border, w - border * 2, h - border * 2);

    if (scale >= 6) {
      ctx.fillStyle = darkenColor(baseColor, 0.3);
      const inset = Math.max(3, w * 0.1);
      ctx.fillRect(px + inset, py + inset, w - inset * 2, h - inset * 2);

      if (scale >= 10) {
        ctx.fillStyle = '#555550';
        const boltR = Math.max(1.5, w * 0.02);
        const boltOff = inset * 0.6;
        const corners = [
          [px + boltOff, py + boltOff],
          [px + w - boltOff, py + boltOff],
          [px + boltOff, py + h - boltOff],
          [px + w - boltOff, py + h - boltOff],
        ];
        for (const [bx, by] of corners) {
          ctx.beginPath();
          ctx.arc(bx, by, boltR, 0, Math.PI * 2);
          ctx.fill();
        }
      }

      ctx.strokeStyle = 'rgba(255,255,255,0.08)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px + border, py + border);
      ctx.lineTo(px + w - border, py + border);
      ctx.stroke();
    }

    if (scale >= 8) {
      ctx.save();
      ctx.translate(cx, cy);
      const iconSize = Math.min(w, h) * 0.3;

      if (t.entity === 'chemical-plant') {
        ctx.strokeStyle = 'rgba(200,220,255,0.4)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineCap = 'round';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.15, -iconSize * 0.7);
        ctx.lineTo(-iconSize * 0.15, -iconSize * 0.2);
        ctx.lineTo(-iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.15, -iconSize * 0.2);
        ctx.lineTo(iconSize * 0.15, -iconSize * 0.7);
        ctx.stroke();
        ctx.fillStyle = 'rgba(60,180,120,0.2)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.35, iconSize * 0.2);
        ctx.lineTo(-iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, iconSize * 0.2);
        ctx.fill();
      } else if (t.entity === 'oil-refinery') {
        ctx.fillStyle = 'rgba(180,160,120,0.15)';
        ctx.strokeStyle = 'rgba(200,180,140,0.3)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.08);
        for (let i = 0; i < 3; i++) {
          const ty = -iconSize * 0.6 + i * iconSize * 0.45;
          const tw = iconSize * (0.4 + i * 0.15);
          ctx.fillRect(-tw / 2, ty, tw, iconSize * 0.35);
          ctx.strokeRect(-tw / 2, ty, tw, iconSize * 0.35);
        }
        ctx.strokeStyle = 'rgba(200,180,140,0.2)';
        ctx.lineWidth = Math.max(1, iconSize * 0.06);
        ctx.beginPath();
        ctx.moveTo(iconSize * 0.3, -iconSize * 0.6);
        ctx.lineTo(iconSize * 0.3, -iconSize * 0.85);
        ctx.stroke();
      } else {
        ctx.strokeStyle = 'rgba(200,200,190,0.35)';
        ctx.fillStyle = 'rgba(200,200,190,0.08)';
        ctx.lineWidth = Math.max(2, iconSize * 0.12);
        const teeth = 8;
        const outerR = iconSize * 0.75;
        const innerR = iconSize * 0.5;
        ctx.beginPath();
        for (let i = 0; i < teeth; i++) {
          const a1 = (i / teeth) * Math.PI * 2;
          const a2 = ((i + 0.3) / teeth) * Math.PI * 2;
          const a3 = ((i + 0.5) / teeth) * Math.PI * 2;
          const a4 = ((i + 0.8) / teeth) * Math.PI * 2;
          if (i === 0) ctx.moveTo(Math.cos(a1) * outerR, Math.sin(a1) * outerR);
          ctx.lineTo(Math.cos(a2) * outerR, Math.sin(a2) * outerR);
          ctx.lineTo(Math.cos(a3) * innerR, Math.sin(a3) * innerR);
          ctx.lineTo(Math.cos(a4) * innerR, Math.sin(a4) * innerR);
          ctx.lineTo(Math.cos(((i + 1) / teeth) * Math.PI * 2) * outerR,
                      Math.sin(((i + 1) / teeth) * Math.PI * 2) * outerR);
        }
        ctx.closePath();
        ctx.fill();
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(0, 0, innerR * 0.35, 0, Math.PI * 2);
        ctx.fillStyle = darkenColor(baseColor, 0.25);
        ctx.fill();
        ctx.strokeStyle = 'rgba(200,200,190,0.25)';
        ctx.lineWidth = Math.max(1, iconSize * 0.06);
        ctx.stroke();
      }
      ctx.restore();
    }

    if (t.recipe && scale >= 14) {
      ctx.fillStyle = 'rgba(220,210,190,0.6)';
      ctx.font = 'bold ' + Math.max(8, scale * 0.45) + 'px sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'bottom';
      ctx.fillText(t.recipe.substring(0, 10), cx, py + h - Math.max(2, h * 0.05));
    }
  },

  drawPole(ctx, px, py, s, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = s - gap;
    const cx = px + w / 2;
    const cy = py + w / 2;

    ctx.fillStyle = '#2a2418';
    ctx.fillRect(px, py, w, w);

    const poleW = Math.max(2, w * 0.15);
    ctx.fillStyle = '#4a3820';
    ctx.fillRect(cx - poleW / 2, cy - w * 0.35, poleW, w * 0.7);

    ctx.fillStyle = '#3a2a15';
    const armLen = w * 0.35;
    const armW = Math.max(1.5, w * 0.12);
    ctx.fillRect(cx - armLen, cy - w * 0.2 - armW / 2, armLen * 2, armW);

    if (scale >= 8) {
      ctx.fillStyle = '#777';
      const dotR = Math.max(1, w * 0.04);
      ctx.beginPath();
      ctx.arc(cx - armLen + dotR, cy - w * 0.2, dotR, 0, Math.PI * 2);
      ctx.fill();
      ctx.beginPath();
      ctx.arc(cx + armLen - dotR, cy - w * 0.2, dotR, 0, Math.PI * 2);
      ctx.fill();
      ctx.beginPath();
      ctx.arc(cx, cy - w * 0.35, dotR, 0, Math.PI * 2);
      ctx.fill();
    }

    if (scale >= 6) {
      ctx.strokeStyle = 'rgba(180,160,40,0.08)';
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 5]);
      ctx.beginPath();
      ctx.arc(cx, cy, 3.5 * scale, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }
  },

  drawSplitter(ctx, px, py, pw, ph, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = pw - gap;
    const h = ph - gap;
    ctx.fillStyle = '#3a352a';
    ctx.fillRect(px, py, w, h);
    const frame = Math.max(1, Math.min(w, h) * 0.08);
    ctx.fillStyle = '#6b5c28';
    if (t.w > t.h) {
      ctx.fillRect(px + frame, py + frame, w / 2 - frame - 1, h - frame * 2);
      ctx.fillRect(px + w / 2 + 1, py + frame, w / 2 - frame - 1, h - frame * 2);
    } else {
      ctx.fillRect(px + frame, py + frame, w - frame * 2, h / 2 - frame - 1);
      ctx.fillRect(px + frame, py + h / 2 + 1, w - frame * 2, h / 2 - frame - 1);
    }
    if (scale >= 6) {
      ctx.fillStyle = '#555';
      if (t.w > t.h) {
        ctx.fillRect(px + w / 2 - 1, py + frame, 2, h - frame * 2);
      } else {
        ctx.fillRect(px + frame, py + h / 2 - 1, w - frame * 2, 2);
      }
    }
  },
};
"""

# ---------------------------------------------------------------------------
# Combined: all themes + dispatch helper
# ---------------------------------------------------------------------------

THEME_JS = UTILS_JS + SCHEMATIC_THEME_JS + FACTORIO_THEME_JS + r"""
// Theme dispatch
function getTheme() {
  return (typeof currentTheme !== 'undefined' && currentTheme === 'factorio') ? factorio : schematic;
}
"""
