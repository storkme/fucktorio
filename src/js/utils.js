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
function isFurnace(name) {
  return name === 'stone-furnace' || name === 'steel-furnace'
    || name === 'electric-furnace';
}
function isBeacon(name) { return name === 'beacon'; }
function isSplitter(name) {
  return name === 'splitter' || name === 'fast-splitter'
    || name === 'express-splitter';
}
function isPump(name) { return name === 'pump'; }
function isStorageTank(name) { return name === 'storage-tank'; }
function isMiningDrill(name) { return name === 'electric-mining-drill'; }
function isPowerPole(name) {
  return name === 'medium-electric-pole' || name === 'big-electric-pole'
    || name === 'substation';
}
function isUnderground(name) {
  return name === 'underground-belt' || name === 'fast-underground-belt'
    || name === 'express-underground-belt';
}

// Find the paired underground belt for a given one (scan along its direction)
function findUndergroundPair(t) {
  const d = t.dir || 0;
  const dx = dirDx(d);
  const dy = dirDy(d);
  // input faces the direction it swallows from; output faces the direction it spits to
  // Scan forward (in belt direction) for input, backward for output
  const isInput = t.ioType === 'input';
  const scanDx = isInput ? dx : -dx;
  const scanDy = isInput ? dy : -dy;
  // Underground belts can span up to 4 tiles apart (5 tiles gap) in vanilla
  for (let dist = 1; dist <= 6; dist++) {
    const nx = t.x + scanDx * dist;
    const ny = t.y + scanDy * dist;
    const nb = tileMap[nx + ',' + ny];
    if (nb && isUnderground(nb.entity) && nb.ioType && nb.ioType !== t.ioType) {
      const nd = nb.dir || 0;
      if (nd === d) return { x: nx, y: ny, dist };
    }
  }
  return null;
}

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

function beltMergeInfo(t) {
  // Detect double-sideload merge: two perpendicular feeders from opposite sides,
  // no straight feeder from behind. Returns { feeders: [{dx,dy}, ...] } or null.
  const d = t.dir || 0;
  const dirs = [[0,-1],[1,0],[0,1],[-1,0]];
  let hasStraightFeeder = false;
  const perpFeeders = [];
  for (const [dx,dy] of dirs) {
    const nb = tileMap[(t.x+dx)+','+(t.y+dy)];
    if (!nb || !isBelt(nb.entity)) continue;
    const nd = nb.dir || 0;
    if (nb.x + dirDx(nd) !== t.x || nb.y + dirDy(nd) !== t.y) continue;
    if (nd === d) {
      hasStraightFeeder = true;
    } else {
      const cross = dirDx(nd) * dirDy(d) - dirDy(nd) * dirDx(d);
      if (cross !== 0) perpFeeders.push({dx, dy});
    }
  }
  if (perpFeeders.length === 2 && !hasStraightFeeder) return { feeders: perpFeeders };
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

function wrapText(ctx, text, maxWidth, maxLines) {
  // Split on hyphens (Factorio recipe names use hyphens as word separators)
  const words = text.split('-');
  const lines = [];
  let current = words[0] || '';
  for (let i = 1; i < words.length; i++) {
    const test = current + '-' + words[i];
    if (ctx.measureText(test).width <= maxWidth) {
      current = test;
    } else {
      lines.push(current);
      current = words[i];
      if (lines.length >= maxLines - 1) {
        // Remaining words go on last line
        current = words.slice(i).join('-');
        break;
      }
    }
  }
  if (current.length > 0) {
    if (ctx.measureText(current).width > maxWidth) {
      while (current.length > 1 && ctx.measureText(current + '\u2026').width > maxWidth) {
        current = current.slice(0, -1);
      }
      current += '\u2026';
    }
    lines.push(current);
  }
  return lines;
}

// Per-lane capacity for throughput utilization coloring
const LANE_CAPACITY = {
  'transport-belt': 7.5,
  'fast-transport-belt': 15.0,
  'express-transport-belt': 22.5,
};

/**
 * Draw throughput overlay on a belt tile — two colored lane strips showing utilization.
 * Called as a second pass after normal belt drawing when the overlay is active.
 */
function drawThroughputOverlay(ctx, px, py, scale, t, rates) {
  if (!rates) return;
  const s = scale;
  const cap = LANE_CAPACITY[t.entity] || 7.5;

  function utilizationColor(rate, capacity) {
    if (rate <= 0) return null;
    const pct = Math.min(rate / capacity, 1.5);
    if (pct <= 0.5) {
      // Green
      const g = Math.round(180 + pct * 150);
      return `rgba(40,${g},40,0.45)`;
    } else if (pct <= 0.8) {
      // Yellow
      const frac = (pct - 0.5) / 0.3;
      const r = Math.round(200 + frac * 55);
      const g = Math.round(220 - frac * 80);
      return `rgba(${r},${g},40,0.5)`;
    } else {
      // Red
      const frac = Math.min((pct - 0.8) / 0.4, 1);
      return `rgba(${Math.round(220 + frac * 35)},${Math.round(60 - frac * 30)},30,0.55)`;
    }
  }

  const leftColor = utilizationColor(rates.left, cap);
  const rightColor = utilizationColor(rates.right, cap);
  if (!leftColor && !rightColor) return;

  ctx.save();
  const cx = px + s * 0.5;
  const cy = py + s * 0.5;
  ctx.translate(cx, cy);
  ctx.rotate(dirAngle(t.dir || 0));

  const hw = s * 0.5;
  // Left lane = left half when facing belt direction (negative x in rotated space)
  if (leftColor) {
    ctx.fillStyle = leftColor;
    ctx.fillRect(-hw, -hw, hw, s);
  }
  // Right lane = right half (positive x in rotated space)
  if (rightColor) {
    ctx.fillStyle = rightColor;
    ctx.fillRect(0, -hw, hw, s);
  }

  // Rate text at high zoom
  if (scale >= 12) {
    ctx.fillStyle = '#fff';
    ctx.font = `bold ${Math.max(8, s * 0.22)}px monospace`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    if (rates.left > 0) {
      ctx.fillText(rates.left.toFixed(1), -hw * 0.5, 0);
    }
    if (rates.right > 0) {
      ctx.fillText(rates.right.toFixed(1), hw * 0.5, 0);
    }
  }

  ctx.restore();
}
