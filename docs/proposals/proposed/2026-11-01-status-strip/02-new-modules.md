# 02 — New modules

## Scope

Add four modules to the registry from 01, each off by default except where noted.

| id           | shows                                      | hover card                         | default                                                                                                                             |
| ------------ | ------------------------------------------ | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `permission` | 默认 / 自动 / 计划 with its icon           | the mode's meaning                 | off — deferred: the mode lives in the agent store, not in the view the bar receives, so it needs a prop threaded from the chat page |
| `todos`      | `n/m` done, plus `N 进行中` when items run | the open items, one `CardRow` each | on                                                                                                                                  |
| `subagents`  | live children, plus `等待中` count         | label + task per child             | on                                                                                                                                  |
| `cost`       | the turn's cost with the currency symbol   | in/out/cached unit prices          | off                                                                                                                                 |

## Rules

- A module with no data renders nothing (`hideWhenEmpty`) — no empty chip, no
  placeholder dash.
- Every value comes from state the shell already holds (`view`, `gitInfo`,
  `modelCost`, the agent store); no new IPC round trip on the render path.
- Each module subscribes to its own slice, so a streaming delta does not
  re-render the strip as a whole.

## Must not touch

- The four existing modules' markup.
- Chip styling beyond what a new value needs (existing tokens only).

## Done condition

- Each module shows its real value in a session that has one, and nothing in a
  session that does not.
- Hover cards open, pin and close exactly like the existing chips.
- Tests: `todos` and `subagents` counts from a fixture view; `permission` label
  per mode; `cost` hidden when the model declares no price.
