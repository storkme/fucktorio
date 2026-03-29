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

// Item name abbreviation for rendering on entities (e.g. "iron-plate" -> "Fe", "copper-cable" -> "Cu~")
const _itemAbbrevs = {
  'iron-plate': 'Fe', 'iron-ore': 'Fe*', 'iron-gear-wheel': '\u2699',
  'iron-stick': 'Fe|',
  'copper-plate': 'Cu', 'copper-ore': 'Cu*', 'copper-cable': 'Cu~',
  'steel-plate': 'St',
  'stone': 'Stn', 'stone-brick': 'Brk',
  'coal': 'C',
  'wood': 'W',
  'electronic-circuit': 'GC', 'advanced-circuit': 'RC', 'processing-unit': 'BC',
  'plastic-bar': 'Pl',
  'sulfur': 'S',
  'battery': 'Bat',
  'engine-unit': 'Eng', 'electric-engine-unit': 'E.E',
  'flying-robot-frame': 'Bot',
  'low-density-structure': 'LDS',
  'rocket-fuel': 'RF', 'solid-fuel': 'SF',
  'petroleum-gas': 'Pet', 'light-oil': 'L.O', 'heavy-oil': 'H.O',
  'lubricant': 'Lub', 'sulfuric-acid': 'H2S', 'water': 'H2O',
  'crude-oil': 'Oil', 'steam': 'Stm',
};
function itemAbbrev(name) {
  if (!name) return '';
  if (_itemAbbrevs[name]) return _itemAbbrevs[name];
  // Fallback: first 3 chars of first word
  const parts = name.split('-');
  return parts[0].substring(0, 3).charAt(0).toUpperCase() + parts[0].substring(1, 3);
}

// Item color for icon badges
const _itemColors = {
  'iron-plate': '#b0b0b8', 'iron-ore': '#8888a0', 'iron-gear-wheel': '#b0b0b8',
  'iron-stick': '#a0a0a8',
  'copper-plate': '#d88840', 'copper-ore': '#b86830', 'copper-cable': '#c87040',
  'steel-plate': '#d0d0d8',
  'stone': '#c0b890', 'stone-brick': '#a09870',
  'coal': '#404040',
  'electronic-circuit': '#40a840', 'advanced-circuit': '#c03030', 'processing-unit': '#3060c0',
  'plastic-bar': '#e0e0e0',
  'sulfur': '#d8d030',
  'petroleum-gas': '#806090', 'light-oil': '#d0a030', 'heavy-oil': '#804020',
  'water': '#4080d0', 'crude-oil': '#303030', 'steam': '#e0e0e8',
  'sulfuric-acid': '#c0c020', 'lubricant': '#30a030',
};
function itemColor(name) {
  if (!name) return '#ccc';
  if (_itemColors[name]) return _itemColors[name];
  // Hash to a color for unknowns
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) & 0xffffff;
  return '#' + ((h & 0x7f7f7f) + 0x404040).toString(16).padStart(6, '0');
}

