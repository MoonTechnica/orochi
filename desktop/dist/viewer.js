// Runs inside viewer.html's sandboxed frame only. The window sends
// {kind:"open", lib, worker, bytes}; pdf.js arrives as source text and is imported from a blob,
// because an opaque-origin frame cannot fetch modules from the app.
"use strict";
(() => {
  const pagesHost = document.getElementById("pages");
  const countHost = document.getElementById("count");
  const zoomHost = document.getElementById("zoom");
  let pdf = null;
  let scale = 1.2;
  let observer = null;
  let docx = null;

  const say = (message) => parent.postMessage(message, "*");
  const fail = (error) => {
    pagesHost.textContent = "";
    const note = document.createElement("div");
    note.id = "note";
    note.textContent = String(error?.message || error);
    pagesHost.append(note);
    say({ kind: "error", message: String(error?.message || error) });
  };

  async function layout() {
    observer?.disconnect();
    pagesHost.textContent = "";
    zoomHost.textContent = `${Math.round(scale * 100)}%`;
    const first = await pdf.getPage(1);
    const base = first.getViewport({ scale });
    observer = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) draw(entry.target);
      }
    }, { root: pagesHost, rootMargin: "400px" });
    for (let number = 1; number <= pdf.numPages; number += 1) {
      const canvas = document.createElement("canvas");
      canvas.className = "page";
      canvas.dataset.number = String(number);
      // Until a page is read its size is page one's, which is right for most documents.
      canvas.style.width = `${base.width}px`;
      canvas.style.height = `${base.height}px`;
      pagesHost.append(canvas);
      observer.observe(canvas);
    }
  }

  async function draw(canvas) {
    if (canvas.dataset.drawn === String(scale)) return;
    canvas.dataset.drawn = String(scale);
    try {
      const page = await pdf.getPage(Number(canvas.dataset.number));
      const viewport = page.getViewport({ scale });
      const ratio = window.devicePixelRatio || 1;
      canvas.width = Math.floor(viewport.width * ratio);
      canvas.height = Math.floor(viewport.height * ratio);
      canvas.style.width = `${viewport.width}px`;
      canvas.style.height = `${viewport.height}px`;
      await page.render({
        canvasContext: canvas.getContext("2d"),
        viewport,
        transform: ratio === 1 ? null : [ratio, 0, 0, ratio, 0, 0],
      }).promise;
    } catch (error) {
      canvas.dataset.drawn = "";
      say({ kind: "page-error", message: String(error?.message || error) });
    }
  }

  // Classic scripts (UMD builds) are run from blobs: the frame cannot fetch anything.
  const run = (source) => new Promise((done, bad) => {
    const node = document.createElement("script");
    node.src = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
    node.onload = done;
    node.onerror = () => bad(new Error("a viewer library did not load"));
    document.head.append(node);
  });

  async function openPdf(message) {
    const blob = (source) => URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
    const lib = await import(blob(message.lib));
    lib.GlobalWorkerOptions.workerSrc = blob(message.worker);
    pdf = await lib.getDocument({
      data: new Uint8Array(message.bytes),
      // A document's own script never runs, and nothing is fetched on its behalf.
      isEvalSupported: false,
      enableScripting: false,
      useWorkerFetch: false,
      disableFontFace: false,
    }).promise;
    countHost.textContent = `${pdf.numPages} p`;
    await layout();
    say({ kind: "opened", pages: pdf.numPages });
  }

  async function openDocx(message) {
    scale = 1;
    for (const source of message.scripts) await run(source);
    pagesHost.textContent = "";
    const sheet = document.createElement("div");
    pagesHost.append(sheet);
    await window.docx.renderAsync(message.bytes, sheet, null, {
      inWrapper: true,
      breakPages: true,
      ignoreLastRenderedPageBreak: false,
      useBase64URL: true,
    });
    const pages = sheet.querySelectorAll("section.docx").length;
    countHost.textContent = pages ? `${pages} p` : "";
    docx = sheet;
    applyZoom();
    say({ kind: "opened", pages });
  }

  const ROWS = 500;
  const COLUMNS = 60;

  // Cached values only: a formula shows what the file last calculated, and nothing runs.
  async function openSheets(message) {
    for (const source of message.scripts) await run(source);
    const book = window.XLSX.read(new Uint8Array(message.bytes), { type: "array", cellFormula: false, cellHTML: false });
    pagesHost.textContent = "";
    const tabs = document.createElement("div");
    tabs.id = "sheets";
    const grid = document.createElement("div");
    grid.id = "grid";
    pagesHost.append(tabs, grid);
    const show = (name, button) => {
      for (const b of tabs.children) b.dataset.on = b === button ? "true" : "false";
      grid.textContent = "";
      const rows = window.XLSX.utils.sheet_to_json(book.Sheets[name], { header: 1, raw: false, defval: "", blankrows: false });
      const table = document.createElement("table");
      for (const row of rows.slice(0, ROWS)) {
        const tr = document.createElement("tr");
        for (const cell of row.slice(0, COLUMNS)) {
          const td = document.createElement("td");
          td.textContent = String(cell);
          tr.append(td);
        }
        table.append(tr);
      }
      grid.append(table);
      if (rows.length > ROWS) {
        const more = document.createElement("div");
        more.id = "note";
        more.textContent = `first ${ROWS} of ${rows.length} rows`;
        grid.append(more);
      }
    };
    book.SheetNames.forEach((name, index) => {
      const button = document.createElement("button");
      button.textContent = name;
      button.addEventListener("click", () => show(name, button));
      tabs.append(button);
      if (index === 0) show(name, button);
    });
    countHost.textContent = `${book.SheetNames.length} sheet${book.SheetNames.length === 1 ? "" : "s"}`;
    say({ kind: "opened", pages: book.SheetNames.length });
  }

  async function openSlides(message) {
    const blob = URL.createObjectURL(new Blob([message.lib], { type: "text/javascript" }));
    const lib = await import(blob);
    pagesHost.textContent = "";
    const stage = document.createElement("div");
    stage.id = "slides";
    pagesHost.append(stage);
    // No PDF fallback for embedded EMF and no network: what the file holds is what is drawn.
    const viewer = await lib.PptxViewer.open(message.bytes, stage, {
      renderMode: "list",
      zipLimits: lib.RECOMMENDED_ZIP_LIMITS,
      listOptions: { windowed: true },
      scrollContainer: pagesHost,
      pdfjs: false,
    });
    countHost.textContent = `${viewer.slideCount} slides`;
    say({ kind: "opened", pages: viewer.slideCount });
  }

  function applyZoom() {
    zoomHost.textContent = `${Math.round(scale * 100)}%`;
    if (docx) docx.style.zoom = String(scale);
  }

  function open(message) {
    if (message.type === "docx") return openDocx(message);
    if (message.type === "xlsx") return openSheets(message);
    if (message.type === "pptx") return openSlides(message);
    return openPdf(message);
  }

  document.getElementById("in").addEventListener("click", () => {
    if (pdf) { scale = Math.min(4, scale * 1.25); layout(); }
    else if (docx) { scale = Math.min(3, scale * 1.25); applyZoom(); }
  });
  document.getElementById("out").addEventListener("click", () => {
    if (pdf) { scale = Math.max(0.4, scale / 1.25); layout(); }
    else if (docx) { scale = Math.max(0.4, scale / 1.25); applyZoom(); }
  });

  window.addEventListener("message", (event) => {
    if (event.source !== parent) return;
    const message = event.data;
    if (message && message.kind === "open") open(message).catch(fail);
  });
  say({ kind: "ready" });
})();
