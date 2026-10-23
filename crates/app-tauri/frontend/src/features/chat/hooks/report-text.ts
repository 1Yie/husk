// A child's report as the UI shows it. The kernel's header line and a batch
// result's section marker are metadata for the parent model — they name the
// child and its cost, they are not the report. Stripped HERE, at display time
// only: the tool row keeps the raw text, so the model and the log are untouched.

/** The kernel's header on a report — `[subagent report — child_id 7 · 132 tool
 *  calls, 758s]`. */
const REPORT_HEADER = /^\[subagent report[^\]]*\]\s*/;

/** A batch result's per-task section marker (`── [i] ok · 3s ──`), which sits
 *  in front of that child's own header. */
const SECTION_MARKER = /^── \[\d+\][^\n]*──\s*/;

/** A report without its leading furniture. Only the START is stripped: the same
 *  wording inside a report is the child's own text and stays. */
export function reportBody(text: string): string {
  // Marker first — it wraps the header in a batch section.
  return text.trimStart().replace(SECTION_MARKER, "").replace(REPORT_HEADER, "").trim();
}
