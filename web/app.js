const ALLOWED_POOLS = [
  '0xb2cc224c1c9fee385f8ad6a55b4d94e92359dc59',
  '0xd0b53d9277642d899df5c87a3966a349a798f224',
  '0x72ab388e2e2f6facef59e3c3fa2c4e29011c2d38',
];

const state = {
  snapshot: null,
  selected: new Set(),
  venueIndex: {},
  scaleMode: 'auto',
};

// Per-venue color mapping: each venue gets a unique hue, bid/ask use green/red tint
const VENUE_COLORS = {
  uniswap_v3:    { bid: '#22c55e', ask: '#ef4444' },
  pancake_v3:    { bid: '#10b981', ask: '#f97316' },
  slipstream:    { bid: '#14b8a6', ask: '#fb923c' },
  pancake_v2:    { bid: '#06b6d4', ask: '#e879f9' },
  aerodrome_v2:  { bid: '#3b82f6', ask: '#f472b6' },
  unified:       { bid: '#f59e0b', ask: '#fbbf24' },
  unified_kyber: { bid: '#d97706', ask: '#fcd34d' },
  kyber:         { bid: '#06b6d4', ask: '#67e8f9' },
  tessera:       { bid: '#a855f7', ask: '#c084fc' },
};

const FALLBACK_BID = '#22c55e';
const FALLBACK_ASK = '#ef4444';

function venueColor(venue, side) {
  const entry = VENUE_COLORS[venue];
  if (entry) return side === 'bid' ? entry.bid : entry.ask;
  return side === 'bid' ? FALLBACK_BID : FALLBACK_ASK;
}

function fmt(value, decimals = 4) {
  if (value == null || Number.isNaN(value)) return '\u2014';
  return Number(value).toLocaleString(undefined, { maximumFractionDigits: decimals });
}

function venueLabel(venue) {
  const labels = {
    uniswap_v3: 'Uni V3',
    pancake_v3: 'Cake V3',
    pancake_v2: 'Cake V2',
    slipstream: 'Slip',
    aerodrome_v2: 'Aero V2',
    unified: 'Unified',
    unified_kyber: 'Uni+Kyber',
    kyber: 'Kyber',
    tessera: 'Tessera',
  };
  return labels[venue] || venue.replace(/_/g, ' ');
}

function shortAddr(addr) {
  if (!addr) return '';
  const s = addr.toLowerCase();
  return s.slice(0, 6) + '\u2026' + s.slice(-4);
}

function extractAddr(value) {
  if (!value) return null;
  const m = String(value).match(/0x[a-fA-F0-9]{40}/);
  return m ? m[0] : null;
}

function poolLabel(curve, short = false) {
  if (!curve || !curve.pool) return '';
  let label = String(curve.pool);
  const prefix = curve.venue + ':';
  if (label.startsWith(prefix)) label = label.slice(prefix.length);
  if (short) {
    const addr = extractAddr(label);
    if (addr) label = label.replace(addr, shortAddr(addr));
  }
  return label.split(':').join(' / ');
}

// ── Status & Clock ──

function updateStatus(text, stateValue = 'loading') {
  const el = document.getElementById('status');
  if (el) { el.textContent = text; el.dataset.state = stateValue; }
}

function updateClock(text) {
  const el = document.getElementById('snapshot-clock');
  if (el) el.textContent = text;
}

// ── Data fetching ──

async function fetchLatest() {
  const res = await fetch('/api/snapshots/latest');
  if (!res.ok) throw new Error('No snapshot yet');
  return res.json();
}

function connectWs() {
  const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${protocol}://${location.host}/ws`);
  ws.onopen = () => updateStatus('Live', 'ok');
  ws.onmessage = (event) => {
    try { applySnapshot(JSON.parse(event.data)); }
    catch (err) { console.error(err); }
  };
  ws.onclose = () => {
    updateStatus('Reconnecting', 'error');
    setTimeout(connectWs, 3000);
  };
}

// ── Snapshot application ──

function applySnapshot(snapshot) {
  state.snapshot = snapshot;
  const venues = [...new Set(snapshot.curves.map((c) => c.venue))];

  if (state.selected.size === 0) {
    if (venues.length <= 5) {
      venues.forEach((v) => state.selected.add(v));
    } else {
      const defaults = ['unified', 'kyber', 'tessera'];
      const picked = defaults.filter((v) => venues.includes(v));
      (picked.length > 0 ? picked : venues).forEach((v) => state.selected.add(v));
    }
  }

  state.venueIndex = Object.fromEntries(venues.map((v, i) => [v, i]));

  updateMeta(snapshot.meta);
  renderVenueChips(venues);
  renderChart(snapshot);
  renderStats(snapshot);
  renderDepthTable(snapshot);
  renderPoolList(snapshot);
}

