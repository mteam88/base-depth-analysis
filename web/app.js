const state = {
  snapshot: null,
  selected: new Set(),
  venueIndex: {},
  statusEl: null,
};

const askPalette = ['#c2410c', '#d97706', '#ef4444', '#f97316', '#b45309', '#ea580c'];
const bidPalette = ['#0f766e', '#059669', '#22c55e', '#16a34a', '#14b8a6', '#15803d'];

function formatNumber(value, decimals = 4) {
  if (value === null || value === undefined || Number.isNaN(value)) return '—';
  return Number(value).toLocaleString(undefined, { maximumFractionDigits: decimals });
}

function venueLabel(venue) {
  return venue.replace(/_/g, ' ').toUpperCase();
}

function updateStatus(text) {
  if (!state.statusEl) {
    state.statusEl = document.getElementById('status');
  }
  if (state.statusEl) {
    state.statusEl.textContent = text;
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
    const defaults = ['unified', 'kyber', 'unified_kyber'];
    const picked = defaults.filter((v) => venues.includes(v));
    if (picked.length > 0) {
      picked.forEach((v) => state.selected.add(v));
    } else {
      venues.forEach((v) => state.selected.add(v));
    }
  }
  state.venueIndex = Object.fromEntries(venues.map((v, idx) => [v, idx]));
  renderVenueChips(venues);
  renderChart(snapshot);
  renderStats(snapshot);
  renderDepthTable(snapshot);
}

function updateMeta(meta) {
  const metaEl = document.getElementById('snapshot-meta');
  if (!metaEl) return;
  const time = new Date(meta.snapshot_time).toLocaleString();
  metaEl.innerHTML = `Pair: <strong>${meta.pair.base.symbol}/${meta.pair.quote.symbol}</strong> · Block <strong>${meta.block_number}</strong> · ${time}`;
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
  // Keep prices within 10% of the median — anything beyond is an outlier pool
  const lo = median * 0.9;
  const hi = median * 1.1;
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
    return {
      x: curve.points.map((p) => p.price),
      y: curve.points.map((p) => p.cum_base),
      type: 'scatter',
      mode: 'lines',
      line: {
        color,
        width: 2,
        shape: 'hv',
        dash: curve.side === 'bid' ? 'dot' : 'solid',
      },
      name: `${venueLabel(curve.venue)} ${curve.side.toUpperCase()}`,
      hovertemplate: 'Price %{x:.6f}<br>Depth %{y:.4f}<extra></extra>',
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
      line: { color: '#111827', width: 1, dash: 'dot' },
    });
  }

  const range = priceRange(curves);
  const layout = {
    margin: { l: 60, r: 30, t: 20, b: 50 },
    xaxis: { title: 'Price (quote / base)', gridcolor: 'rgba(0,0,0,0.05)', range },
    yaxis: { title: 'Cumulative base', gridcolor: 'rgba(0,0,0,0.05)' },
    paper_bgcolor: 'rgba(0,0,0,0)',
    plot_bgcolor: 'rgba(255,255,255,0.6)',
    legend: { orientation: 'h', y: -0.2 },
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
  const maxBase = meta.config?.max_base || 100;
  const levels = [0.001, 0.01, 0.05, 0.1, 0.25]
    .map((ratio) => maxBase * ratio)
    .filter((v) => v > 0)
    .slice(0, 5);

  const bids = curves.filter((c) => c.side === 'bid');
  const asks = curves.filter((c) => c.side === 'ask');

  table.innerHTML = levels
    .map((level) => {
      const bidPrice = bestPriceAtDepth(bids, level, true);
      const askPrice = bestPriceAtDepth(asks, level, false);
      const spread = bidPrice && askPrice ? ((askPrice - bidPrice) / ((askPrice + bidPrice) / 2)) * 10000 : null;
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

function connectWs() {
  const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${protocol}://${location.host}/ws`);
  ws.onopen = () => updateStatus('Live feed connected');
  ws.onmessage = (event) => {
    try {
      const payload = JSON.parse(event.data);
      applySnapshot(payload);
    } catch (err) {
      console.error(err);
    }
  };
  ws.onclose = () => {
    updateStatus('Live feed disconnected · retrying…');
    setTimeout(connectWs, 3000);
  };
}

(async () => {
  updateStatus('Loading snapshot…');
  try {
    const snapshot = await fetchLatest();
    applySnapshot(snapshot);
    updateStatus('Snapshot loaded');
  } catch (err) {
    updateStatus('Waiting for first snapshot…');
  }
  connectWs();
})();
