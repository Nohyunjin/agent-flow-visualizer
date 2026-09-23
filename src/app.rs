use crate::{
    dashboard::Dashboard,
    model::*,
    parallel::{Parallel, matches_event},
    source::resolve_target,
};
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Agents,
    Flow,
    Detail,
}
impl Pane {
    pub fn next(self) -> Self {
        match self {
            Self::Agents => Self::Flow,
            Self::Flow => Self::Detail,
            Self::Detail => Self::Agents,
        }
    }
    pub fn previous(self) -> Self {
        self.next().next()
    }
}
#[derive(Clone, Debug)]
pub struct AgentRow {
    pub index: usize,
    pub depth: usize,
    pub descendants: Vec<usize>,
    pub expanded: bool,
}
impl AgentRow {
    pub fn working_descendants(&self, snapshot: &Snapshot, now: DateTime<Utc>) -> usize {
        self.descendants
            .iter()
            .filter(|&&index| snapshot.sessions[index].status(now) == "WORKING")
            .count()
    }
}

pub struct App {
    pub dashboard: Dashboard,
    pub parallel: Parallel,
    pub snapshot: Snapshot,
    pub pane: Pane,
    pub agents: Vec<AgentRow>,
    expanded_agents: HashSet<String>,
    agent_filter: (String, Option<Provider>, bool),
    pub flow: Vec<(usize, usize)>,
    pub agent_state: ListState,
    pub flow_state: ListState,
    pub selected_key: Option<String>,
    pub detail_scroll: u16,
    pub detail_max_scroll: u16,
    pub follow: bool,
    pub paused: bool,
    pub subtree: bool,
    pub tools_only: bool,
    pub errors_only: bool,
    pub active_only: bool,
    pub provider: Option<Provider>,
    pub agent_query: String,
    pub event_query: String,
    pub search: Option<Pane>,
    pub help: bool,
    pub help_scroll: u16,
    pub demo: bool,
    pub notice: String,
    pub refresh: bool,
}

