# Product

## Register

product

## Users

Developers who run Claude Code and Codex concurrently in local terminals and need to understand which agent is doing what without switching between every conversation.

## Product Purpose

Agent Flow is a read-only Rust terminal application. It discovers local session transcripts, connects tool calls to their results, and shows parent/child agent relationships alongside a navigable event flow.

## Brand Personality

Precise, quiet, immediate. Familiar keyboard navigation inspired by k9s.

## Anti-references

Decorative dashboards, charts that conceal actual events, and inferred activity presented as confirmed process state.

## Design Principles

- Start with the agent and its task; reveal full inputs and results on selection.
- Keep parallel agents in separate event streams; make recorded handoffs easy to follow.
- Keep provenance and uncertainty visible.
- Preserve selection while new events arrive; follow the tail only when requested.
- Make every primary operation available from the keyboard.
- Read existing logs without changing agent configuration or sending their contents anywhere.

## Accessibility & Inclusion

Status uses text and symbols as well as color. Use the terminal's own background and font, support small windows through panel switching, and display Unicode including Korean.