// Draw item badge on an entity tile (call after main entity rendering)
function drawItemBadge(ctx, px, py, w, carries) {
  if (!carries || scale < 10) return;
  const abbr = itemAbbrev(carries);
  if (!abbr) return;
  const fontSize = Math.max(7, Math.min(w * 0.35, scale * 0.4));
  ctx.font = 'bold ' + fontSize + 'px monospace';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  const tx = px + w / 2;
  const ty = py + w * 0.78;
  // Background pill
  const metrics = ctx.measureText(abbr);
  const pw2 = metrics.width / 2 + 2;
  const ph2 = fontSize / 2 + 1;
  ctx.fillStyle = 'rgba(0,0,0,0.6)';
  ctx.beginPath();
  ctx.moveTo(tx - pw2 + ph2, ty - ph2);
  ctx.lineTo(tx + pw2 - ph2, ty - ph2);
  ctx.arc(tx + pw2 - ph2, ty, ph2, -Math.PI/2, Math.PI/2);
  ctx.lineTo(tx - pw2 + ph2, ty + ph2);
  ctx.arc(tx - pw2 + ph2, ty, ph2, Math.PI/2, -Math.PI/2);
  ctx.closePath();
  ctx.fill();
  // Text
  ctx.fillStyle = itemColor(carries);
  ctx.fillText(abbr, tx, ty);
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

    // Item badge
    drawItemBadge(ctx, px, py, w, t.carries);
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
    // Dark grey metallic track base matching in-game look
    const baseColors = {
      'transport-belt': '#484840',
      'fast-transport-belt': '#443838',
      'express-transport-belt': '#383844',
    };
    // Visible ridges on the track surface
    const trackColors = {
      'transport-belt': '#5a5a50',
      'fast-transport-belt': '#5a4545',
      'express-transport-belt': '#45455a',
    };
    // Bold, saturated chevrons matching icon colors
    const arrowColors = {
      'transport-belt': '#d4a820',
      'fast-transport-belt': '#cc3030',
      'express-transport-belt': '#3080cc',
    };
    const base = baseColors[t.entity] || '#484840';
    const track = trackColors[t.entity] || '#5a5a50';
    const arrow = arrowColors[t.entity] || '#d4a820';
    const frameColor = '#2a2820';

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
        ctx.strokeStyle = '#151510';
        ctx.lineWidth = Math.max(0.5, w * 0.07);
        const startAngle = turn.turn === 'cw' ? Math.PI : 0;
        const endAngle = Math.PI * 0.5;
        ctx.beginPath();
        ctx.arc(cornerX, cornerY, w, startAngle, endAngle, turn.turn === 'cw');
        ctx.stroke();

        // Bold direction chevron along curve
        ctx.strokeStyle = arrow;
        ctx.lineWidth = Math.max(1.5, s * 0.12);
        ctx.lineCap = 'round';
        const midFrac = 0.5;
        const a = turn.turn === 'cw'
          ? Math.PI - midFrac * Math.PI * 0.5
          : midFrac * Math.PI * 0.5;
        const chevR = w * 0.5;
        const mx = cornerX + chevR * Math.cos(a);
        const my = cornerY + chevR * Math.sin(a);
        const tangentA = turn.turn === 'cw' ? a - Math.PI * 0.5 : a + Math.PI * 0.5;
        const aSize = w * 0.18;
        const perpX = Math.cos(tangentA);
        const perpY = Math.sin(tangentA);
        const normX = -perpY;
        const normY = perpX;
        ctx.beginPath();
        ctx.moveTo(mx - normX * aSize + perpX * aSize * 0.5, my - normY * aSize + perpY * aSize * 0.5);
        ctx.lineTo(mx + normX * aSize, my + normY * aSize);
        ctx.lineTo(mx - normX * aSize - perpX * aSize * 0.5, my - normY * aSize - perpY * aSize * 0.5);
        ctx.stroke();
      } else {
        const angle = dirAngle(t.dir || 0);
        ctx.translate(cx, cy);
        ctx.rotate(angle);

        // Center divider line
        ctx.strokeStyle = 'rgba(0,0,0,0.3)';
        ctx.lineWidth = Math.max(0.5, w * 0.04);
        ctx.beginPath();
        ctx.moveTo(0, -w / 2);
        ctx.lineTo(0, w / 2);
        ctx.stroke();

        // Track ridges — horizontal bars like the belt icon
        ctx.strokeStyle = track;
        ctx.lineWidth = Math.max(1, w * 0.06);
        const ridgeCount = 5;
        for (let i = 0; i < ridgeCount; i++) {
          const ry = -w * 0.42 + (i / (ridgeCount - 1)) * w * 0.84;
          ctx.beginPath();
          ctx.moveTo(-w * 0.38, ry);
          ctx.lineTo(-w * 0.06, ry);
          ctx.moveTo(w * 0.06, ry);
          ctx.lineTo(w * 0.38, ry);
          ctx.stroke();
        }

        // Bold chevron arrows matching icon style
        ctx.strokeStyle = arrow;
        ctx.lineWidth = Math.max(1.5, s * 0.12);
        ctx.lineCap = 'round';
        const aSize = w * 0.2;
        for (let c = 0; c < 2; c++) {
          const oy = -w * 0.15 + c * w * 0.3;
          ctx.beginPath();
          ctx.moveTo(-aSize, oy + aSize * 0.5);
          ctx.lineTo(0, oy - aSize * 0.3);
          ctx.lineTo(aSize, oy + aSize * 0.5);
          ctx.stroke();
        }

        // Dark frame edges
        ctx.strokeStyle = '#151510';
        ctx.lineWidth = Math.max(0.5, w * 0.07);
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
    const pipeWidth = Math.max(2, w * 0.48);
    // Bronze/brass tones matching the in-game pipe icon
    const pipeBase = t.entity === 'pipe-to-ground' ? '#5a5540' : '#7a7558';
    const pipeHighlight = t.entity === 'pipe-to-ground' ? '#6a6550' : '#9a9575';
    const pipeShadow = '#2a2518';
    const pipeRim = t.entity === 'pipe-to-ground' ? '#484430' : '#605a40';

    ctx.lineCap = 'butt';

    if (neighbors.length === 0) {
      // Isolated pipe — circular flange
      ctx.fillStyle = pipeRim;
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.55, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = pipeBase;
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.42, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = pipeShadow;
      ctx.lineWidth = Math.max(1, pipeWidth * 0.08);
      ctx.stroke();
      // Specular highlight
      ctx.fillStyle = pipeHighlight;
      ctx.beginPath();
      ctx.arc(cx - pipeWidth * 0.12, cy - pipeWidth * 0.12, pipeWidth * 0.18, 0, Math.PI * 2);
      ctx.fill();
    } else {
      for (const nb of neighbors) {
        // Shadow outline
        ctx.strokeStyle = pipeShadow;
        ctx.lineWidth = pipeWidth + 3;
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        ctx.lineTo(cx + nb.dx * w / 2, cy + nb.dy * w / 2);
        ctx.stroke();
        // Pipe body
        ctx.strokeStyle = pipeBase;
        ctx.lineWidth = pipeWidth;
        ctx.beginPath();
        ctx.moveTo(cx, cy);
        ctx.lineTo(cx + nb.dx * w / 2, cy + nb.dy * w / 2);
        ctx.stroke();
        // Top highlight ridge
        ctx.strokeStyle = pipeHighlight;
        ctx.lineWidth = pipeWidth * 0.2;
        ctx.beginPath();
        const offX = nb.dy !== 0 ? -pipeWidth * 0.2 : 0;
        const offY = nb.dx !== 0 ? -pipeWidth * 0.2 : 0;
        ctx.moveTo(cx + offX, cy + offY);
        ctx.lineTo(cx + nb.dx * w / 2 + offX, cy + nb.dy * w / 2 + offY);
        ctx.stroke();
      }
    }

    // Junction cap with bolts
    if (neighbors.length >= 2) {
      ctx.fillStyle = pipeRim;
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.45, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = pipeBase;
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.35, 0, Math.PI * 2);
      ctx.fill();
      if (scale >= 10) {
        ctx.fillStyle = '#555040';
        const boltR = pipeWidth * 0.06;
        for (let a = 0; a < 4; a++) {
          const ba = a * Math.PI / 2;
          const br = pipeWidth * 0.3;
          ctx.beginPath();
          ctx.arc(cx + Math.cos(ba) * br, cy + Math.sin(ba) * br, boltR, 0, Math.PI * 2);
          ctx.fill();
        }
      }
    }

    // Underground pipe hole
    if (t.entity === 'pipe-to-ground') {
      ctx.fillStyle = '#0a0a08';
      ctx.beginPath();
      ctx.arc(cx, cy, pipeWidth * 0.2, 0, Math.PI * 2);
      ctx.fill();
      if (scale >= 8) {
        ctx.strokeStyle = '#3a3828';
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(cx - pipeWidth * 0.13, cy);
        ctx.lineTo(cx + pipeWidth * 0.13, cy);
        ctx.moveTo(cx, cy - pipeWidth * 0.13);
        ctx.lineTo(cx, cy + pipeWidth * 0.13);
        ctx.stroke();
      }
    }

    // Fluid hint glow
    if (scale >= 10 && t.entity === 'pipe') {
      ctx.fillStyle = 'rgba(100,160,80,0.12)';
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

    // Dark platform base
    ctx.fillStyle = '#333028';
    ctx.fillRect(px, py, w, w);

    if (scale >= 6) {
      ctx.fillStyle = '#3d3a30';
      const pad = w * 0.08;
      ctx.fillRect(px + pad, py + pad, w - pad * 2, w - pad * 2);
    }

    // Colors matching in-game icons: yellow inserter, blue fast, red long-handed
    const colors = {
      'inserter': '#b08a20',
      'fast-inserter': '#2878b8',
      'long-handed-inserter': '#b83030',
    };
    const highlightColors = {
      'inserter': '#d8b040',
      'fast-inserter': '#48a8e8',
      'long-handed-inserter': '#e04848',
    };
    const armColor = colors[t.entity] || '#b08a20';
    const armHighlight = highlightColors[t.entity] || '#d8b040';
    const angle = dirAngle(t.dir || 0);

    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(angle);

    // Mechanical base housing
    const baseSize = w * 0.24;
    ctx.fillStyle = '#2a2820';
    ctx.fillRect(-baseSize, w * 0.06, baseSize * 2, baseSize * 1.3);
    ctx.fillStyle = '#3a3830';
    ctx.fillRect(-baseSize + 1, w * 0.06 + 1, baseSize * 2 - 2, baseSize * 1.3 - 2);

    // Circular hub at pivot point (like the icon's round yellow/blue center)
    if (scale >= 6) {
      ctx.fillStyle = armColor;
      ctx.beginPath();
      ctx.arc(0, w * 0.12, w * 0.1, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = armHighlight;
      ctx.beginPath();
      ctx.arc(-w * 0.02, w * 0.1, w * 0.05, 0, Math.PI * 2);
      ctx.fill();
    }

    // Main arm
    ctx.strokeStyle = armColor;
    ctx.lineWidth = Math.max(2, w * 0.15);
    ctx.lineCap = 'round';
    ctx.beginPath();
    ctx.moveTo(0, w * 0.12);
    ctx.lineTo(0, -w * 0.32);
    ctx.stroke();
    // Arm highlight edge
    ctx.strokeStyle = armHighlight;
    ctx.lineWidth = Math.max(1, w * 0.05);
    ctx.beginPath();
    ctx.moveTo(w * 0.04, w * 0.12);
    ctx.lineTo(w * 0.04, -w * 0.32);
    ctx.stroke();

    // Grabber claw
    const clawY = -w * 0.32;
    const clawW = w * 0.2;
    ctx.strokeStyle = '#888070';
    ctx.lineWidth = Math.max(1.5, w * 0.1);
    ctx.beginPath();
    ctx.moveTo(-clawW, clawY - clawW * 0.5);
    ctx.lineTo(0, clawY + clawW * 0.1);
    ctx.lineTo(clawW, clawY - clawW * 0.5);
    ctx.stroke();

    // Joint dots
    if (scale >= 8) {
      ctx.fillStyle = '#666058';
      ctx.beginPath();
      ctx.arc(0, w * 0.12, w * 0.04, 0, Math.PI * 2);
      ctx.fill();
      ctx.beginPath();
      ctx.arc(0, -w * 0.32, w * 0.035, 0, Math.PI * 2);
      ctx.fill();
    }

    // Long-handed inserter extension
    if (t.entity === 'long-handed-inserter' && scale >= 6) {
      ctx.strokeStyle = armColor;
      ctx.lineWidth = Math.max(1.5, w * 0.11);
      ctx.beginPath();
      ctx.moveTo(0, -w * 0.32);
      ctx.lineTo(0, -w * 0.48);
      ctx.stroke();
      ctx.strokeStyle = armHighlight;
      ctx.lineWidth = Math.max(1, w * 0.04);
      ctx.beginPath();
      ctx.moveTo(w * 0.03, -w * 0.32);
      ctx.lineTo(w * 0.03, -w * 0.48);
      ctx.stroke();
      ctx.strokeStyle = '#888070';
      ctx.lineWidth = Math.max(1, w * 0.08);
      const clawY2 = -w * 0.48;
      ctx.beginPath();
      ctx.moveTo(-clawW * 0.8, clawY2 - clawW * 0.4);
      ctx.lineTo(0, clawY2 + clawW * 0.05);
      ctx.lineTo(clawW * 0.8, clawY2 - clawW * 0.4);
      ctx.stroke();
    }

    ctx.restore();

    // Item badge
    drawItemBadge(ctx, px, py, w, t.carries);
  },

  drawMachine(ctx, px, py, pw, ph, t) {
    const gap = scale >= 4 ? 1 : 0;
    const w = pw - gap;
    const h = ph - gap;
    const cx = px + w / 2;
    const cy = py + h / 2;

    // Dark iron frame
    ctx.fillStyle = '#222018';
    ctx.fillRect(px, py, w, h);

    const baseColor = t.color || '#888';
    // Machine body panel — tinted by recipe color
    ctx.fillStyle = darkenColor(baseColor, 0.4);
    const border = Math.max(2, w * 0.05);
    ctx.fillRect(px + border, py + border, w - border * 2, h - border * 2);

    if (scale >= 6) {
      // Inner panel (darker center area like the machine icons)
      ctx.fillStyle = darkenColor(baseColor, 0.25);
      const inset = Math.max(3, w * 0.12);
      ctx.fillRect(px + inset, py + inset, w - inset * 2, h - inset * 2);

      // Rivet/bolt details at corners
      if (scale >= 10) {
        ctx.fillStyle = '#5a5548';
        const boltR = Math.max(1.5, w * 0.022);
        const boltOff = inset * 0.65;
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

      // Top edge highlight (metallic sheen)
      ctx.strokeStyle = 'rgba(255,255,255,0.1)';
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(px + border, py + border);
      ctx.lineTo(px + w - border, py + border);
      ctx.stroke();
      // Bottom edge shadow
      ctx.strokeStyle = 'rgba(0,0,0,0.2)';
      ctx.beginPath();
      ctx.moveTo(px + border, py + h - border);
      ctx.lineTo(px + w - border, py + h - border);
      ctx.stroke();
    }

    if (scale >= 8) {
      ctx.save();
      ctx.translate(cx, cy);
      const iconSize = Math.min(w, h) * 0.35;

      if (t.entity === 'chemical-plant') {
        // Flask/distillation column silhouette matching the icon
        ctx.strokeStyle = 'rgba(180,210,240,0.55)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.12);
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
        // Twin input tubes
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.2, -iconSize * 0.8);
        ctx.lineTo(-iconSize * 0.2, -iconSize * 0.25);
        ctx.lineTo(-iconSize * 0.55, iconSize * 0.55);
        ctx.lineTo(iconSize * 0.55, iconSize * 0.55);
        ctx.lineTo(iconSize * 0.2, -iconSize * 0.25);
        ctx.lineTo(iconSize * 0.2, -iconSize * 0.8);
        ctx.stroke();
        // Fluid fill in the bulb
        ctx.fillStyle = 'rgba(40,180,120,0.3)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.4, iconSize * 0.25);
        ctx.lineTo(-iconSize * 0.55, iconSize * 0.55);
        ctx.lineTo(iconSize * 0.55, iconSize * 0.55);
        ctx.lineTo(iconSize * 0.4, iconSize * 0.25);
        ctx.fill();
        // Glass highlight
        ctx.strokeStyle = 'rgba(220,240,255,0.2)';
        ctx.lineWidth = Math.max(1, iconSize * 0.06);
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.12, -iconSize * 0.6);
        ctx.lineTo(-iconSize * 0.35, iconSize * 0.35);
        ctx.stroke();
      } else if (t.entity === 'oil-refinery') {
        // Stacked distillation towers like the icon
        ctx.fillStyle = 'rgba(180,160,120,0.2)';
        ctx.strokeStyle = 'rgba(200,180,140,0.45)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.08);
        ctx.lineJoin = 'round';
        for (let i = 0; i < 3; i++) {
          const ty = -iconSize * 0.65 + i * iconSize * 0.48;
          const tw = iconSize * (0.35 + i * 0.18);
          ctx.fillRect(-tw / 2, ty, tw, iconSize * 0.38);
          ctx.strokeRect(-tw / 2, ty, tw, iconSize * 0.38);
        }
        // Chimney stack
        ctx.strokeStyle = 'rgba(200,180,140,0.3)';
        ctx.lineWidth = Math.max(1, iconSize * 0.07);
        ctx.beginPath();
        ctx.moveTo(iconSize * 0.25, -iconSize * 0.65);
        ctx.lineTo(iconSize * 0.25, -iconSize * 0.95);
        ctx.stroke();
        // Pipe connection
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.15, -iconSize * 0.35);
        ctx.lineTo(-iconSize * 0.35, -iconSize * 0.5);
        ctx.stroke();
      } else {
        // Assembler gear icon — more prominent and metallic like the in-game icons
        const gearColor = 'rgba(190,185,170,0.6)';
        const gearFill = 'rgba(190,185,170,0.15)';
        ctx.strokeStyle = gearColor;
        ctx.fillStyle = gearFill;
        ctx.lineWidth = Math.max(2, iconSize * 0.13);
        const teeth = 8;
        const outerR = iconSize * 0.8;
        const innerR = iconSize * 0.55;
        ctx.beginPath();
        for (let i = 0; i < teeth; i++) {
          const a1 = (i / teeth) * Math.PI * 2;
          const a2 = ((i + 0.25) / teeth) * Math.PI * 2;
          const a3 = ((i + 0.5) / teeth) * Math.PI * 2;
          const a4 = ((i + 0.75) / teeth) * Math.PI * 2;
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
        // Center hub
        ctx.beginPath();
        ctx.arc(0, 0, innerR * 0.4, 0, Math.PI * 2);
        ctx.fillStyle = darkenColor(baseColor, 0.2);
        ctx.fill();
        ctx.strokeStyle = 'rgba(190,185,170,0.4)';
        ctx.lineWidth = Math.max(1, iconSize * 0.07);
        ctx.stroke();
        // Hub specular highlight
        if (scale >= 12) {
          ctx.fillStyle = 'rgba(255,255,240,0.12)';
          ctx.beginPath();
          ctx.arc(-innerR * 0.1, -innerR * 0.1, innerR * 0.2, 0, Math.PI * 2);
          ctx.fill();
        }
      }
      ctx.restore();
    }

    if (t.recipe && scale >= 14) {
      ctx.fillStyle = 'rgba(220,210,190,0.65)';
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

    // Wooden pole shaft (warm brown like the icon)
    const poleW = Math.max(2, w * 0.16);
    ctx.fillStyle = '#5a4428';
    ctx.fillRect(cx - poleW / 2, cy - w * 0.35, poleW, w * 0.7);
    // Wood grain highlight
    if (scale >= 8) {
      ctx.fillStyle = '#6a5438';
      ctx.fillRect(cx - poleW / 4, cy - w * 0.3, poleW / 3, w * 0.55);
    }

    // Cross-arm with copper wire connectors
    ctx.fillStyle = '#4a3820';
    const armLen = w * 0.36;
    const armW = Math.max(1.5, w * 0.12);
    ctx.fillRect(cx - armLen, cy - w * 0.22 - armW / 2, armLen * 2, armW);

    // Copper wire connection points (matching the gold/copper in the icon)
    if (scale >= 8) {
      ctx.fillStyle = '#c8a040';
      const dotR = Math.max(1.5, w * 0.05);
      ctx.beginPath();
      ctx.arc(cx - armLen + dotR * 0.8, cy - w * 0.22, dotR, 0, Math.PI * 2);
      ctx.fill();
      ctx.beginPath();
      ctx.arc(cx + armLen - dotR * 0.8, cy - w * 0.22, dotR, 0, Math.PI * 2);
      ctx.fill();
      // Top insulator
      ctx.fillStyle = '#a08030';
      ctx.beginPath();
      ctx.arc(cx, cy - w * 0.35, dotR * 0.8, 0, Math.PI * 2);
      ctx.fill();
    }

    // Coverage radius indicator
    if (scale >= 6) {
      ctx.strokeStyle = 'rgba(200,170,50,0.1)';
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
    // Heavy dark iron frame like the splitter icon
    ctx.fillStyle = '#282420';
    ctx.fillRect(px, py, w, h);
    const frame = Math.max(1.5, Math.min(w, h) * 0.1);

    // Belt tracks visible through each half (dark grey like belt tracks)
    const trackColor = '#484840';
    if (t.w > t.h) {
      ctx.fillStyle = trackColor;
      ctx.fillRect(px + frame, py + frame, w / 2 - frame - 1, h - frame * 2);
      ctx.fillRect(px + w / 2 + 1, py + frame, w / 2 - frame - 1, h - frame * 2);
    } else {
      ctx.fillStyle = trackColor;
      ctx.fillRect(px + frame, py + frame, w - frame * 2, h / 2 - frame - 1);
      ctx.fillRect(px + frame, py + h / 2 + 1, w - frame * 2, h / 2 - frame - 1);
    }

    if (scale >= 6) {
      // Center divider mechanism (heavy metal bar)
      ctx.fillStyle = '#3a3630';
      if (t.w > t.h) {
        ctx.fillRect(px + w / 2 - 2, py + frame * 0.5, 4, h - frame);
      } else {
        ctx.fillRect(px + frame * 0.5, py + h / 2 - 2, w - frame, 4);
      }

      // Direction chevrons on each half
      if (scale >= 8) {
        ctx.strokeStyle = '#d4a820';
        ctx.lineWidth = Math.max(1, Math.min(w, h) * 0.06);
        ctx.lineCap = 'round';
        const angle = dirAngle(t.dir || 0);
        ctx.save();
        if (t.w > t.h) {
          // Two chevrons, one per half
          const halfW = w / 4;
          const chevS = Math.min(w, h) * 0.12;
          for (let half = 0; half < 2; half++) {
            const hcx = px + halfW + half * w / 2;
            const hcy = py + h / 2;
            ctx.save();
            ctx.translate(hcx, hcy);
            ctx.rotate(angle);
            ctx.beginPath();
            ctx.moveTo(-chevS, chevS * 0.4);
            ctx.lineTo(0, -chevS * 0.4);
            ctx.lineTo(chevS, chevS * 0.4);
            ctx.stroke();
            ctx.restore();
          }
        } else {
          const halfH = h / 4;
          const chevS = Math.min(w, h) * 0.12;
          for (let half = 0; half < 2; half++) {
            const hcx = px + w / 2;
            const hcy = py + halfH + half * h / 2;
            ctx.save();
            ctx.translate(hcx, hcy);
            ctx.rotate(angle);
            ctx.beginPath();
            ctx.moveTo(-chevS, chevS * 0.4);
            ctx.lineTo(0, -chevS * 0.4);
            ctx.lineTo(chevS, chevS * 0.4);
            ctx.stroke();
            ctx.restore();
          }
        }
        ctx.restore();
      }
    }
  },
};
"""

# ---------------------------------------------------------------------------
# Combined: all themes + dispatch helper
# ---------------------------------------------------------------------------

THEME_JS = (
    UTILS_JS
    + SCHEMATIC_THEME_JS
    + FACTORIO_THEME_JS
    + r"""
// Theme dispatch
function getTheme() {
  return (typeof currentTheme !== 'undefined' && currentTheme === 'factorio') ? factorio : schematic;
}
"""
)
