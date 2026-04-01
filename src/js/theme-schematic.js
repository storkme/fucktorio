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
    const merge = !turn ? beltMergeInfo(t) : null;

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

        // Merge indicators: small inward arrows from each feeder side
        if (merge) {
          ctx.strokeStyle = chev;
          ctx.lineWidth = Math.max(1, s * 0.1);
          ctx.lineCap = 'round';
          const aSize = w * 0.18;
          for (const f of merge.feeders) {
            // f.dx/dy point from the tile toward the feeder, so the arrow comes from that side
            const ex = f.dx * w * 0.42;
            const ey = f.dy * w * 0.42;
            const ix = f.dx * w * 0.12;
            const iy = f.dy * w * 0.12;
            // Arrowhead pointing inward
            ctx.beginPath();
            ctx.moveTo(ex - f.dy * aSize * 0.5, ey - (-f.dx) * aSize * 0.5);
            ctx.lineTo(ix, iy);
            ctx.lineTo(ex + f.dy * aSize * 0.5, ey + (-f.dx) * aSize * 0.5);
            ctx.stroke();
          }
        }
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
    const gap = Math.max(1, scale * 0.08);
    const w = pw - gap * 2;
    const h = ph - gap * 2;
    px += gap;
    py += gap;
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
      } else if (isFurnace(t.entity)) {
        // Furnace: trapezoid body + flame
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineJoin = 'round';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(-iconSize * 0.35, -iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, -iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.5);
        ctx.closePath();
        ctx.stroke();
        // Flame
        ctx.fillStyle = 'rgba(255,160,40,0.5)';
        ctx.beginPath();
        ctx.moveTo(0, -iconSize * 0.25);
        ctx.quadraticCurveTo(iconSize * 0.25, iconSize * 0.15, 0, iconSize * 0.35);
        ctx.quadraticCurveTo(-iconSize * 0.25, iconSize * 0.15, 0, -iconSize * 0.25);
        ctx.fill();
      } else if (isBeacon(t.entity)) {
        // Beacon: broadcast/signal icon — concentric arcs radiating outward
        ctx.strokeStyle = 'rgba(120,180,255,0.6)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineCap = 'round';
        // Central dot
        ctx.fillStyle = 'rgba(120,180,255,0.7)';
        ctx.beginPath();
        ctx.arc(0, 0, iconSize * 0.12, 0, Math.PI * 2);
        ctx.fill();
        // Signal arcs
        for (let i = 1; i <= 3; i++) {
          const r = iconSize * 0.2 * i;
          ctx.globalAlpha = 0.6 - i * 0.12;
          ctx.beginPath();
          ctx.arc(0, 0, r, -Math.PI * 0.4, Math.PI * 0.4);
          ctx.stroke();
          ctx.beginPath();
          ctx.arc(0, 0, r, Math.PI * 0.6, Math.PI * 1.4);
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      } else if (t.entity === 'centrifuge') {
        // Spinning radial icon
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.arc(0, 0, iconSize * 0.5, 0, Math.PI * 2);
        ctx.stroke();
        for (let i = 0; i < 3; i++) {
          const a = (i / 3) * Math.PI * 2;
          ctx.beginPath();
          ctx.moveTo(0, 0);
          ctx.lineTo(Math.cos(a) * iconSize * 0.5, Math.sin(a) * iconSize * 0.5);
          ctx.stroke();
        }
      } else if (t.entity === 'lab' || t.entity === 'biolab') {
        // Science flask icon
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.1, -iconSize * 0.6);
        ctx.lineTo(-iconSize * 0.1, -iconSize * 0.15);
        ctx.lineTo(-iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.1, -iconSize * 0.15);
        ctx.lineTo(iconSize * 0.1, -iconSize * 0.6);
        ctx.stroke();
        ctx.fillStyle = t.entity === 'biolab' ? 'rgba(80,220,120,0.3)' : 'rgba(200,80,200,0.3)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.3, iconSize * 0.2);
        ctx.lineTo(-iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.3, iconSize * 0.2);
        ctx.fill();
      } else if (t.entity === 'storage-tank') {
        // Cylinder/tank icon
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.ellipse(0, -iconSize * 0.35, iconSize * 0.4, iconSize * 0.15, 0, 0, Math.PI * 2);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.4, -iconSize * 0.35);
        ctx.lineTo(-iconSize * 0.4, iconSize * 0.35);
        ctx.ellipse(0, iconSize * 0.35, iconSize * 0.4, iconSize * 0.15, 0, Math.PI, 0, true);
        ctx.lineTo(iconSize * 0.4, -iconSize * 0.35);
        ctx.stroke();
        ctx.fillStyle = 'rgba(80,160,220,0.2)';
        ctx.fillRect(-iconSize * 0.4, -iconSize * 0.1, iconSize * 0.8, iconSize * 0.6);
      } else if (t.entity === 'electric-mining-drill') {
        // Pickaxe icon
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.12);
        ctx.lineCap = 'round';
        ctx.beginPath();
        ctx.moveTo(iconSize * 0.3, -iconSize * 0.5);
        ctx.lineTo(-iconSize * 0.3, iconSize * 0.3);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.1, -iconSize * 0.3);
        ctx.lineTo(iconSize * 0.5, -iconSize * 0.3);
        ctx.lineTo(iconSize * 0.3, -iconSize * 0.5);
        ctx.stroke();
      } else if (t.entity === 'foundry') {
        // Crucible/molten icon
        ctx.strokeStyle = 'rgba(255,200,100,0.6)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineJoin = 'round';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.5, -iconSize * 0.3);
        ctx.lineTo(-iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, -iconSize * 0.3);
        ctx.stroke();
        ctx.fillStyle = 'rgba(255,140,20,0.4)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.4, iconSize * 0.1);
        ctx.lineTo(-iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.4, iconSize * 0.1);
        ctx.fill();
      } else if (t.entity === 'biochamber') {
        // Organic cell icon
        ctx.strokeStyle = 'rgba(80,220,80,0.6)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.arc(0, 0, iconSize * 0.5, 0, Math.PI * 2);
        ctx.stroke();
        ctx.fillStyle = 'rgba(80,220,80,0.2)';
        ctx.fill();
        ctx.fillStyle = 'rgba(80,220,80,0.4)';
        ctx.beginPath();
        ctx.arc(-iconSize * 0.15, -iconSize * 0.1, iconSize * 0.15, 0, Math.PI * 2);
        ctx.fill();
      } else if (t.entity === 'electromagnetic-plant') {
        // Lightning bolt icon
        ctx.fillStyle = 'rgba(120,180,255,0.6)';
        ctx.beginPath();
        ctx.moveTo(iconSize * 0.1, -iconSize * 0.6);
        ctx.lineTo(-iconSize * 0.25, iconSize * 0.05);
        ctx.lineTo(iconSize * 0.05, iconSize * 0.05);
        ctx.lineTo(-iconSize * 0.1, iconSize * 0.6);
        ctx.lineTo(iconSize * 0.25, -iconSize * 0.05);
        ctx.lineTo(-iconSize * 0.05, -iconSize * 0.05);
        ctx.closePath();
        ctx.fill();
      } else if (t.entity === 'cryogenic-plant') {
        // Snowflake/crystal icon
        ctx.strokeStyle = 'rgba(160,220,255,0.6)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineCap = 'round';
        for (let i = 0; i < 6; i++) {
          const a = (i / 6) * Math.PI * 2;
          ctx.beginPath();
          ctx.moveTo(0, 0);
          ctx.lineTo(Math.cos(a) * iconSize * 0.55, Math.sin(a) * iconSize * 0.55);
          ctx.stroke();
        }
        ctx.beginPath();
        ctx.arc(0, 0, iconSize * 0.15, 0, Math.PI * 2);
        ctx.fillStyle = 'rgba(160,220,255,0.3)';
        ctx.fill();
      } else if (t.entity === 'recycler') {
        // Recycle arrows icon
        ctx.strokeStyle = 'rgba(100,200,100,0.6)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.12);
        ctx.lineCap = 'round';
        for (let i = 0; i < 3; i++) {
          const a = (i / 3) * Math.PI * 2 - Math.PI / 2;
          const na = ((i + 1) / 3) * Math.PI * 2 - Math.PI / 2;
          const r = iconSize * 0.4;
          ctx.beginPath();
          ctx.arc(0, 0, r, a + 0.3, na - 0.3);
          ctx.stroke();
          // Arrowhead
          const tipA = na - 0.3;
          const tx = Math.cos(tipA) * r;
          const ty = Math.sin(tipA) * r;
          const aS = iconSize * 0.15;
          ctx.beginPath();
          ctx.moveTo(tx + Math.cos(tipA + 0.5) * aS, ty + Math.sin(tipA + 0.5) * aS);
          ctx.lineTo(tx, ty);
          ctx.lineTo(tx + Math.cos(tipA - 1.2) * aS, ty + Math.sin(tipA - 1.2) * aS);
          ctx.stroke();
        }
      } else if (t.entity === 'crusher') {
        // Crushing jaws icon
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.12);
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.5, -iconSize * 0.4);
        ctx.lineTo(0, iconSize * 0.1);
        ctx.lineTo(iconSize * 0.5, -iconSize * 0.4);
        ctx.stroke();
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.5, iconSize * 0.4);
        ctx.lineTo(0, -iconSize * 0.1);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.4);
        ctx.stroke();
      } else if (t.entity === 'rocket-silo') {
        // Rocket icon
        ctx.strokeStyle = 'rgba(255,255,255,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.moveTo(0, -iconSize * 0.7);
        ctx.lineTo(-iconSize * 0.2, iconSize * 0.3);
        ctx.lineTo(iconSize * 0.2, iconSize * 0.3);
        ctx.closePath();
        ctx.stroke();
        ctx.fillStyle = 'rgba(255,100,30,0.4)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.12, iconSize * 0.3);
        ctx.lineTo(0, iconSize * 0.6);
        ctx.lineTo(iconSize * 0.12, iconSize * 0.3);
        ctx.fill();
      } else if (isPowerPole(t.entity)) {
        // Power pole cross icon
        ctx.strokeStyle = 'rgba(200,180,50,0.6)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.15);
        ctx.lineCap = 'round';
        const arm = iconSize * 0.5;
        ctx.beginPath();
        ctx.moveTo(-arm, 0); ctx.lineTo(arm, 0);
        ctx.moveTo(0, -arm); ctx.lineTo(0, arm);
        ctx.stroke();
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

    // Beacon supply area radius (9x9 = 4.5 tiles from center)
    if (isBeacon(t.entity) && scale >= 4) {
      ctx.strokeStyle = 'rgba(120,180,255,0.15)';
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      const beaconR = 4.5 * scale;
      ctx.beginPath();
      ctx.rect(cx - beaconR, cy - beaconR, beaconR * 2, beaconR * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    if (t.recipe && scale >= 14) {
      const fontSize = Math.max(8, scale * 0.45);
      ctx.fillStyle = 'rgba(0,0,0,0.7)';
      ctx.font = 'bold ' + fontSize + 'px sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'bottom';
      const maxW = w * 0.9;
      const lines = wrapText(ctx, t.recipe, maxW, 3);
      const lineH = fontSize * 1.15;
      const baseY = py + h - Math.max(2, h * 0.05);
      for (let i = lines.length - 1; i >= 0; i--) {
        ctx.fillText(lines[i], cx, baseY - (lines.length - 1 - i) * lineH);
      }
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

  drawUnderground(ctx, px, py, s, t) {
    const gap = Math.max(1, scale * 0.08);
    const w = s - gap * 2;
    const cx = px + s / 2;
    const cy = py + s / 2;
    const beltColors = {
      'underground-belt': '#a89030',
      'fast-underground-belt': '#b03030',
      'express-underground-belt': '#3070b0',
    };
    const chevColors = {
      'underground-belt': '#e0d070',
      'fast-underground-belt': '#ff6060',
      'express-underground-belt': '#70b0f0',
    };
    const base = beltColors[t.entity] || '#a89030';
    const chev = chevColors[t.entity] || '#e0d070';
    const isInput = t.ioType === 'input';

    // Base tile with inset
    ctx.fillStyle = darkenColor(base, 0.5);
    ctx.fillRect(px + gap, py + gap, w, w);

    // Inner belt surface
    const frame = Math.max(1, w * 0.12);
    ctx.fillStyle = base;
    ctx.fillRect(px + gap + frame, py + gap + frame, w - frame * 2, w - frame * 2);

    // Dark center hole for underground entry/exit
    ctx.fillStyle = isInput ? 'rgba(0,0,0,0.6)' : 'rgba(0,0,0,0.35)';
    const holeR = w * 0.2;
    ctx.beginPath();
    ctx.arc(cx, cy, holeR, 0, Math.PI * 2);
    ctx.fill();

    if (scale >= 4) {
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(dirAngle(t.dir || 0));

      // Direction chevron — points inward for input, outward for output
      ctx.strokeStyle = chev;
      ctx.lineWidth = Math.max(1.5, s * 0.12);
      ctx.lineCap = 'round';
      const aSize = w * 0.22;
      const yOff = -w * 0.28;
      const yTip = -w * 0.08;
      ctx.beginPath();
      ctx.moveTo(-aSize, yOff);
      ctx.lineTo(0, yTip);
      ctx.lineTo(aSize, yOff);
      ctx.stroke();

      ctx.restore();
    }

    // Faint trace line to paired underground belt
    const pair = findUndergroundPair(t);
    if (pair) {
      const pairPx = (pair.x - t.x) * scale;
      const pairPy = (pair.y - t.y) * scale;
      ctx.save();
      ctx.strokeStyle = base;
      ctx.globalAlpha = 0.2;
      ctx.lineWidth = Math.max(2, w * 0.3);
      ctx.setLineDash([Math.max(2, scale * 0.15), Math.max(2, scale * 0.15)]);
      ctx.beginPath();
      ctx.moveTo(cx, cy);
      ctx.lineTo(cx + pairPx, cy + pairPy);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.restore();
    }
  },

  drawPump(ctx, px, py, pw, ph, t) {
    const gap = Math.max(1, scale * 0.08);
    const w = pw - gap * 2;
    const h = ph - gap * 2;
    px += gap;
    py += gap;
    const cx = px + w / 2;
    const cy = py + h / 2;

    // Base
    ctx.fillStyle = '#2a4a3a';
    ctx.fillRect(px, py, w, h);

    // Pipe sections
    ctx.fillStyle = '#5a9ad0';
    const pipeW = Math.min(w, h) * 0.3;
    ctx.fillRect(cx - pipeW / 2, py, pipeW, h);

    // Direction arrow
    if (scale >= 4) {
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(dirAngle(t.dir || 0));
      ctx.fillStyle = '#90d0ff';
      const aSize = Math.min(w, h) * 0.2;
      ctx.beginPath();
      ctx.moveTo(0, -aSize);
      ctx.lineTo(aSize * 0.7, aSize * 0.3);
      ctx.lineTo(-aSize * 0.7, aSize * 0.3);
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    }
  },

  drawSplitter(ctx, px, py, pw, ph, t) {
    const gap = Math.max(1, scale * 0.08);
    pw -= gap * 2;
    ph -= gap * 2;
    px += gap;
    py += gap;
    const w = pw;
    const h = ph;
    const splitterColors = {
      'splitter': '#a89030',
      'fast-splitter': '#b03030',
      'express-splitter': '#3070b0',
    };
    ctx.fillStyle = splitterColors[t.entity] || '#a89030';
    ctx.fillRect(px, py, w, h);
    if (scale >= 6) {
      ctx.strokeStyle = 'rgba(0,0,0,0.3)';
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
