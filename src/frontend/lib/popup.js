// Unified popup/overlay management.
// Handles positioning, click-outside-close, Escape-to-dismiss.
// Only one popup open at a time.

let activePopup = null;
let cleanupFns = null;

export function closeActive() {
  if (!activePopup) return;
  activePopup.remove();
  if (cleanupFns) {
    cleanupFns();
    cleanupFns = null;
  }
  activePopup = null;
}

export function showPopup(targetEl, contentEl, opts = {}) {
  closeActive();

  document.body.appendChild(contentEl);
  activePopup = contentEl;

  // Position
  if (opts.anchor === "cursor" && opts.x != null && opts.y != null) {
    contentEl.style.position = "fixed";
    contentEl.style.zIndex = "9999";

    // Place at cursor, then adjust if off-screen
    let left = opts.x;
    let top = opts.y;

    // Use RAF to measure after append
    requestAnimationFrame(() => {
      const rect = contentEl.getBoundingClientRect();
      if (left + rect.width > window.innerWidth) left = left - rect.width;
      if (top + rect.height > window.innerHeight) top = top - rect.height;
      contentEl.style.left = Math.max(0, left) + "px";
      contentEl.style.top = Math.max(0, top) + "px";
    });

    contentEl.style.left = left + "px";
    contentEl.style.top = top + "px";
  } else if (targetEl) {
    const rect = targetEl.getBoundingClientRect();
    contentEl.style.position = "fixed";
    contentEl.style.zIndex = "9999";
    contentEl.style.left = rect.left + "px";
    contentEl.style.top = rect.bottom + 4 + "px";
  }

  // Dismiss on Escape
  const onKey = (e) => {
    if (e.key === "Escape") closeActive();
  };

  // Dismiss on click outside
  const onClick = (e) => {
    if (!contentEl.contains(e.target)) closeActive();
  };

  // Delay listener registration to avoid the triggering click
  requestAnimationFrame(() => {
    document.addEventListener("click", onClick, true);
    document.addEventListener("contextmenu", onClick, true);
  });
  document.addEventListener("keydown", onKey);

  cleanupFns = () => {
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("contextmenu", onClick, true);
    document.removeEventListener("keydown", onKey);
    if (opts.onClose) opts.onClose();
  };
}