// ── Meta strip ──

function updateMeta(meta) {
  const el = document.getElementById('snapshot-meta');
  const pairLabel = document.getElementById('pair-label');
  if (!meta) return;

  const pair = `${meta.pair.base.symbol}/${meta.pair.quote.symbol}`;
  const time = new Date(meta.snapshot_time).toLocaleTimeString();

  if (pairLabel) pairLabel.textContent = pair;
  updateClock(time);

  if (el) {
    const items = [
      ['Block', fmt(meta.block_number, 0)],
      ['Venues', meta.venues.length],
      ['Steps', meta.config?.steps ?? '\u2014'],
      ['Max', meta.config?.max_base ?? '\u2014'],
    ];
    el.innerHTML = items.map(([label, val]) =>
      `<span class="meta-item"><span class="meta-label">${label}</span> <span class="meta-val">${val}</span></span>`
    ).join('');
  }
}

// ── Venue chips ──

function renderVenueChips(venues) {
  const container = document.getElementById('venue-toggle');
  if (!container) return;
  container.innerHTML = '';
  venues.forEach((venue) => {
    const btn = document.createElement('button');
    const activeClass = state.selected.has(venue) ? ' active' : '';
    // Add venue-specific class for special styling (tessera, kyber, unified)
    btn.className = `venue-chip ${venue}${activeClass}`;
    btn.textContent = venueLabel(venue);
    btn.addEventListener('click', () => {
      state.selected.has(venue) ? state.selected.delete(venue) : state.selected.add(venue);
      renderVenueChips(venues);
      renderChart(state.snapshot);
      renderStats(state.snapshot);
      renderDepthTable(state.snapshot);
    });
    container.appendChild(btn);
  });
}

// ── Intelligent Chart Scaling ──

function computeRange(curves, mode) {
  if (!curves.length) return null;

  // Gather all prices across visible curves
  const allPrices = [];
  curves.forEach((c) => c.points.forEach((p) => {
    if (p.price != null && isFinite(p.price)) allPrices.push(p.price);
  }));
  if (allPrices.length < 2) return null;

  allPrices.sort((a, b) => a - b);

  // Compute IQR-based range to handle outliers intelligently
  const q1Idx = Math.floor(allPrices.length * 0.1);
  const q3Idx = Math.floor(allPrices.length * 0.9);
  const q1 = allPrices[q1Idx];
  const q3 = allPrices[q3Idx];
  const iqr = q3 - q1;
  const median = allPrices[Math.floor(allPrices.length * 0.5)];

  // Filter to "inlier" prices (within 2x IQR of Q1/Q3)
  const inlierLo = q1 - 2 * iqr;
  const inlierHi = q3 + 2 * iqr;
  const inliers = allPrices.filter((p) => p >= inlierLo && p <= inlierHi);
  const dataMin = inliers.length ? inliers[0] : allPrices[0];
  const dataMax = inliers.length ? inliers[inliers.length - 1] : allPrices[allPrices.length - 1];

  if (mode === 'tight') {
    // Focus on the bid/ask spread area: tighter than auto
    const spreadRange = dataMax - dataMin;
    const pad = spreadRange * 0.15;
    return [dataMin - pad, dataMax + pad];
  }

  if (mode === 'wide') {
    // Show all data including outliers
    const fullMin = allPrices[0];
    const fullMax = allPrices[allPrices.length - 1];
    const fullRange = fullMax - fullMin;
    const pad = fullRange * 0.1;
    return [fullMin - pad, fullMax + pad];
  }

  // Auto mode: IQR-adaptive scaling
  // Use the inlier range as base, with proportional padding
  const range = dataMax - dataMin;
  if (range === 0) {
    // All prices identical - use +/- 0.5% of median
    const halfPct = median * 0.005;
    return [median - halfPct, median + halfPct];
  }

  // Padding scales with the data spread relative to median
  // Narrow spreads (like 0.1%) get more relative padding for visibility
  // Wide spreads (like 5%) get less padding to avoid wasted space
  const spreadPct = range / median;
  const padFactor = spreadPct < 0.005 ? 0.5 : spreadPct < 0.02 ? 0.3 : 0.15;
  const pad = range * padFactor;
  return [dataMin - pad, dataMax + pad];
}

// ── Chart rendering ──

