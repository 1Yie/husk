import { useEffect, useState } from "react";

/** Detail body that mounts its children only after the row has been
 * opened once — the grid collapse is CSS-only (`0fr` + overflow-hidden),
 * so without this every collapsed row still paid a full DiffView /
 * DetailLines render at mount (hundreds of highlighted lines per
 * tool call — the actual ~50-160ms-per-turn mount cost). Once opened it
 * stays mounted so the collapse animation keeps working. */
export function RowDetail({ open, children }: { open: boolean; children: React.ReactNode }) {
  const [everOpened, setEverOpened] = useState(open);
  useEffect(() => {
    if (open) setEverOpened(true);
  }, [open]);
  return (
    <div
      className="grid transition-[grid-template-rows,opacity] duration-250 ease-out w-full"
      style={{
        gridTemplateRows: open ? "1fr" : "0fr",
        opacity: open ? 1 : 0,
      }}
    >
      <div className="min-h-0 overflow-hidden w-full">
        <div className="w-full pt-1 pb-1.5">
          {everOpened ? children : null}
        </div>
      </div>
    </div>
  );
}