impl App {
    pub fn new(snapshot: Snapshot, demo: bool) -> Self {
        let mut app = Self {
            dashboard: Dashboard::default(),
            parallel: Parallel::default(),
            snapshot,
            pane: Pane::Agents,
            agents: vec![],
            expanded_agents: HashSet::new(),
            agent_filter: (String::new(), None, false),
            flow: vec![],
            agent_state: ListState::default(),
            flow_state: ListState::default(),
            selected_key: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            follow: true,
            paused: false,
            subtree: false,
            tools_only: false,
            errors_only: false,
            active_only: false,
            provider: None,
            agent_query: String::new(),
            event_query: String::new(),
            search: None,
            help: false,
            help_scroll: 0,
            demo,
            notice: String::new(),
            refresh: false,
        };
        app.rebuild();
        app
    }
    pub fn selected_session(&self) -> Option<&Session> {
        self.selected_key.as_ref().and_then(|key| {
            self.snapshot
                .sessions
                .iter()
                .find(|s| &s.key == key)
                .map(|s| s.as_ref())
        })
    }
    pub fn selected_event(&self) -> Option<(&Session, &FlowEvent)> {
        let (si, ei) = *self.flow.get(self.flow_state.selected()?)?;
        let s = self.snapshot.sessions.get(si)?;
        Some((s, s.events.get(ei)?))
    }
    fn event_key(&self) -> Option<(String, String)> {
        self.selected_event()
            .map(|(s, e)| (s.key.clone(), e.id.clone()))
    }
    pub fn update(&mut self, snapshot: Snapshot) {
        let selected = self.event_key();
        self.snapshot = snapshot;
        self.rebuild_with_event(selected);
    }
    pub fn rebuild(&mut self) {
        let selected = self.event_key();
        self.rebuild_with_event(selected);
    }
    fn rebuild_with_event(&mut self, selected: Option<(String, String)>) {
        let query = self.agent_query.to_lowercase();
        let mut visible: HashSet<usize> = self
            .snapshot
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                self.provider.is_none_or(|p| p == s.provider)
                    && (!self.active_only || s.status(Utc::now()) == "WORKING")
                    && (query.is_empty()
                        || format!("{} {} {} {}", s.title, s.cwd, s.id, s.agent_path)
                            .to_lowercase()
                            .contains(&query))
            })
            .map(|(i, _)| i)
            .collect();
        let matched: Vec<_> = visible.iter().copied().collect();
        let filter = (self.agent_query.clone(), self.provider, self.active_only);
        if self.agent_filter != filter {
            self.agent_filter = filter;
            if !query.is_empty() || self.provider.is_some() || self.active_only {
                for i in &matched {
                    self.reveal_agent(&self.snapshot.sessions[*i].key.clone());
                }
            }
        }
        if let Some(key) = self.selected_key.clone() {
            self.reveal_agent(&key);
        }
        for i in matched {
            let mut current = i;
            let mut seen = HashSet::new();
            while seen.insert(current) {
                let Some(parent) = &self.snapshot.sessions[current].parent else {
                    break;
                };
                let Some(index) = self.snapshot.sessions.iter().position(|s| &s.key == parent)
                else {
                    break;
                };
                visible.insert(index);
                current = index;
            }
        }
        let mut all_rows = Vec::new();
        let mut emitted = HashSet::new();
        for i in 0..self.snapshot.sessions.len() {
            let s = &self.snapshot.sessions[i];
            if visible.contains(&i)
                && !s.parent.as_ref().is_some_and(|p| {
                    self.snapshot
                        .sessions
                        .iter()
                        .enumerate()
                        .any(|(j, s)| &s.key == p && visible.contains(&j))
                })
            {
                append_tree(&self.snapshot, i, 0, &visible, &mut emitted, &mut all_rows);
            }
        }
        for i in 0..self.snapshot.sessions.len() {
            if visible.contains(&i) && !emitted.contains(&i) {
                append_tree(&self.snapshot, i, 0, &visible, &mut emitted, &mut all_rows);
            }
        }
        // Build the complete visible tree first, so folded descendants cannot be
        // reintroduced as apparent roots by the orphan/cycle fallback above.
        let by_key: HashMap<_, _> = self
            .snapshot
            .sessions
            .iter()
            .enumerate()
            .map(|(i, s)| (s.key.as_str(), i))
            .collect();
        let mut descendants = vec![Vec::new(); self.snapshot.sessions.len()];
        for (index, session) in self.snapshot.sessions.iter().enumerate() {
            let mut parent = session.parent.as_deref();
            let mut seen = HashSet::from([session.key.as_str()]);
            while let Some(key) = parent {
                if !seen.insert(key) {
                    break;
                }
                let Some(i) = by_key.get(key).copied() else {
                    break;
                };
                descendants[i].push(index);
                parent = self.snapshot.sessions[i].parent.as_deref();
            }
        }
        self.agents.clear();
        let mut folded_depth = None;
        for mut row in all_rows {
            if folded_depth.is_some_and(|depth| row.depth > depth) {
                continue;
            }
            row.descendants = std::mem::take(&mut descendants[row.index]);
            row.expanded = self
                .expanded_agents
                .contains(&self.snapshot.sessions[row.index].key);
            folded_depth = (!row.expanded).then_some(row.depth);
            self.agents.push(row);
        }
        let selected_agent = self
            .agents
            .iter()
            .position(|r| Some(&self.snapshot.sessions[r.index].key) == self.selected_key.as_ref())
            .or({
                if self.agents.is_empty() {
                    None
                } else {
                    Some(0)
                }
            });
        self.agent_state.select(selected_agent);
        let selected_key =
            selected_agent.map(|i| self.snapshot.sessions[self.agents[i].index].key.clone());
        let selected = if self.selected_key == selected_key {
            selected
        } else {
            self.detail_scroll = 0;
            None
        };
        self.selected_key = selected_key;
        self.rebuild_flow(selected);
        self.dashboard.rebuild(
            &self.snapshot,
            self.provider,
            &self.agent_query,
            self.active_only,
        );
        if self.parallel.visible {
            self.parallel.rebuild(
                &self.snapshot,
                self.tools_only,
                self.errors_only,
                &self.event_query,
            );
        }
    }
    fn reveal_agent(&mut self, key: &str) {
        let mut current = key.to_owned();
        let mut seen = HashSet::from([current.clone()]);
        while let Some(parent) = self
            .snapshot
            .sessions
            .iter()
            .find(|s| s.key == current)
            .and_then(|s| s.parent.clone())
        {
            if !seen.insert(parent.clone()) {
                break;
            }
            self.expanded_agents.insert(parent.clone());
            current = parent;
        }
    }
    fn fold_agent(&mut self, expand: Option<bool>) {
        let Some(index) = self.agent_state.selected() else {
            return;
        };
        let Some(row) = self.agents.get(index) else {
            return;
        };
        let key = self.snapshot.sessions[row.index].key.clone();
        let is_expanded = row.expanded;
        if !row.descendants.is_empty() && expand.unwrap_or(!is_expanded) != is_expanded {
            if is_expanded {
                self.expanded_agents.remove(&key);
            } else {
                self.expanded_agents.insert(key);
            }
            self.rebuild();
        } else if expand == Some(false)
            && let Some(parent) = self.selected_session().and_then(|s| s.parent.clone())
            && let Some(i) = self
                .agents
                .iter()
                .position(|r| self.snapshot.sessions[r.index].key == parent)
        {
            self.choose_agent(i);
        } else if expand == Some(true)
            && is_expanded
            && self
                .agents
                .get(index + 1)
                .is_some_and(|next| next.depth > row.depth)
        {
            self.choose_agent(index + 1);
        }
    }
    fn fold_all_agents(&mut self) {
        let selected = self.event_key();
        let before = self.selected_key.clone();
        if let Some(i) = self.agent_state.selected()
            && let Some(root) = self.agents[..=i].iter().rev().find(|r| r.depth == 0)
        {
            self.selected_key = Some(self.snapshot.sessions[root.index].key.clone());
        }
        self.expanded_agents.clear();
        if before != self.selected_key {
            self.detail_scroll = 0;
        }
        self.rebuild_with_event(if before == self.selected_key {
            selected
        } else {
            None
        });
    }
    fn rebuild_flow(&mut self, selected: Option<(String, String)>) {
        self.flow.clear();
        let Some(key) = self.selected_key.clone() else {
            self.flow_state.select(None);
            return;
        };
        let mut included = HashSet::from([key]);
        if self.subtree {
            loop {
                let before = included.len();
                for s in &self.snapshot.sessions {
                    if s.parent.as_ref().is_some_and(|p| included.contains(p)) {
                        included.insert(s.key.clone());
                    }
                }
                if before == included.len() {
                    break;
                }
            }
        }
        let query = self.event_query.to_lowercase();
        for (si, s) in self.snapshot.sessions.iter().enumerate() {
            if !included.contains(&s.key) {
                continue;
            }
            for (ei, e) in s.events.iter().enumerate() {
                if !matches_event(e, self.tools_only, self.errors_only, &query) {
                    continue;
                }
                self.flow.push((si, ei));
            }
        }
        self.flow.sort_by(|(a, b), (c, d)| {
            self.snapshot.sessions[*a].events[*b]
                .time
                .cmp(&self.snapshot.sessions[*c].events[*d].time)
                .then(a.cmp(c))
                .then(b.cmp(d))
        });
        let index = if self.flow.is_empty() {
            None
        } else if self.follow || selected.is_none() {
            // A new agent view starts at its latest visible event, even with follow off.
            Some(self.flow.len() - 1)
        } else {
            selected
                .and_then(|(sk, ek)| {
                    self.flow.iter().position(|(s, e)| {
                        self.snapshot.sessions[*s].key == sk
                            && self.snapshot.sessions[*s].events[*e].id == ek
                    })
                })
                .or_else(|| {
                    Some(
                        self.flow_state
                            .selected()
                            .unwrap_or(0)
                            .min(self.flow.len() - 1),
                    )
                })
        };
        self.flow_state.select(index);
    }
    fn choose_agent(&mut self, index: usize) {
        if let Some(row) = self.agents.get(index) {
            let key = self.snapshot.sessions[row.index].key.clone();
            if self.selected_key.as_ref() == Some(&key) {
                return;
            }
            self.selected_key = Some(key);
            self.agent_state.select(Some(index));
            self.detail_scroll = 0;
            self.rebuild_flow(None);
        }
    }
    pub fn jump_to(&mut self, key: &str) {
        let selected = if self.selected_key.as_deref() == Some(key) {
            self.event_key()
        } else {
            None
        };
        // Navigation to a linked agent must not be silently blocked by a filter.
        self.provider = None;
        self.active_only = false;
        self.agent_query.clear();
        self.selected_key = Some(key.into());
        self.rebuild_with_event(selected);
        self.pane = Pane::Flow;
        self.detail_scroll = 0;
    }
    fn open_dashboard_target(&mut self, tool: bool) {
        if let Some((key, event)) = self.dashboard.target(tool) {
            self.parallel.visible = false;
            self.subtree = false;
            // Explicit drill-down must reveal its exact event even under stale Flow filters.
            self.tools_only = false;
            self.errors_only = false;
            self.event_query.clear();
            self.follow = false;
            self.selected_key = Some(key.clone());
            self.rebuild_with_event(Some((key, event)));
            self.pane = Pane::Flow;
            self.detail_scroll = 0;
            self.dashboard.visible = false;
            self.notice.clear();
        } else {
            self.notice = if tool {
                "No timed tool call in this selection."
            } else {
                "No retained turn boundary in this session."
            }
            .into();
        }
    }
    pub fn toggle_parallel(&mut self) {
        if self.parallel.visible && !self.dashboard.visible {
            self.open_parallel_target(Pane::Flow);
            return;
        }
        let selected = if self.dashboard.visible {
            self.dashboard.selected().map(|r| (r.key.clone(), None))
        } else {
            self.event_key()
                .map(|(key, event)| (key, Some(event)))
                .or_else(|| self.selected_key.clone().map(|key| (key, None)))
        };
        if let Some((key, event)) = selected {
            self.parallel.open(&self.snapshot, &key);
            self.parallel.rebuild(
                &self.snapshot,
                self.tools_only,
                self.errors_only,
                &self.event_query,
            );
            self.parallel.select(&self.snapshot, &key, event.as_deref());
            self.dashboard.visible = false;
            self.notice.clear();
        }
    }
    fn open_parallel_target(&mut self, pane: Pane) {
        let target = self.parallel.target();
        self.parallel.visible = false;
        self.subtree = false;
        if let Some((key, event)) = target {
            // A sibling remains reachable even if the agent tree was filtered before entry.
            self.provider = None;
            self.active_only = false;
            self.agent_query.clear();
            self.follow = false;
            self.selected_key = Some(key.clone());
            self.rebuild_with_event(event.map(|event| (key, event)));
        }
        self.pane = pane;
        self.detail_scroll = 0;
        self.notice.clear();
    }
    fn movement(&mut self, delta: isize) {
        match self.pane {
            Pane::Agents => {
                if !self.agents.is_empty() {
                    self.choose_agent(
                        self.agent_state
                            .selected()
                            .unwrap_or(0)
                            .saturating_add_signed(delta)
                            .min(self.agents.len() - 1),
                    );
                }
            }
            Pane::Flow => {
                if !self.flow.is_empty() {
                    self.flow_state.select(Some(
                        self.flow_state
                            .selected()
                            .unwrap_or(0)
                            .saturating_add_signed(delta)
                            .min(self.flow.len() - 1),
                    ));
                    self.follow = false;
                    self.detail_scroll = 0;
                }
            }
            Pane::Detail => {
                self.detail_scroll = self
                    .detail_scroll
                    .saturating_add_signed(delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16)
                    .min(self.detail_max_scroll);
            }
        }
    }
    /// Returns true only when the user requests to exit.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        if self.help {
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = self.help_scroll.saturating_add(1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                KeyCode::PageDown => self.help_scroll = self.help_scroll.saturating_add(10),
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                _ => self.help = false,
            }
            return false;
        }
        if let Some(pane) = self.search {
            let query = if pane == Pane::Agents {
                &mut self.agent_query
            } else {
                &mut self.event_query
            };
            match key.code {
                KeyCode::Esc => {
                    query.clear();
                    self.search = None;
                }
                KeyCode::Enter => self.search = None,
                KeyCode::Backspace => {
                    query.pop();
                }
                KeyCode::Char(c) => query.push(c),
                _ => {}
            }
            self.rebuild();
            return false;
        }
        if key.code == KeyCode::Char('d') {
            self.dashboard.visible = !self.dashboard.visible;
            self.notice.clear();
            return false;
        }
        if key.code == KeyCode::Char('v') {
            self.toggle_parallel();
            return false;
        }
        if self.parallel.visible && !self.dashboard.visible {
            match key.code {
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.parallel.move_lane(1),
                KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                    self.parallel.move_lane(-1)
                }
                KeyCode::Down | KeyCode::Char('j' | ']') => {
                    self.parallel.move_event(&self.snapshot, 1)
                }
                KeyCode::Up | KeyCode::Char('k' | '[') => {
                    self.parallel.move_event(&self.snapshot, -1)
                }
                KeyCode::PageDown => self.parallel.move_event(&self.snapshot, 10),
                KeyCode::PageUp => self.parallel.move_event(&self.snapshot, -10),
                KeyCode::Home | KeyCode::Char('g') => {
                    self.parallel.move_event(&self.snapshot, -1_000_000)
                }
                KeyCode::End | KeyCode::Char('G') => {
                    self.parallel.move_event(&self.snapshot, 1_000_000)
                }
                KeyCode::Enter | KeyCode::Char('2') => self.open_parallel_target(Pane::Flow),
                KeyCode::Char('1') => self.open_parallel_target(Pane::Agents),
                KeyCode::Char('3') => self.open_parallel_target(Pane::Detail),
                KeyCode::Char('/') => self.search = Some(Pane::Flow),
                KeyCode::Char('f') => {
                    if let Some(lane) = self.parallel.lanes.get_mut(self.parallel.focus) {
                        lane.follow = !lane.follow;
                    }
                    self.parallel.rebuild(
                        &self.snapshot,
                        self.tools_only,
                        self.errors_only,
                        &self.event_query,
                    );
                }
                KeyCode::Backspace | KeyCode::Char('b') => {
                    let parent = self
                        .parallel
                        .lanes
                        .get(self.parallel.focus)
                        .and_then(|l| self.snapshot.sessions[l.session].parent.clone());
                    if let Some(parent) = parent {
                        self.parallel.select(&self.snapshot, &parent, None);
                    }
                }
                KeyCode::Char('q' | '?' | ' ' | 't' | 'e' | 'r') | KeyCode::Esc => {}
                _ => return false,
            }
            if !matches!(
                key.code,
                KeyCode::Char('q' | '?' | ' ' | 't' | 'e' | 'r') | KeyCode::Esc
            ) {
                return false;
            }
        }
        if self.dashboard.visible {
            match key.code {
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
                    self.dashboard.focus_turns = !self.dashboard.focus_turns
                }
                KeyCode::Char('1') => self.dashboard.focus_turns = false,
                KeyCode::Char('2') => self.dashboard.focus_turns = true,
                KeyCode::Down | KeyCode::Char('j') => self.dashboard.movement(1),
                KeyCode::Up | KeyCode::Char('k') => self.dashboard.movement(-1),
                KeyCode::PageDown => self.dashboard.movement(10),
                KeyCode::PageUp => self.dashboard.movement(-10),
                KeyCode::Home | KeyCode::Char('g') => self.dashboard.movement(-1_000_000),
                KeyCode::End | KeyCode::Char('G') => self.dashboard.movement(1_000_000),
                KeyCode::Enter => self.open_dashboard_target(false),
                KeyCode::Char('x') => self.open_dashboard_target(true),
                KeyCode::Char('o') => {
                    self.dashboard.sort = self.dashboard.sort.next();
                    self.dashboard.rebuild(
                        &self.snapshot,
                        self.provider,
                        &self.agent_query,
                        self.active_only,
                    );
                }
                KeyCode::Char('/') => self.search = Some(Pane::Agents),
                KeyCode::Char('q' | '?' | ' ' | 'a' | 'p' | 'r') | KeyCode::Esc => {}
                _ => return false,
            }
            if !matches!(
                key.code,
                KeyCode::Char('q' | '?' | ' ' | 'a' | 'p' | 'r') | KeyCode::Esc
            ) {
                return false;
            }
        }
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Tab => self.pane = self.pane.next(),
            KeyCode::BackTab => self.pane = self.pane.previous(),
            KeyCode::Char('1') => self.pane = Pane::Agents,
            KeyCode::Char('2') => self.pane = Pane::Flow,
            KeyCode::Char('3') => self.pane = Pane::Detail,
            KeyCode::Left | KeyCode::Char('h') if self.pane == Pane::Agents => {
                self.fold_agent(Some(false))
            }
            KeyCode::Right | KeyCode::Char('l') if self.pane == Pane::Agents => {
                self.fold_agent(Some(true))
            }
            KeyCode::Char('z') if self.pane == Pane::Agents => self.fold_agent(None),
            KeyCode::Char('Z') if self.pane == Pane::Agents => self.fold_all_agents(),
            KeyCode::Left | KeyCode::Char('h') => self.pane = self.pane.previous(),
            KeyCode::Right | KeyCode::Char('l') => self.pane = self.pane.next(),
            KeyCode::Down | KeyCode::Char('j') => self.movement(1),
            KeyCode::Up | KeyCode::Char('k') => self.movement(-1),
            KeyCode::PageDown => self.movement(10),
            KeyCode::PageUp => self.movement(-10),
            KeyCode::Home | KeyCode::Char('g') => self.movement(-1_000_000),
            KeyCode::End | KeyCode::Char('G') => self.movement(1_000_000),
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let pane = self.pane;
                self.pane = Pane::Flow;
                self.movement(if key.code == KeyCode::Char('[') {
                    -1
                } else {
                    1
                });
                self.pane = pane;
            }
            KeyCode::Enter => {
                if self.pane == Pane::Agents {
                    self.pane = Pane::Flow;
                } else {
                    let target = self.selected_event().and_then(|(s, e)| {
                        e.target
                            .as_ref()
                            .and_then(|t| resolve_target(&self.snapshot, s, t))
                    });
                    if let Some(target) = target {
                        self.jump_to(&target);
                    } else {
                        self.pane = Pane::Detail;
                        self.detail_scroll = 0;
                    }
                }
            }
            KeyCode::Backspace | KeyCode::Char('b') => {
                if let Some(parent) = self.selected_session().and_then(|s| s.parent.clone()) {
                    self.jump_to(&parent);
                }
            }
            KeyCode::Char('/') => self.search = Some(self.pane),
            KeyCode::Esc => {
                self.agent_query.clear();
                self.event_query.clear();
                self.rebuild();
            }
            KeyCode::Char('f') => {
                self.follow = !self.follow;
                self.rebuild();
            }
            KeyCode::Char(' ') => self.paused = !self.paused,
            KeyCode::Char('s') => {
                self.subtree = !self.subtree;
                self.rebuild();
            }
            KeyCode::Char('t') => {
                self.tools_only = !self.tools_only;
                self.rebuild();
            }
            KeyCode::Char('e') => {
                self.errors_only = !self.errors_only;
                self.rebuild();
            }
            KeyCode::Char('a') => {
                self.active_only = !self.active_only;
                self.rebuild();
            }
            KeyCode::Char('p') => {
                self.provider = match self.provider {
                    None => Some(Provider::Codex),
                    Some(Provider::Codex) => Some(Provider::Claude),
                    Some(Provider::Claude) => None,
                };
                self.rebuild();
            }
            KeyCode::Char('r') => self.refresh = true,
            _ => {}
        }
        false
    }
}

fn append_tree(
    snapshot: &Snapshot,
    i: usize,
    depth: usize,
    visible: &HashSet<usize>,
    emitted: &mut HashSet<usize>,
    rows: &mut Vec<AgentRow>,
) {
    if !emitted.insert(i) {
        return;
    }
    rows.push(AgentRow {
        index: i,
        depth,
        descendants: Vec::new(),
        expanded: false,
    });
    for child in 0..snapshot.sessions.len() {
        if visible.contains(&child)
            && snapshot.sessions[child].parent.as_ref() == Some(&snapshot.sessions[i].key)
        {
            append_tree(snapshot, child, depth + 1, visible, emitted, rows);
        }
    }
}
