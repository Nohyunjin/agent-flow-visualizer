# Agent Flow terminal design

## Scene and theme

A developer keeps this terminal beside existing Claude Code and Codex sessions and repeatedly glances between them. Preserve their terminal background, font, and configured ANSI palette so switching windows does not change the working environment's light level.

## Strategy

Restrained, terminal-native. Cyan marks focus and actor identity. Yellow marks pending activity, green returned results, red explicit errors, and dim neutral text secondary provenance. Status words accompany color. Inverted selection follows the terminal's own contrast settings.

## Layout

Start in Dashboard, with `d` switching to the three-panel Flow explorer. Dashboard ranks individual agents by slowest ended turn by default; `o` cycles total ended turn time, slowest tool and open turn age. At 125 columns use 56/44 percent session/turn lists; at 95–124 columns and sufficient height stack the lists; otherwise show the focused list. `Tab`, `1` and `2` select lists. Preserve selected session and turn identities across refresh and reordering.

Session rows show task, provider, Main/Sub, transcript status, total ended time, slowest turn/tool and open age. Turn rows show duration, ENDED/INTERRUPTED/OPEN/MISSING END and timestamp/reported provenance, followed by the request and slowest tool. Use `*` for partial history or timing gaps and `—` for unknown values. Time aggregation covers only retained events; never sum children into parents, tools into turns, or overlapping intervals twice. Open age is separate from ended totals. `Enter` opens the turn boundary in Flow, `x` its slowest tool, with follow off and event filters cleared so the requested event remains visible.

Three persistent contexts: agent tree, event flow, inspector. At 145 columns, use 25/39/36 percent columns. At 95 columns, retain a 32-column agent tree and stack flow/inspector. Below 95 columns, show the focused panel and retain the same keyboard controls. Minimum usable size is 42 columns by 12 rows.

Flow defaults to Agent Detail: only the selected agent's own events, with ancestry, task and direct-child count above. `s` explicitly enables the combined chronological list. `v` opens Parallel from either Dashboard or Flow. Parallel uses independent columns for one root family, ordered by ancestry and stable session key, with up to three columns of at least 44 cells each (or one full-width column on smaller terminals). The focused column stays visible as the user moves across a large family.

Parallel columns keep separate selection, scroll and Follow state. Their rows are not time-aligned; say this in the header and keep timestamps on events. Show task, parent, transcript status, visible/retained event count and context at the top of each column. Event filters apply across columns; agent-list filters do not hide members of the selected family. `Enter` and `v` open the focused event in Agent Detail with follow off; `3` opens its inspector. Never replace the selected event with a sibling's or the latest event during drill-down.

## Flow vocabulary

Tree branches express parentage. Timestamped three-line event rows express sequence. Spawn/message arrows identify recorded links using agent names; their preview shows the instruction, not a generic delivery receipt. The inspector shows the full paired input/result. Do not draw causal links between unrelated parallel agents. Full provenance lives below the event content.

Agent rows use four lines: task identity, provider/status/age, Main/Sub current context usage and capacity with percentage free, then retained event count (`ev`) and workspace or delegated task. Show the latest request's input plus output, including cached input, rather than cumulative processing. Each agent owns its context; parents do not include child usage. Prefix estimates with `~`, show unknown usage as `—`, and unknown capacity as `?` without a free percentage. Capacity comes from recorded limits, model tags, or supported model defaults, with the source explained in the inspector. Red at 10% free or less and yellow at 25% or less accompany the numeric headroom.

The inspector's CONTEXT AT EVENT is an immutable snapshot of the context known when the event occurred; later usage and tool results cannot change it. Tool rows use call-start context. Show request input/output, remaining tokens, the recorded timestamp, and whether capacity is a model default that settings may override. Compaction clears current usage until the next recorded request. Estimates exclude subsequent messages/tool output and do not promise how long remains until automatic compaction.

## Interaction

Focus uses a highlighted border and numbered panel title. Selection uses inversion and a leading chevron. Manual event navigation disables follow. Switching to another agent, including returning to a previous one or following an agent link, selects the latest event matching the current flow filters without changing follow mode. Snapshot refresh within the same agent preserves selected IDs when follow is off. Empty states explain applicable controls. The help view scrolls in small terminals.

The Agents tree starts with branches folded so large families cannot displace other main sessions. Prefix branches with `▸` / `▾` and the total loaded descendant count before the task name. `z` toggles the selected branch; `Z` folds all and returns a selected descendant to its visible root. Right/`l` expands, then enters the first child; Left/`h` folds, then moves to the parent. Tab and numbered pane navigation remain available. Keep expansion by session key during live updates, including when children arrive or sessions reorder. Search/provider/activity filter changes reveal matched paths; explicit agent drill-down reveals the selected path. Folding only changes tree visibility, never event content, timing, or Parallel lanes. Same-agent folding preserves event selection and Follow; returning to a root follows the normal agent-switch selection rule.
