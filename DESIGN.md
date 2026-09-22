# Agent Flow terminal design

## Scene and theme

A developer keeps this terminal beside existing Claude Code and Codex sessions and repeatedly glances between them. Preserve their terminal background, font, and configured ANSI palette so switching windows does not change the working environment's light level.

## Strategy

Restrained, terminal-native. Cyan marks focus and actor identity. Yellow marks pending activity, green returned results, red explicit errors, and dim neutral text secondary provenance. Status words accompany color. Inverted selection follows the terminal's own contrast settings.

## Layout

Three persistent contexts: agent tree, event flow, inspector. At 145 columns, use 25/39/36 percent columns. At 95 columns, retain a 32-column agent tree and stack flow/inspector. Below 95 columns, show the focused panel and retain the same keyboard controls. Minimum usable size is 42 columns by 12 rows.

## Flow vocabulary

Tree branches express parentage. Timestamped three-line event rows express sequence. Spawn arrows identify delegation, while the inspector shows paired input/result. Do not draw causal links between unrelated parallel agents. Full provenance lives below the event content.

Agent rows use four lines: task identity, provider/status/age, Main/Sub current context usage and capacity with percentage free, then retained event count (`ev`) and workspace or delegated task. Show the latest request's input plus output, including cached input, rather than cumulative processing. Each agent owns its context; parents do not include child usage. Prefix estimates with `~`, show unknown usage as `—`, and unknown capacity as `?` without a free percentage. Capacity comes from recorded limits, model tags, or supported model defaults, with the source explained in the inspector. Red at 10% free or less and yellow at 25% or less accompany the numeric headroom.

The inspector's CONTEXT AT EVENT is an immutable snapshot of the context known when the event occurred; later usage and tool results cannot change it. Tool rows use call-start context. Show request input/output, remaining tokens, the recorded timestamp, and whether capacity is a model default that settings may override. Compaction clears current usage until the next recorded request. Estimates exclude subsequent messages/tool output and do not promise how long remains until automatic compaction.

## Interaction

Focus uses a highlighted border and numbered panel title. Selection uses inversion and a leading chevron. Manual event navigation disables follow. Switching to another agent, including returning to a previous one or following an agent link, selects the latest event matching the current flow filters without changing follow mode. Snapshot refresh within the same agent preserves selected IDs when follow is off. Empty states explain applicable controls. The help view scrolls in small terminals.
