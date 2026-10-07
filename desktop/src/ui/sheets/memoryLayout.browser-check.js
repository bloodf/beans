// Run against a mounted production memory sheet in an offline browser fixture.
// The caller scrolls/focuses controls; this check never sends requests or changes form values.
export function assertMemorySheetLayout(fieldLabels, actionLabels) {
  const sheet = document.querySelector(".sheet-frame:last-child > .memory-service-sheet");
  if (!sheet) throw new Error("No mounted memory sheet");
  const body = sheet.querySelector(":scope > .sheet-content");
  const header = sheet.querySelector(":scope > .sheet-header");
  const footer = sheet.querySelector(":scope > .sheet-buttons");
  if (!body || !header || !footer) throw new Error("Missing memory sheet regions");
  const bounds = (element) => {
    const rect = element.getBoundingClientRect();
    return { top: rect.top, bottom: rect.bottom, left: rect.left, right: rect.right };
  };
  const sheetBounds = bounds(sheet), bodyBounds = bounds(body), footerBounds = bounds(footer);
  const failures = [], hits = [];
  const visible = (rect, clip) => rect.top >= clip.top - 1 && rect.bottom <= clip.bottom + 1 && rect.left >= clip.left - 1 && rect.right <= clip.right + 1;
  const viewport = { top: 0, left: 0, bottom: innerHeight, right: innerWidth };
  for (const [name, element] of [["header", header], ["footer", footer]]) {
    if (!visible(bounds(element), sheetBounds) || !visible(bounds(element), viewport)) failures.push(`${name} leaves the visible sheet`);
  }
  if (bodyBounds.bottom > footerBounds.top + 1) failures.push("body overlaps footer");
  for (const [kind, labels] of [["field", fieldLabels], ["action", actionLabels]]) {
    for (const label of labels) {
      const element = kind === "field"
        ? body.querySelector(`[aria-label="${CSS.escape(label)}"]`)
        : [...sheet.querySelectorAll("button")].find((button) => button.getAttribute("aria-label") === label || button.textContent.trim() === label);
      if (!element) { failures.push(`Missing ${kind}: ${label}`); continue; }
      const rect = bounds(element);
      const inBody = body.contains(element);
      if (!visible(rect, inBody ? bodyBounds : sheetBounds) || !visible(rect, viewport)) failures.push(`${label} is clipped`);
      if (inBody && rect.bottom > footerBounds.top + 1) failures.push(`${label} is occluded by footer`);
      const hit = document.elementFromPoint((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
      const receivesHit = !!hit && (element === hit || element.contains(hit));
      hits.push({ label, receivesHit, hit: hit?.getAttribute("aria-label") || hit?.textContent?.trim().slice(0, 60) || null });
      if (!receivesHit) failures.push(`${label} does not receive its center hit`);
    }
  }
  const result = { failures, hits, body: bodyBounds, footer: footerBounds, bodyScrollTop: body.scrollTop, bodyScrollMaximum: body.scrollHeight - body.clientHeight, sheetScrollTop: sheet.scrollTop };
  if (failures.length) throw new Error(JSON.stringify(result));
  return result;
}
