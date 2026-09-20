// ChatSkeleton — the loading placeholder shown while a session's history
// is being fetched + rebuilt (openSession / workspace switch / fork).
// Mirrors the real stream's rhythm: a right-aligned user bubble, a few
// assistant text lines, a tool chip — so the swap reads as "content
// resolving", not a spinner.
export function ChatSkeleton() {
  const bar = "rounded-md bg-neutral-200/70";
  return (
    <div
      className="flex w-full flex-col gap-6 animate-pulse"
      aria-hidden="true"
    >
      {/* user bubble */}
      <div className={`ms-auto h-9 w-56 rounded-xl ${bar}`} />

      {/* assistant paragraph */}
      <div className="flex flex-col gap-2.5 pl-1">
        <div className={`h-4 w-3/4 ${bar}`} />
        <div className={`h-4 w-full ${bar}`} />
        <div className={`h-4 w-5/6 ${bar}`} />
        <div className={`h-4 w-1/2 ${bar}`} />
      </div>

      {/* tool chip */}
      <div className={`h-8 w-44 rounded-lg ${bar}`} />

      {/* user bubble */}
      <div className={`ms-auto h-9 w-40 rounded-xl ${bar}`} />

      {/* assistant paragraph */}
      <div className="flex flex-col gap-2.5 pl-1">
        <div className={`h-4 w-full ${bar}`} />
        <div className={`h-4 w-2/3 ${bar}`} />
        <div className={`h-4 w-4/5 ${bar}`} />
      </div>
    </div>
  );
}