function renderChart(snapshot) {
  if (!snapshot) return;
  const curves = snapshot.curves.filter((c) => state.selected.has(c.venue));
  if (curves.length === 0) {
    Plotly.purge('chart');
    return;
  }

  const traces = curves.map((curve) => {
    const color = venueColor(curve.venue, curve.side);
    const vName = venueLabel(curve.venue);
    const pShort = poolLabel(curve, true);
    const pFull = poolLabel(curve) || pShort || vName;
    const parts = [vName, curve.side.toUpperCase()];
    if (pShort && pShort.toUpperCase() !== vName.toUpperCase()) parts.push(pShort);

    return {
      x: curve.points.map((p) => p.price),
      y: curve.points.map((p) => p.cum_base),
      type: 'scatter',
      mode: 'lines',
      line: {
        color,
        width: curve.side === 'bid' ? 1.8 : 1.8,
        shape: 'hv',
        dash: curve.venue === 'tessera' ? 'dash' : 'solid',
      },
      name: parts.join(' \u00b7 '),
      hovertemplate:
        `<b>${vName}</b> ${curve.side}<br>` +
        `Pool: ${pFull}<br>` +
        `Price: %{x:.6f}<br>` +
        `Depth: %{y:.4f}<extra></extra>`,
    };
  });

  // Mid-price annotation
  const { bestBid, bestAsk, maxDepth } = bestPrices(curves);
  const shapes = [];
  const annotations = [];
  if (bestBid && bestAsk) {
    const mid = (bestBid + bestAsk) / 2;
    shapes.push({
      type: 'line',
      x0: mid, x1: mid,
      y0: 0, y1: maxDepth,
      line: { color: 'rgba(255,255,255,0.15)', width: 1, dash: 'dot' },
    });
    annotations.push({
      x: mid, y: maxDepth,
      text: `Mid ${fmt(mid, 2)}`,
      showarrow: false,
      font: { size: 10, color: 'rgba(255,255,255,0.4)', family: 'JetBrains Mono' },
      yshift: 10,
    });
  }

  const range = computeRange(curves, state.scaleMode);

  const layout = {
    margin: { l: 56, r: 16, t: 8, b: 44 },
    xaxis: {
      title: { text: 'Price', font: { size: 11, color: '#4b5563' } },
      gridcolor: 'rgba(255,255,255,0.04)',
      zerolinecolor: 'rgba(255,255,255,0.06)',
      range,
      tickfont: { family: 'JetBrains Mono, monospace', size: 10, color: '#6b7280' },
      tickformat: ',.2f',
    },
    yaxis: {
      title: { text: 'Cumulative Base', font: { size: 11, color: '#4b5563' } },
      gridcolor: 'rgba(255,255,255,0.04)',
      zerolinecolor: 'rgba(255,255,255,0.06)',
      tickfont: { family: 'JetBrains Mono, monospace', size: 10, color: '#6b7280' },
    },
    font: { family: 'Inter, sans-serif', color: '#e1e4e8' },
    paper_bgcolor: 'rgba(0,0,0,0)',
    plot_bgcolor: '#0f1218',
    legend: {
      orientation: 'h',
      y: -0.15,
      x: 0.5,
      xanchor: 'center',
      font: { size: 10, color: '#6b7280' },
      bgcolor: 'rgba(0,0,0,0)',
    },
    shapes,
    annotations,
    hovermode: 'closest',
    hoverlabel: {
      bgcolor: '#1a1f2a',
      bordercolor: 'rgba(255,255,255,0.1)',
      font: { family: 'JetBrains Mono', size: 11, color: '#e1e4e8' },
    },
  };

  Plotly.react('chart', traces, layout, {
    displayModeBar: false,
    responsive: true,
  });
}

// Aggregator curves are a benchmark, not a venue: they route through the same
// pools and are sampled off-block, so they are left out of BBO and ladder stats.
const AGGREGATOR_VENUES = new Set(['kyber', 'unified_kyber']);

function venueCurves(curves) {
  return curves.filter((c) => !AGGREGATOR_VENUES.has(c.venue));
}

function bestPrices(curves) {
  let bestBid = null, bestAsk = null, maxDepth = 0;
  venueCurves(curves).forEach((curve) => {
    if (!curve.points.length) return;
    const first = curve.points[0];
    maxDepth = Math.max(maxDepth, curve.points[curve.points.length - 1].cum_base);
    if (curve.side === 'bid') {
      bestBid = bestBid === null ? first.price : Math.max(bestBid, first.price);
    } else {
      bestAsk = bestAsk === null ? first.price : Math.min(bestAsk, first.price);
    }
  });
  return { bestBid, bestAsk, maxDepth };
}

// ── Stats bar ──

function renderStats(snapshot) {
  const el = document.getElementById('market-stats');
  if (!el || !snapshot) return;
  const curves = snapshot.curves.filter((c) => state.selected.has(c.venue));
  const { bestBid, bestAsk } = bestPrices(curves);
  const mid = bestBid && bestAsk ? (bestBid + bestAsk) / 2 : null;
  const spreadBps = bestBid && bestAsk ? ((bestAsk - bestBid) / mid) * 10000 : null;

  el.innerHTML = [
    stat('Best Bid', fmt(bestBid, 6), 'bid'),
    stat('Best Ask', fmt(bestAsk, 6), 'ask'),
    stat('Mid', fmt(mid, 6), 'mid'),
    stat('Spread', spreadBps != null ? fmt(spreadBps, 2) + ' bps' : '\u2014', 'spread'),
  ].join('');
}

