const ALLOWED_POOLS = [
  '0xb2cc224c1c9fee385f8ad6a55b4d94e92359dc59',
  '0xd0b53d9277642d899df5c87a3966a349a798f224',
  '0x72ab388e2e2f6facef59e3c3fa2c4e29011c2d38',
];

const state = {
  snapshot: null,
  selected: new Set(),
  venueIndex: {},
  statusEl: null,
};

const askPalette = ['#c7542e', '#d97706', '#f97316', '#ea580c', '#b45309'];
const bidPalette = ['#1f7a6b', '#0f766e', '#14b8a6', '#0ea5a5', '#0d9488'];

function getCssVar(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function formatNumber(value, decimals = 4) {
  if (value === null || value === undefined || Number.isNaN(value)) return '—';
  return Number(value).toLocaleString(undefined, { maximumFractionDigits: decimals });
}

function venueLabel(venue) {
  return venue.replace(/_/g, ' ').toUpperCase();
}

function shortAddress(addr) {
  if (!addr) return '';
  const compact = addr.toLowerCase();
  return `${compact.slice(0, 6)}...${compact.slice(-4)}`;
}

function extractAddress(value) {
  if (!value) return null;
  const match = String(value).match(/0x[a-fA-F0-9]{40}/);
  return match ? match[0] : null;
}

function normalizeAddress(value) {
  return value ? value.toLowerCase() : '';
}

function poolLabel(curve, short = false) {
  if (!curve || !curve.pool) return '';
  let label = String(curve.pool);
  const prefix = `${curve.venue}:`;
  if (label.startsWith(prefix)) {
    label = label.slice(prefix.length);
  }
  if (short) {
    const addr = extractAddress(label);
    if (addr) {
      label = label.replace(addr, shortAddress(addr));
    }
  }
  return label.split(':').join(' / ');
}

function updateStatus(text, stateValue = 'loading') {
  if (!state.statusEl) {
    state.statusEl = document.getElementById('status');
  }
  if (state.statusEl) {
    state.statusEl.textContent = text;
    state.statusEl.dataset.state = stateValue;
  }
}

function updateClock(text) {
  const clock = document.getElementById('snapshot-clock');
  if (clock) {
    clock.textContent = text;
  }
}

async function fetchLatest() {
  const res = await fetch('/api/snapshots/latest');
  if (!res.ok) throw new Error('No snapshot yet');
  return res.json();
}

function applySnapshot(snapshot) {
  state.snapshot = snapshot;
  updateMeta(snapshot.meta);
  const venues = Array.from(new Set(snapshot.curves.map((c) => c.venue)));
  if (state.selected.size === 0) {
    if (venues.length <= 5) {
      venues.forEach((v) => state.selected.add(v));
    } else {
      const defaults = ['unified', 'kyber', 'tessera'];
      const picked = defaults.filter((v) => venues.includes(v));
      if (picked.length > 0) {
        picked.forEach((v) => state.selected.add(v));
      } else {
        venues.forEach((v) => state.selected.add(v));
      }
    }
  }
  state.venueIndex = Object.fromEntries(venues.map((v, idx) => [v, idx]));
  renderVenueChips(venues);
  renderChart(snapshot);
  renderStats(snapshot);
  renderDepthTable(snapshot);
  renderPoolList(snapshot);
}

function updateMeta(meta) {
  const metaEl = document.getElementById('snapshot-meta');
  if (!metaEl) return;
  const time = new Date(meta.snapshot_time).toLocaleString();
  updateClock(`Last update: ${time}`);

  const items = [
    { label: 'Pair', value: `${meta.pair.base.symbol}/${meta.pair.quote.symbol}` },
    { label: 'Block', value: formatNumber(meta.block_number, 0) },
    { label: 'Snapshot', value: time },
    { label: 'Venues', value: `${meta.venues.length}` },
    { label: 'Steps', value: `${meta.config?.steps ?? '—'}` },
  ];

  if (meta.config?.kyber_steps) {
    items.push({ label: 'Kyber steps', value: `${meta.config.kyber_steps}` });
  }

  metaEl.innerHTML = items
    .map((item) => {
      return `
        <div class="meta-card">
          <span class="meta-label">${item.label}</span>
          <div class="meta-value">${item.value}</div>
        </div>
      `;
    })
    .join('');
}

function renderVenueChips(venues) {
  const container = document.getElementById('venue-toggle');
  if (!container) return;
  container.innerHTML = '';
  venues.forEach((venue) => {
    const btn = document.createElement('button');
    btn.className = `chip ${state.selected.has(venue) ? 'active' : ''}`;
    btn.textContent = venueLabel(venue);
    btn.addEventListener('click', () => {
      if (state.selected.has(venue)) {
        state.selected.delete(venue);
      } else {
        state.selected.add(venue);
      }
      renderVenueChips(venues);
      renderChart(state.snapshot);
      renderStats(state.snapshot);
      renderDepthTable(state.snapshot);
    });
    container.appendChild(btn);
  });
}

function colorFor(venue, side) {
  const idx = state.venueIndex[venue] ?? 0;
  return side === 'ask'
    ? askPalette[idx % askPalette.length]
    : bidPalette[idx % bidPalette.length];
}

function priceRange(curves) {
  const prices = [];
  curves.forEach((c) => c.points.forEach((p) => prices.push(p.price)));
  if (prices.length < 2) return null;
  prices.sort((a, b) => a - b);
  const median = prices[Math.floor(prices.length / 2)];
  const lo = median * 0.88;
  const hi = median * 1.12;
  const pad = (hi - lo) * 0.05;
  return [lo - pad, hi + pad];
}

function renderChart(snapshot) {
  if (!snapshot) return;
  const curves = snapshot.curves.filter((c) => state.selected.has(c.venue));
  if (curves.length === 0) {
    Plotly.purge('chart');
    return;
  }
  const traces = curves.map((curve) => {
    const color = colorFor(curve.venue, curve.side);
    const venueName = venueLabel(curve.venue);
    const poolShort = poolLabel(curve, true);
    const poolFull = poolLabel(curve, false) || poolShort || venueName;
    const nameParts = [venueName, curve.side.toUpperCase()];
    if (poolShort && poolShort.toUpperCase() !== venueName) nameParts.push(poolShort);
    return {
      x: curve.points.map((p) => p.price),
      y: curve.points.map((p) => p.cum_base),
      type: 'scatter',
      mode: 'lines',
      line: {
        color,
        width: 2.2,
        shape: 'hv',
        dash: curve.side === 'bid' ? 'dot' : 'solid',
      },
      name: nameParts.join(' - '),
      hovertemplate: `Pool ${poolFull}<br>Price %{x:.6f}<br>Depth %{y:.4f}<extra></extra>`,
    };
  });

  const { bestBid, bestAsk, maxDepth } = bestPrices(curves);
  const shapes = [];
  if (bestBid && bestAsk) {
    const mid = (bestBid + bestAsk) / 2;
    shapes.push({
      type: 'line',
      x0: mid,
      x1: mid,
      y0: 0,
      y1: maxDepth,
      line: { color: '#1c1a16', width: 1, dash: 'dot' },
    });
  }

  const range = priceRange(curves);
  const layout = {
    margin: { l: 60, r: 30, t: 10, b: 50 },
    xaxis: {
      title: 'Price (quote per base)',
      gridcolor: getCssVar('--chart-grid'),
      range,
      tickfont: { family: 'Space Mono, monospace', size: 11 },
    },
    yaxis: {
      title: 'Cumulative base',
      gridcolor: getCssVar('--chart-grid'),
      tickfont: { family: 'Space Mono, monospace', size: 11 },
    },
    font: { family: 'Manrope, sans-serif', color: getCssVar('--ink') },
    paper_bgcolor: 'rgba(0,0,0,0)',
    plot_bgcolor: getCssVar('--chart-bg'),
    legend: { orientation: 'h', y: -0.2, font: { size: 10 } },
    shapes,
  };

  Plotly.react('chart', traces, layout, { displayModeBar: false, responsive: true });
}

function bestPrices(curves) {
  let bestBid = null;
  let bestAsk = null;
  let maxDepth = 0;
  curves.forEach((curve) => {
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

function renderStats(snapshot) {
  const statsEl = document.getElementById('market-stats');
  if (!statsEl || !snapshot) return;
  const curves = snapshot.curves.filter((c) => state.selected.has(c.venue));
  const { bestBid, bestAsk } = bestPrices(curves);
  const mid = bestBid && bestAsk ? (bestBid + bestAsk) / 2 : null;
  const spreadBps = bestBid && bestAsk ? ((bestAsk - bestBid) / mid) * 10000 : null;
  statsEl.innerHTML = [
    statBlock('Best Bid', formatNumber(bestBid, 6)),
    statBlock('Best Ask', formatNumber(bestAsk, 6)),
    statBlock('Mid', formatNumber(mid, 6)),
    statBlock('Spread bps', formatNumber(spreadBps, 2)),
  ].join('');
}

function statBlock(label, value) {
  return `<div class="stat"><span>${label}</span><strong>${value}</strong></div>`;
}

function renderDepthTable(snapshot) {
  const table = document.getElementById('depth-table');
  if (!table || !snapshot) return;
  const meta = snapshot.meta;
  const curves = snapshot.curves.filter((c) => state.selected.has(c.venue));
  if (curves.length === 0) {
    table.innerHTML = '<div class="depth-row">No active venues selected.</div>';
    return;
  }
  const maxBase = meta.config?.max_base || 100;
  const levels = [0.0025, 0.01, 0.05, 0.1, 0.25]
    .map((ratio) => maxBase * ratio)
    .filter((v) => v > 0)
    .slice(0, 5);

  const bids = curves.filter((c) => c.side === 'bid');
  const asks = curves.filter((c) => c.side === 'ask');

  table.innerHTML = levels
    .map((level) => {
      const bidPrice = bestPriceAtDepth(bids, level, true);
      const askPrice = bestPriceAtDepth(asks, level, false);
      const spread = bidPrice && askPrice
        ? ((askPrice - bidPrice) / ((askPrice + bidPrice) / 2)) * 10000
        : null;
      return `
        <div class="depth-row">
          <div><strong>${formatNumber(level, 4)}</strong> ${meta.pair.base.symbol}</div>
          <div>Bid ${formatNumber(bidPrice, 6)}</div>
          <div>Ask ${formatNumber(askPrice, 6)} / ${formatNumber(spread, 2)} bps</div>
        </div>
      `;
    })
    .join('');
}

function bestPriceAtDepth(curves, depth, isBid) {
  let best = null;
  curves.forEach((curve) => {
    const price = priceAtDepth(curve, depth);
    if (price === null) return;
    if (best === null) {
      best = price;
    } else {
      best = isBid ? Math.max(best, price) : Math.min(best, price);
    }
  });
  return best;
}

function priceAtDepth(curve, depth) {
  for (const point of curve.points) {
    if (point.cum_base >= depth) return point.price;
  }
  return null;
}

function renderPoolList(snapshot) {
  const container = document.getElementById('pool-list');
  if (!container) return;
  if (!snapshot) {
    container.innerHTML = '<div class="pool-row missing"><div>Waiting for pools</div><div class="pool-status">OFF</div></div>';
    return;
  }
  const poolMap = new Map();
  snapshot.curves.forEach((curve) => {
    const address = extractAddress(curve.pool);
    if (!address) return;
    const key = normalizeAddress(address);
    if (!poolMap.has(key)) {
      poolMap.set(key, { address, venues: new Set() });
    }
    poolMap.get(key).venues.add(curve.venue);
  });

  container.innerHTML = ALLOWED_POOLS.map((address) => {
    const key = normalizeAddress(address);
    const entry = poolMap.get(key);
    const venues = entry
      ? Array.from(entry.venues).map(venueLabel).join(', ')
      : 'Not detected';
    const rowClass = entry ? 'pool-row' : 'pool-row missing';
    const status = entry ? 'LIVE' : 'OFF';
    return `
      <div class="${rowClass}">
        <div>
          <div class="pool-address">${shortAddress(address)}</div>
          <div class="pool-venues">${venues}</div>
        </div>
        <div class="pool-status">${status}</div>
      </div>
    `;
  }).join('');
}

function connectWs() {
  const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${protocol}://${location.host}/ws`);
  ws.onopen = () => updateStatus('Live feed connected', 'ok');
  ws.onmessage = (event) => {
    try {
      const payload = JSON.parse(event.data);
      applySnapshot(payload);
    } catch (err) {
      console.error(err);
    }
  };
  ws.onclose = () => {
    updateStatus('Live feed disconnected, retrying...', 'error');
    setTimeout(connectWs, 3000);
  };
}

(function init() {
  updateStatus('Loading snapshot...', 'loading');
  renderPoolList(null);
  fetchLatest()
    .then((snapshot) => {
      applySnapshot(snapshot);
      updateStatus('Snapshot loaded', 'ok');
    })
    .catch(() => {
      updateStatus('Waiting for first snapshot...', 'loading');
    })
    .finally(() => {
      connectWs();
    });
})();
