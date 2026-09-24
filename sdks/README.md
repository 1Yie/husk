# Hook SDKs

Write a husk plugin hook in the language you already have. A hook is a local
command: the kernel spawns it once per intercepted event, writes one JSON
payload to stdin, reads one JSON verdict from stdout, and degrades anything
else (non-zero exit, timeout, garbage) to `continue`. These libs wrap that
contract so a hook is just a handler function.

## Python

Vendor `python/husk_hooks.py` into your plugin dir (next to `manifest.json`):

```python
from husk_hooks import on, run, veto

@on("before_tool_execute")
def guard(p):
    if "push --force" in p["args"].get("command", ""):
        return veto("force-push needs a human")

run()
```

## TypeScript / JavaScript

Vendor `ts/husk-hooks.ts` (runs under `bun`, `node`, or `tsx` — zero deps):

```ts
import { on, run, veto } from "./husk-hooks";

on("before_tool_execute", (p) =>
  (p.args?.command as string)?.includes("push --force")
    ? veto("force-push needs a human")
    : undefined,          // undefined = continue
);

await run();
```

## manifest.json

The manifest is identical either way — `run.command` is whatever runs your
script, relative paths resolve against the plugin dir:

```jsonc
{
  "id": "git-guard",
  "name": "Git Guard",
  "capabilities": {
    "hooks": [{
      "event": "before_tool_execute",
      "filter": { "tool": "bash" },
      "run": { "command": "python3", "args": ["guard.py"] }
      //     or { "command": "bun", "args": ["guard.ts"] }
    }]
  }
}
```

## Verdicts

| event | stdin fields | verdicts |
|---|---|---|
| `on_user_input` | `input` | `cont()` / `block(reason)` / `inject(note)` |
| `before_tool_execute` | `tool`, `args` | `cont()` / `veto(reason)` / `rewrite(args)` |
| `after_tool_execute` | `tool`, `args`, `output` | `cont()` / `rewrite_output(output)` |
| `on_state_transition` | `old`, `new` | ignored — fire-and-forget |

One script can serve several events — `@on(...)` per event, the payload's
`event` field is the dispatcher. A handler that returns nothing means
continue; one that throws prints to stderr (the hook log) and continues.