function stat(label, value, cls) {
  return `<div class="stat-item"><span class="stat-label">${label}</span><span class="stat-val ${cls}">${value}</span></div>`;
}

// ── Depth ladder ──

function renderDepthTable(snapshot) {
  const table = document.getElementById('depth-table');
  if (!table || !snapshot) return;
  const meta = snapshot.meta;
  const curves = snapshot.curves.filter((c) => state.selected.has(c.venue));

  if (curves.length === 0) {
    table.innerHTML = '<div class="depth-row"><span class="depth-label">No venues selected</span></div>';
    return;
  }

  const maxBase = meta.config?.max_base || 100;
  const levels = [0.0025, 0.01, 0.05, 0.1, 0.25].map((r) => maxBase * r).filter((v) => v > 0).slice(0, 5);
  const bids = venueCurves(curves).filter((c) => c.side === 'bid');
  const asks = venueCurves(curves).filter((c) => c.side === 'ask');

  const header = `<div class="depth-row header"><span>Depth</span><span style="text-align:right">Bid</span><span style="text-align:right">Ask / Spread</span></div>`;

  const rows = levels.map((level) => {
    const bidP = bestPriceAtDepth(bids, level, true);
    const askP = bestPriceAtDepth(asks, level, false);
    const spread = bidP && askP ? ((askP - bidP) / ((askP + bidP) / 2)) * 10000 : null;
    return `<div class="depth-row">
      <span class="depth-label">${fmt(level, 4)}</span>
      <span class="depth-bid">${fmt(bidP, 2)}</span>
      <span class="depth-ask">${fmt(askP, 2)} <span class="depth-spread">${spread != null ? fmt(spread, 1) + 'bp' : ''}</span></span>
    </div>`;
  });

  table.innerHTML = header + rows.join('');
}

function bestPriceAtDepth(curves, depth, isBid) {
  let best = null;
  curves.forEach((curve) => {
    const price = priceAtDepth(curve, depth);
    if (price === null) return;
    if (best === null) best = price;
    else best = isBid ? Math.max(best, price) : Math.min(best, price);
  });
  return best;
}

function priceAtDepth(curve, depth) {
  for (const p of curve.points) {
    if (p.cum_base >= depth) return p.price;
  }
  return null;
}

// ── Pool list ──

function renderPoolList(snapshot) {
  const container = document.getElementById('pool-list');
  if (!container) return;
  if (!snapshot) {
    container.innerHTML = '<div class="pool-row"><div class="pool-info"><span class="pool-addr">Waiting for data</span></div></div>';
    return;
  }

  const poolMap = new Map();
  snapshot.curves.forEach((curve) => {
    const address = extractAddr(curve.pool);
    if (!address) return;
    const key = address.toLowerCase();
    if (!poolMap.has(key)) poolMap.set(key, { address, venues: new Set() });
    poolMap.get(key).venues.add(curve.venue);
  });

  container.innerHTML = ALLOWED_POOLS.map((address) => {
    const key = address.toLowerCase();
    const entry = poolMap.get(key);
    const venues = entry ? [...entry.venues].map(venueLabel).join(', ') : 'Not found';
    const statusCls = entry ? 'live' : 'off';
    const statusText = entry ? 'LIVE' : 'OFF';
    return `<div class="pool-row">
      <div class="pool-info">
        <span class="pool-addr">${shortAddr(address)}</span>
        <span class="pool-venue-tag">${venues}</span>
      </div>
      <span class="pool-status-badge ${statusCls}">${statusText}</span>
    </div>`;
  }).join('');
}

// ── Scale toggle ──

function initScaleToggle() {
  const container = document.getElementById('scale-toggle');
  if (!container) return;
  container.addEventListener('click', (e) => {
    const btn = e.target.closest('.scale-btn');
    if (!btn) return;
    const mode = btn.dataset.mode;
    state.scaleMode = mode;
    container.querySelectorAll('.scale-btn').forEach((b) => b.classList.remove('active'));
    btn.classList.add('active');
    renderChart(state.snapshot);
  });
}

// ── Init ──

(function init() {
  updateStatus('Loading', 'loading');
  initScaleToggle();
  renderPoolList(null);

  fetchLatest()
    .then((snapshot) => {
      applySnapshot(snapshot);
      updateStatus('Live', 'ok');
    })
    .catch(() => {
      updateStatus('Waiting', 'loading');
    })
    .finally(() => {
      connectWs();
    });
})();
