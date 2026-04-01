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
    const merge = !turn ? beltMergeInfo(t) : null;
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

        // Merge indicators: inward arrows from each feeder side
        if (merge) {
          ctx.strokeStyle = arrow;
          ctx.lineWidth = Math.max(1, s * 0.1);
          ctx.lineCap = 'round';
          const aSize = w * 0.18;
          for (const f of merge.feeders) {
            const ex = f.dx * w * 0.42;
            const ey = f.dy * w * 0.42;
            const ix = f.dx * w * 0.12;
            const iy = f.dy * w * 0.12;
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
    const gap = Math.max(1, scale * 0.08);
    const w = pw - gap * 2;
    const h = ph - gap * 2;
    px += gap;
    py += gap;
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
      } else if (isFurnace(t.entity)) {
        // Furnace: industrial trapezoid + flame glow
        ctx.strokeStyle = 'rgba(200,160,100,0.55)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.12);
        ctx.lineJoin = 'round';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.5, iconSize * 0.5);
        ctx.lineTo(-iconSize * 0.35, -iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, -iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, iconSize * 0.5);
        ctx.closePath();
        ctx.fillStyle = 'rgba(180,120,60,0.15)';
        ctx.fill();
        ctx.stroke();
        // Flame glow
        ctx.fillStyle = 'rgba(255,120,20,0.35)';
        ctx.beginPath();
        ctx.moveTo(0, -iconSize * 0.2);
        ctx.quadraticCurveTo(iconSize * 0.2, iconSize * 0.15, 0, iconSize * 0.35);
        ctx.quadraticCurveTo(-iconSize * 0.2, iconSize * 0.15, 0, -iconSize * 0.2);
        ctx.fill();
      } else if (isBeacon(t.entity)) {
        // Beacon: broadcast signal arcs
        ctx.strokeStyle = 'rgba(80,140,220,0.55)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineCap = 'round';
        // Central dot
        ctx.fillStyle = 'rgba(80,140,220,0.6)';
        ctx.beginPath();
        ctx.arc(0, 0, iconSize * 0.12, 0, Math.PI * 2);
        ctx.fill();
        // Signal arcs
        for (let i = 1; i <= 3; i++) {
          const r = iconSize * 0.2 * i;
          ctx.globalAlpha = 0.5 - i * 0.1;
          ctx.beginPath();
          ctx.arc(0, 0, r, -Math.PI * 0.4, Math.PI * 0.4);
          ctx.stroke();
          ctx.beginPath();
          ctx.arc(0, 0, r, Math.PI * 0.6, Math.PI * 1.4);
          ctx.stroke();
        }
        ctx.globalAlpha = 1;
      } else if (t.entity === 'centrifuge') {
        ctx.strokeStyle = 'rgba(190,185,170,0.5)';
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
        ctx.strokeStyle = 'rgba(190,185,170,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.1, -iconSize * 0.6);
        ctx.lineTo(-iconSize * 0.1, -iconSize * 0.15);
        ctx.lineTo(-iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.1, -iconSize * 0.15);
        ctx.lineTo(iconSize * 0.1, -iconSize * 0.6);
        ctx.stroke();
        ctx.fillStyle = t.entity === 'biolab' ? 'rgba(60,180,100,0.25)' : 'rgba(160,60,160,0.25)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.3, iconSize * 0.2);
        ctx.lineTo(-iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.45, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.3, iconSize * 0.2);
        ctx.fill();
      } else if (t.entity === 'storage-tank') {
        ctx.strokeStyle = 'rgba(190,185,170,0.5)';
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
        ctx.fillStyle = 'rgba(40,120,180,0.15)';
        ctx.fillRect(-iconSize * 0.4, -iconSize * 0.1, iconSize * 0.8, iconSize * 0.6);
      } else if (t.entity === 'electric-mining-drill') {
        ctx.strokeStyle = 'rgba(190,185,170,0.5)';
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
        ctx.strokeStyle = 'rgba(220,180,80,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.lineJoin = 'round';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.5, -iconSize * 0.3);
        ctx.lineTo(-iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.5, -iconSize * 0.3);
        ctx.stroke();
        ctx.fillStyle = 'rgba(255,120,20,0.3)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.4, iconSize * 0.1);
        ctx.lineTo(-iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.35, iconSize * 0.5);
        ctx.lineTo(iconSize * 0.4, iconSize * 0.1);
        ctx.fill();
      } else if (t.entity === 'biochamber') {
        ctx.strokeStyle = 'rgba(60,180,60,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.arc(0, 0, iconSize * 0.5, 0, Math.PI * 2);
        ctx.stroke();
        ctx.fillStyle = 'rgba(60,180,60,0.15)';
        ctx.fill();
        ctx.fillStyle = 'rgba(60,180,60,0.3)';
        ctx.beginPath();
        ctx.arc(-iconSize * 0.15, -iconSize * 0.1, iconSize * 0.15, 0, Math.PI * 2);
        ctx.fill();
      } else if (t.entity === 'electromagnetic-plant') {
        ctx.fillStyle = 'rgba(80,140,220,0.5)';
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
        ctx.strokeStyle = 'rgba(130,190,230,0.5)';
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
        ctx.fillStyle = 'rgba(130,190,230,0.2)';
        ctx.fill();
      } else if (t.entity === 'recycler') {
        ctx.strokeStyle = 'rgba(80,170,80,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.12);
        ctx.lineCap = 'round';
        for (let i = 0; i < 3; i++) {
          const a = (i / 3) * Math.PI * 2 - Math.PI / 2;
          const na = ((i + 1) / 3) * Math.PI * 2 - Math.PI / 2;
          const r = iconSize * 0.4;
          ctx.beginPath();
          ctx.arc(0, 0, r, a + 0.3, na - 0.3);
          ctx.stroke();
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
        ctx.strokeStyle = 'rgba(190,185,170,0.5)';
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
        ctx.strokeStyle = 'rgba(190,185,170,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.1);
        ctx.beginPath();
        ctx.moveTo(0, -iconSize * 0.7);
        ctx.lineTo(-iconSize * 0.2, iconSize * 0.3);
        ctx.lineTo(iconSize * 0.2, iconSize * 0.3);
        ctx.closePath();
        ctx.stroke();
        ctx.fillStyle = 'rgba(255,80,20,0.3)';
        ctx.beginPath();
        ctx.moveTo(-iconSize * 0.12, iconSize * 0.3);
        ctx.lineTo(0, iconSize * 0.6);
        ctx.lineTo(iconSize * 0.12, iconSize * 0.3);
        ctx.fill();
      } else if (isPowerPole(t.entity)) {
        ctx.strokeStyle = 'rgba(180,160,40,0.5)';
        ctx.lineWidth = Math.max(1.5, iconSize * 0.15);
        ctx.lineCap = 'round';
        const arm = iconSize * 0.5;
        ctx.beginPath();
        ctx.moveTo(-arm, 0); ctx.lineTo(arm, 0);
        ctx.moveTo(0, -arm); ctx.lineTo(0, arm);
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

    // Beacon supply area radius (9x9 = 4.5 tiles from center)
    if (isBeacon(t.entity) && scale >= 4) {
      ctx.strokeStyle = 'rgba(80,140,220,0.12)';
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      const beaconR = 4.5 * scale;
      ctx.beginPath();
      ctx.rect(cx - beaconR, cy - beaconR, beaconR * 2, beaconR * 2);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    if (t.recipe && scale >= 14) {
      const fontSize = Math.max(8, scale * 0.42);
      ctx.fillStyle = 'rgba(220,210,190,0.65)';
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

  drawUnderground(ctx, px, py, s, t) {
    const gap = Math.max(1, scale * 0.08);
    const w = s - gap * 2;
    const cx = px + s / 2;
    const cy = py + s / 2;
    const baseColors = {
      'underground-belt': '#484840',
      'fast-underground-belt': '#443838',
      'express-underground-belt': '#383844',
    };
    const arrowColors = {
      'underground-belt': '#d4a820',
      'fast-underground-belt': '#cc3030',
      'express-underground-belt': '#3080cc',
    };
    const base = baseColors[t.entity] || '#484840';
    const arrow = arrowColors[t.entity] || '#d4a820';
    const isInput = t.ioType === 'input';

    // Dark metallic frame with inset
    ctx.fillStyle = '#1a1810';
    ctx.fillRect(px + gap, py + gap, w, w);

    // Inner belt surface
    const frame = Math.max(1, w * 0.12);
    ctx.fillStyle = base;
    ctx.fillRect(px + gap + frame, py + gap + frame, w - frame * 2, w - frame * 2);

    // Dark underground hole
    ctx.fillStyle = isInput ? 'rgba(0,0,0,0.7)' : 'rgba(0,0,0,0.4)';
    const holeR = w * 0.2;
    ctx.beginPath();
    ctx.arc(cx, cy, holeR, 0, Math.PI * 2);
    ctx.fill();

    // Rim around hole
    if (scale >= 6) {
      ctx.strokeStyle = 'rgba(100,95,80,0.5)';
      ctx.lineWidth = Math.max(1, w * 0.06);
      ctx.beginPath();
      ctx.arc(cx, cy, holeR, 0, Math.PI * 2);
      ctx.stroke();
    }

    if (scale >= 4) {
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(dirAngle(t.dir || 0));

      // Direction chevron
      ctx.strokeStyle = arrow;
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
      ctx.strokeStyle = arrow;
      ctx.globalAlpha = 0.15;
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

    // Dark metallic base
    ctx.fillStyle = '#1a2a24';
    ctx.fillRect(px, py, w, h);

    // Pipe body
    const pipeW = Math.min(w, h) * 0.35;
    ctx.fillStyle = '#5a5540';
    ctx.fillRect(cx - pipeW / 2, py, pipeW, h);

    // Highlight ridge
    ctx.fillStyle = '#7a7558';
    ctx.fillRect(cx - pipeW / 4, py, pipeW / 3, h);

    // Direction arrow
    if (scale >= 4) {
      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(dirAngle(t.dir || 0));
      ctx.fillStyle = '#90c0a0';
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
