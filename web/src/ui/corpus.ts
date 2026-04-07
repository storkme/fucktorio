/**
 * Corpus browser panel.
 *
 * Two modes:
 * 1. Paste a raw Factorio blueprint string → parsed immediately via WASM (no preprocessing)
 * 2. Load corpus.json produced by scripts/analysis/mine_corpus.py → browse many blueprints
 *    with stats (bus detection, row pitch, etc.)
 */

import { parseBlueprint } from "../engine.js";
import type { LayoutResult } from "../engine.js";

// ---- Types matching corpus.json schema ----

interface CorpusStats {
  final_product: string | null;
  machine_count: number;
  recipe_count: number;
  is_bus_layout: boolean;
  bus_orientation: string | null;
  bus_lane_count: number;
  bus_pitch: number;
  row_pitch: number;
  row_count: number;
  machines_per_row: number;
  density: number;
  belt_tiles: number;
  pipe_tiles: number;
  bbox_width: number;
  bbox_height: number;
  [key: string]: unknown;
}

interface CorpusEntry {
  name: string;
  stats: CorpusStats;
  layout: LayoutResult;
}

interface CorpusFile {
  blueprints: CorpusEntry[];
  aggregate?: Record<string, number>;
}

// ---- Style ----

const STYLE = `
  .corpus-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: #1e1e1e;
    color: #e0e0e0;
    font-family: sans-serif;
    font-size: 13px;
    box-sizing: border-box;
    overflow: hidden;
  }
  .corpus-load {
    padding: 12px;
    border-bottom: 1px solid #333;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .corpus-load h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: #c8c8c8;
  }
  .corpus-load p {
    margin: 0;
    color: #888;
    font-size: 11px;
    line-height: 1.4;
  }
  .corpus-load-btn {
    background: #0e639c;
    color: #fff;
    border: none;
    border-radius: 3px;
    padding: 6px 10px;
    font-size: 12px;
    cursor: pointer;
    text-align: center;
  }
  .corpus-load-btn:hover { background: #1177bb; }
  .corpus-paste {
    padding: 8px 12px;
    border-bottom: 1px solid #333;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .corpus-paste label {
    font-size: 11px;
    color: #888;
  }
  .corpus-paste textarea {
    background: #252526;
    color: #9cdcfe;
    border: 1px solid #444;
    border-radius: 3px;
    padding: 4px 6px;
    font-family: monospace;
    font-size: 10px;
    resize: none;
    height: 48px;
  }
  .corpus-paste textarea::placeholder { color: #555; }
  .corpus-paste textarea.error { border-color: #c44; }
  .corpus-paste-error {
    color: #f66;
    font-size: 10px;
    font-family: monospace;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .corpus-filters {
    padding: 8px 12px;
    border-bottom: 1px solid #333;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .corpus-filter-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .corpus-filter-row label {
    color: #aaa;
    white-space: nowrap;
  }
  .corpus-filter-row input[type="checkbox"] {
    accent-color: #569cd6;
  }
  .corpus-filter-row select,
  .corpus-filter-row input[type="text"] {
    flex: 1;
    background: #252526;
    color: #e0e0e0;
    border: 1px solid #444;
    border-radius: 3px;
    padding: 3px 5px;
    font-size: 12px;
  }
  .corpus-count {
    font-size: 11px;
    color: #666;
    padding: 0 12px 4px;
  }
  .corpus-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 0;
  }
  .corpus-item {
    padding: 7px 12px;
    cursor: pointer;
    border-bottom: 1px solid #2a2a2a;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .corpus-item:hover { background: #2a2d2e; }
  .corpus-item.selected { background: #094771; }
  .corpus-item-name {
    font-family: monospace;
    font-size: 11px;
    color: #9cdcfe;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .corpus-item-meta {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .corpus-badge {
    font-size: 10px;
    padding: 1px 5px;
    border-radius: 2px;
    font-weight: 600;
    letter-spacing: 0.03em;
  }
  .corpus-badge-bus { background: #1a4a1a; color: #6a9; }
  .corpus-badge-product { background: #2a2a3a; color: #9cdcfe; }
  .corpus-badge-machines { color: #888; font-size: 10px; }
  .corpus-stats {
    padding: 8px 12px;
    border-top: 1px solid #333;
    font-family: monospace;
    font-size: 11px;
    background: #252526;
    display: none;
  }
  .corpus-stats.visible { display: block; }
  .corpus-stats-title {
    color: #9cdcfe;
    font-weight: 600;
    margin-bottom: 4px;
    font-size: 11px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .corpus-stats-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2px 8px;
  }
  .corpus-stats-row {
    display: flex;
    justify-content: space-between;
  }
  .corpus-stats-key { color: #888; }
  .corpus-stats-val { color: #b5cea8; }
  .corpus-empty {
    padding: 24px 12px;
    color: #555;
    font-size: 12px;
    text-align: center;
    line-height: 1.6;
  }
`;

