//! Independent event lanes for one agent family. Rows across lanes are not time-aligned.
use crate::model::{FlowEvent, Outcome, Snapshot};
use ratatui::widgets::ListState;
use std::collections::{HashMap, HashSet};

pub fn matches_event(e: &FlowEvent, tools: bool, errors: bool, query: &str) -> bool {
    (!tools || e.is_tool())
        && (!errors || e.outcome == Outcome::Error)
        && (query.is_empty()
            || format!(
                "{} {} {}",
                e.name,
                e.input,
                e.output.as_deref().unwrap_or("")
            )
            .to_lowercase()
            .contains(query))
}

pub struct Lane {
    pub key: String,
    pub session: usize,
    pub events: Vec<usize>,
    pub state: ListState,
    pub follow: bool,
    selected_id: Option<String>,
}

#[derive(Default)]
pub struct Parallel {
    pub visible: bool,
    pub root: Option<String>,
    pub lanes: Vec<Lane>,
    pub focus: usize,
}

impl Parallel {
    pub fn open(&mut self, snapshot: &Snapshot, key: &str) {
        let mut root = key.to_owned();
        let mut seen = HashSet::new();
        while seen.insert(root.clone()) {
            let Some(parent) = snapshot
                .sessions
                .iter()
                .find(|s| s.key == root)
                .and_then(|s| s.parent.as_ref())
            else {
                break;
            };
            if seen.contains(parent) {
                break;
            }
            root = parent.clone();
        }
        if self.root.as_ref() != Some(&root) {
            self.lanes.clear();
        }
        self.root = Some(root);
        self.visible = true;
    }

    pub fn rebuild(&mut self, snapshot: &Snapshot, tools: bool, errors: bool, query: &str) {
        let focus_key = self.lanes.get(self.focus).map(|l| l.key.clone());
        let mut previous: HashMap<_, _> =
            self.lanes.drain(..).map(|l| (l.key.clone(), l)).collect();
        let Some(root) = self.root.as_ref() else {
            return;
        };
        let mut family = HashSet::from([root.clone()]);
        loop {
            let count = family.len();
            for s in &snapshot.sessions {
                if s.parent.as_ref().is_some_and(|p| family.contains(p)) {
                    family.insert(s.key.clone());
                }
            }
            if family.len() == count {
                break;
            }
        }
        // Stable ancestry order: unrelated snapshot recency changes cannot shuffle lanes.
        let mut indices: Vec<_> = snapshot
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| family.contains(&s.key))
            .map(|(i, _)| i)
            .collect();
        indices.sort_by(|a, b| snapshot.sessions[*a].key.cmp(&snapshot.sessions[*b].key));
        let mut order = Vec::new();
        let mut visited = HashSet::new();
        fn visit(
            key: &str,
            snapshot: &Snapshot,
            indices: &[usize],
            visited: &mut HashSet<String>,
            order: &mut Vec<usize>,
        ) {
            if !visited.insert(key.to_owned()) {
                return;
            }
            if let Some(i) = indices.iter().find(|i| snapshot.sessions[**i].key == key) {
                order.push(*i);
            }
            for i in indices {
                if snapshot.sessions[*i].parent.as_deref() == Some(key) {
                    visit(
                        &snapshot.sessions[*i].key,
                        snapshot,
                        indices,
                        visited,
                        order,
                    );
                }
            }
        }
        visit(root, snapshot, &indices, &mut visited, &mut order);
        let query = query.to_lowercase();
        for i in order {
            let session = &snapshot.sessions[i];
            let mut lane = previous.remove(&session.key).unwrap_or_else(|| Lane {
                key: session.key.clone(),
                session: i,
                events: vec![],
                state: ListState::default(),
                follow: true,
                selected_id: None,
            });
            lane.session = i;
            lane.events = session
                .events
                .iter()
                .enumerate()
                .filter(|(_, e)| matches_event(e, tools, errors, &query))
                .map(|(i, _)| i)
                .collect();
            lane.events.sort_by_key(|i| (session.events[*i].time, *i));
            let selected = if lane.follow {
                lane.events.len().checked_sub(1)
            } else {
                lane.events
                    .iter()
                    .position(|i| Some(&session.events[*i].id) == lane.selected_id.as_ref())
                    .or_else(|| {
                        (!lane.events.is_empty()).then(|| {
                            lane.state
                                .selected()
                                .unwrap_or(0)
                                .min(lane.events.len() - 1)
                        })
                    })
            };
            lane.state.select(selected);
            lane.selected_id = selected.map(|n| session.events[lane.events[n]].id.clone());
            self.lanes.push(lane);
        }
        self.focus = self
            .lanes
            .iter()
            .position(|l| Some(&l.key) == focus_key.as_ref())
            .unwrap_or_else(|| self.focus.min(self.lanes.len().saturating_sub(1)));
    }

    pub fn select(&mut self, snapshot: &Snapshot, key: &str, event: Option<&str>) {
        if let Some(i) = self.lanes.iter().position(|l| l.key == key) {
            self.focus = i;
            let lane = &mut self.lanes[i];
            if let Some(n) = event.and_then(|id| {
                lane.events
                    .iter()
                    .position(|e| snapshot.sessions[lane.session].events[*e].id == id)
            }) {
                lane.state.select(Some(n));
                lane.selected_id = event.map(str::to_owned);
                lane.follow = false;
            }
        }
    }

    pub fn move_lane(&mut self, delta: isize) {
        self.focus = self
            .focus
            .saturating_add_signed(delta)
            .min(self.lanes.len().saturating_sub(1));
    }

    pub fn move_event(&mut self, snapshot: &Snapshot, delta: isize) {
        if let Some(lane) = self.lanes.get_mut(self.focus) {
            if lane.events.is_empty() {
                return;
            }
            let i = lane
                .state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(lane.events.len() - 1);
            lane.state.select(Some(i));
            lane.selected_id = Some(
                snapshot.sessions[lane.session].events[lane.events[i]]
                    .id
                    .clone(),
            );
            lane.follow = false;
        }
    }

    pub fn target(&self) -> Option<(String, Option<String>)> {
        let lane = self.lanes.get(self.focus)?;
        Some((lane.key.clone(), lane.selected_id.clone()))
    }
}