function injectStyle(): void {
  if (document.getElementById("fucktorio-corpus-style")) return;
  const el = document.createElement("style");
  el.id = "fucktorio-corpus-style";
  el.textContent = STYLE;
  document.head.appendChild(el);
}

// ---- Render ----

export function initCorpusPanel(
  container: HTMLElement,
  onRender: (layout: LayoutResult) => void,
): void {
  injectStyle();
  container.innerHTML = "";

  const panel = document.createElement("div");
  panel.className = "corpus-panel";
  container.appendChild(panel);

  // State
  let corpus: CorpusEntry[] = [];
  let filtered: CorpusEntry[] = [];
  let selectedIdx = -1;
  let busOnly = false;
  let productFilter = "";
  let searchFilter = "";

  // ---- Load section ----
  const loadSection = document.createElement("div");
  loadSection.className = "corpus-load";

  const h2 = document.createElement("h2");
  h2.textContent = "Corpus Browser";
  loadSection.appendChild(h2);

  const desc = document.createElement("p");
  desc.textContent = "Load corpus.json generated by scripts/analysis/mine_corpus.py";
  loadSection.appendChild(desc);

  const fileInput = document.createElement("input");
  fileInput.type = "file";
  fileInput.accept = ".json";
  fileInput.style.display = "none";

  const loadBtn = document.createElement("button");
  loadBtn.className = "corpus-load-btn";
  loadBtn.textContent = "Load corpus.json…";
  loadBtn.onclick = () => fileInput.click();

  loadSection.appendChild(fileInput);
  loadSection.appendChild(loadBtn);
  panel.appendChild(loadSection);

  // ---- Paste blueprint section ----
  const pasteSection = document.createElement("div");
  pasteSection.className = "corpus-paste";

  const pasteLabel = document.createElement("label");
  pasteLabel.textContent = "Or paste a blueprint string directly (parsed in-browser via WASM)";
  pasteSection.appendChild(pasteLabel);

  const pasteArea = document.createElement("textarea");
  pasteArea.placeholder = "0eJyt... paste Factorio blueprint string";
  pasteSection.appendChild(pasteArea);

  const pasteError = document.createElement("div");
  pasteError.className = "corpus-paste-error";
  pasteError.style.display = "none";
  pasteSection.appendChild(pasteError);

  panel.appendChild(pasteSection);

  pasteArea.addEventListener("input", () => {
    const val = pasteArea.value.trim();
    if (!val) {
      pasteArea.classList.remove("error");
      pasteError.style.display = "none";
      return;
    }
    try {
      const layout = parseBlueprint(val);
      pasteArea.classList.remove("error");
      pasteError.style.display = "none";
      onRender(layout);
    } catch (err) {
      pasteArea.classList.add("error");
      pasteError.textContent = String(err);
      pasteError.style.display = "block";
    }
  });

  // ---- Filters section ----
  const filtersSection = document.createElement("div");
  filtersSection.className = "corpus-filters";
  filtersSection.style.display = "none";

  // Search by name
  const searchRow = document.createElement("div");
  searchRow.className = "corpus-filter-row";
  const searchLabel = document.createElement("label");
  searchLabel.textContent = "Search";
  const searchInput = document.createElement("input");
  searchInput.type = "text";
  searchInput.placeholder = "filter by name…";
  searchRow.appendChild(searchLabel);
  searchRow.appendChild(searchInput);
  filtersSection.appendChild(searchRow);

  // Product filter
  const productRow = document.createElement("div");
  productRow.className = "corpus-filter-row";
  const productLabel = document.createElement("label");
  productLabel.textContent = "Product";
  const productSelect = document.createElement("select");
  productRow.appendChild(productLabel);
  productRow.appendChild(productSelect);
  filtersSection.appendChild(productRow);

  // Bus-only toggle
  const busRow = document.createElement("div");
  busRow.className = "corpus-filter-row";
  const busCb = document.createElement("input");
  busCb.type = "checkbox";
  const busLabel = document.createElement("label");
  busLabel.style.display = "flex";
  busLabel.style.alignItems = "center";
  busLabel.style.gap = "5px";
  busLabel.style.cursor = "pointer";
  busLabel.appendChild(busCb);
  busLabel.appendChild(document.createTextNode("Bus layouts only"));
  busRow.appendChild(busLabel);
  filtersSection.appendChild(busRow);

  panel.appendChild(filtersSection);

  // ---- Count label ----
  const countEl = document.createElement("div");
  countEl.className = "corpus-count";
  countEl.style.display = "none";
  panel.appendChild(countEl);

  // ---- List ----
  const list = document.createElement("div");
  list.className = "corpus-list";
  panel.appendChild(list);

  // ---- Stats bar ----
  const statsBar = document.createElement("div");
  statsBar.className = "corpus-stats";
  panel.appendChild(statsBar);

  // ---- Logic ----

  function applyFilters(): void {
    filtered = corpus.filter((entry) => {
      if (busOnly && !entry.stats.is_bus_layout) return false;
      if (productFilter && productFilter !== "__all__" && entry.stats.final_product !== productFilter) return false;
      if (searchFilter && !entry.name.toLowerCase().includes(searchFilter.toLowerCase())) return false;
      return true;
    });
    selectedIdx = -1;
    renderList();
  }

  function renderList(): void {
    list.innerHTML = "";
    countEl.textContent = `${filtered.length} of ${corpus.length} blueprint(s)`;

    if (filtered.length === 0) {
      const empty = document.createElement("div");
      empty.className = "corpus-empty";
      empty.textContent = corpus.length === 0
        ? "No corpus loaded yet."
        : "No blueprints match the current filters.";
      list.appendChild(empty);
      statsBar.classList.remove("visible");
      return;
    }

    for (let i = 0; i < filtered.length; i++) {
      const entry = filtered[i];
      const item = document.createElement("div");
      item.className = "corpus-item" + (i === selectedIdx ? " selected" : "");

      const nameEl = document.createElement("div");
      nameEl.className = "corpus-item-name";
      nameEl.textContent = entry.name;
      nameEl.title = entry.name;
      item.appendChild(nameEl);

      const meta = document.createElement("div");
      meta.className = "corpus-item-meta";

      if (entry.stats.is_bus_layout) {
        const badge = document.createElement("span");
        badge.className = "corpus-badge corpus-badge-bus";
        badge.textContent = "BUS";
        meta.appendChild(badge);
      }
      if (entry.stats.final_product) {
        const badge = document.createElement("span");
        badge.className = "corpus-badge corpus-badge-product";
        badge.textContent = entry.stats.final_product;
        meta.appendChild(badge);
      }
      const machines = document.createElement("span");
      machines.className = "corpus-badge corpus-badge-machines";
      machines.textContent = `${entry.stats.machine_count}m`;
      meta.appendChild(machines);

      item.appendChild(meta);

      const idx = i;
      item.addEventListener("click", () => selectEntry(idx));
      list.appendChild(item);
    }
  }

  function selectEntry(idx: number): void {
    selectedIdx = idx;
    const entry = filtered[idx];

    // Re-render list to update selection highlight
    renderList();

    // Render layout on canvas
    onRender(entry.layout);

    // Show stats bar
    renderStatsBar(entry);
  }

  function renderStatsBar(entry: CorpusEntry): void {
    statsBar.innerHTML = "";
    statsBar.classList.add("visible");

    const title = document.createElement("div");
    title.className = "corpus-stats-title";
    title.textContent = entry.name;
    title.title = entry.name;
    statsBar.appendChild(title);

    const grid = document.createElement("div");
    grid.className = "corpus-stats-grid";

    const s = entry.stats;
    const rows: [string, string][] = [
      ["machines", String(s.machine_count)],
      ["recipes", String(s.recipe_count)],
      ["is_bus", s.is_bus_layout ? "yes" : "no"],
      ["density", s.density.toFixed(2)],
    ];
    if (s.is_bus_layout) {
      rows.push(
        ["bus_lanes", String(s.bus_lane_count)],
        ["bus_pitch", s.bus_pitch.toFixed(1)],
        ["row_pitch", s.row_pitch.toFixed(1)],
        ["rows", String(s.row_count)],
      );
    }
    rows.push(
      ["bbox", `${s.bbox_width}×${s.bbox_height}`],
      ["belt_tiles", String(s.belt_tiles)],
    );
    if (s.pipe_tiles > 0) rows.push(["pipe_tiles", String(s.pipe_tiles)]);

    for (const [k, v] of rows) {
      const row = document.createElement("div");
      row.className = "corpus-stats-row";
      const key = document.createElement("span");
      key.className = "corpus-stats-key";
      key.textContent = k;
      const val = document.createElement("span");
      val.className = "corpus-stats-val";
      val.textContent = v;
      row.appendChild(key);
      row.appendChild(val);
      grid.appendChild(row);
    }

    statsBar.appendChild(grid);
  }

  function populateProductFilter(): void {
    const products = new Set(
      corpus.map((e) => e.stats.final_product).filter(Boolean) as string[]
    );
    productSelect.innerHTML = "";
    const all = document.createElement("option");
    all.value = "__all__";
    all.textContent = "All products";
    productSelect.appendChild(all);
    for (const p of Array.from(products).sort()) {
      const opt = document.createElement("option");
      opt.value = p;
      opt.textContent = p;
      productSelect.appendChild(opt);
    }
  }

  // ---- File loading ----
  fileInput.addEventListener("change", () => {
    const file = fileInput.files?.[0];
    if (!file) return;

    const reader = new FileReader();
    reader.onload = (e) => {
      try {
        const data = JSON.parse(e.target?.result as string) as CorpusFile;
        corpus = data.blueprints ?? [];
        filtered = corpus;
        selectedIdx = -1;

        loadBtn.textContent = `Reload corpus.json (${corpus.length} blueprints)`;
        filtersSection.style.display = "";
        countEl.style.display = "";
        statsBar.classList.remove("visible");

        populateProductFilter();
        applyFilters();
      } catch (err) {
        alert(`Failed to parse corpus.json: ${err}`);
      }
    };
    reader.readAsText(file);
  });

  // ---- Filter events ----
  searchInput.addEventListener("input", () => {
    searchFilter = searchInput.value;
    applyFilters();
  });

  productSelect.addEventListener("change", () => {
    productFilter = productSelect.value;
    applyFilters();
  });

  busCb.addEventListener("change", () => {
    busOnly = busCb.checked;
    applyFilters();
  });

  // Initial empty state
  renderList();
}
