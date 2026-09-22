use crate::{model::*, parser::parse_line};
use anyhow::{Context, Result};
use chrono::Utc;
use std::{
    collections::{HashMap, HashSet},
    fs::{File, Metadata},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::Arc,
    time::SystemTime,
};
use walkdir::WalkDir;

const READ_BUDGET: u64 = 8 * 1024 * 1024;
const MAX_LINE: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SourceConfig {
    pub roots: Vec<(Provider, PathBuf)>,
    pub max_sessions: usize,
    pub max_events: usize,
}

struct Tail {
    session: Arc<Session>,
    offset: u64,
    sequence: u64,
    pending: Vec<u8>,
    skipping: bool,
    modified: Option<SystemTime>,
    identity: u64,
}

fn identity(meta: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        0
    }
}

impl Tail {
    fn new(provider: Provider, path: PathBuf) -> Self {
        Self {
            session: Arc::new(Session::new(provider, path)),
            offset: 0,
            sequence: 0,
            pending: vec![],
            skipping: false,
            modified: None,
            identity: 0,
        }
    }
    fn read(&mut self, meta: &Metadata, max_events: usize) -> Result<()> {
        let modified = meta.modified().ok();
        if self.offset > meta.len()
            || (self.identity != 0 && self.identity != identity(meta))
            || (self.offset > 0 && self.offset == meta.len() && modified != self.modified)
        {
            *self = Self::new(self.session.provider, self.session.path.clone());
        }
        self.identity = identity(meta);
        self.modified = modified;
        if self.offset == meta.len() {
            return Ok(());
        }
        let mut file = File::open(&self.session.path)
            .with_context(|| format!("Cannot read {}", self.session.path.display()))?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::new();
        file.take(READ_BUDGET).read_to_end(&mut bytes)?;
        self.offset += bytes.len() as u64;
        let session = Arc::make_mut(&mut self.session);
        session.bytes_read = self.offset;
        session.file_size = meta.len();
        self.pending.extend_from_slice(&bytes);
        let mut start = 0;
        for (end, _) in self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
        {
            let line = &self.pending[start..end];
            if self.skipping || line.len() > MAX_LINE {
                if !self.skipping {
                    session.malformed += 1;
                }
                self.skipping = false;
            } else if !line.iter().all(u8::is_ascii_whitespace) {
                parse_line(session, line, self.sequence, max_events);
            }
            self.sequence += 1;
            start = end + 1;
        }
        self.pending.drain(..start);
        if self.pending.len() > MAX_LINE {
            self.pending.clear();
            if !self.skipping {
                session.malformed += 1;
            }
            self.skipping = true;
        }
        Ok(())
    }
}

pub struct Collector {
    config: SourceConfig,
    tails: HashMap<PathBuf, Tail>,
}

impl Collector {
    pub fn new(config: SourceConfig) -> Self {
        Self {
            config,
            tails: HashMap::new(),
        }
    }
    pub fn refresh(&mut self) -> Snapshot {
        let mut snapshot = Snapshot {
            scanned_at: Utc::now(),
            ..Snapshot::default()
        };
        let mut candidates = Vec::new();
        let mut paths = HashSet::new();
        for (provider, root) in &self.config.roots {
            if !root.exists() {
                snapshot.warnings.push(format!(
                    "{} logs not found: {}",
                    provider.label(),
                    root.display()
                ));
                continue;
            }
            for entry in WalkDir::new(root).follow_links(false) {
                match entry {
                    Ok(e)
                        if e.file_type().is_file()
                            && e.path().extension().is_some_and(|x| x == "jsonl") =>
                    {
                        if paths.insert(e.path().to_owned()) {
                            match e.metadata() {
                                Ok(m) => candidates.push((*provider, e.path().to_owned(), m)),
                                Err(e) => snapshot.warnings.push(e.to_string()),
                            }
                        }
                    }
                    Err(e) => snapshot.warnings.push(e.to_string()),
                    _ => {}
                }
            }
        }
        snapshot.discovered = candidates.len();
        candidates.sort_by(|a, b| {
            b.2.modified()
                .ok()
                .cmp(&a.2.modified().ok())
                .then(a.1.cmp(&b.1))
        });
        let mut selected: HashSet<PathBuf> = candidates
            .iter()
            .take(self.config.max_sessions)
            .map(|(_, p, _)| p.clone())
            .collect();
        // Include parents of retained agents even when those parents have older mtimes.
        let id_paths: HashMap<String, PathBuf> = candidates
            .iter()
            .filter_map(|(provider, path, _)| {
                let stem = path.file_stem()?.to_str()?;
                let id = if *provider == Provider::Codex {
                    stem.get(stem.len().checked_sub(36)?..)?
                } else {
                    stem
                };
                Some((provider.key(id), path.clone()))
            })
            .collect();
        for _ in 0..16 {
            let before = selected.len();
            for (provider, path, meta) in &candidates {
                if !selected.contains(path) {
                    continue;
                }
                let tail = self
                    .tails
                    .entry(path.clone())
                    .or_insert_with(|| Tail::new(*provider, path.clone()));
                if let Err(e) = tail.read(meta, self.config.max_events) {
                    snapshot.warnings.push(format!("{e:#}"));
                }
                if let Some(parent) = &tail.session.parent
                    && let Some(path) = id_paths.get(parent)
                {
                    selected.insert(path.clone());
                }
            }
            if selected.len() == before {
                break;
            }
        }
        self.tails.retain(|p, _| selected.contains(p));
        let mut keys = HashSet::new();
        for (_, path, _) in &candidates {
            if let Some(tail) = self.tails.get(path)
                && keys.insert(tail.session.key.clone())
            {
                snapshot.sessions.push(tail.session.clone());
            }
        }
        // Claude stores descendants in the root's subagents directory. A recorded
        // spawn result is stronger evidence of immediate parentage than that path.
        let mut parents = HashMap::new();
        for session in &snapshot.sessions {
            for e in &session.events {
                if e.kind == Kind::Spawn
                    && let Some(target) = &e.target
                    && let Some(child) = resolve_target(&snapshot, session, target)
                    && child != session.key
                {
                    parents.insert(child, session.key.clone());
                }
            }
        }
        for session in &mut snapshot.sessions {
            if session.provider == Provider::Claude
                && let Some(parent) = parents.get(&session.key)
                && session.parent.as_ref() != Some(parent)
            {
                Arc::make_mut(session).parent = Some(parent.clone());
            }
        }
        snapshot.sessions.sort_by(|a, b| {
            b.last_activity
                .cmp(&a.last_activity)
                .then(a.key.cmp(&b.key))
        });
        snapshot.warnings.truncate(20);
        snapshot
    }
}

pub fn resolve_target(snapshot: &Snapshot, source: &Session, target: &str) -> Option<String> {
    if target.is_empty() {
        return None;
    }
    let scoped_path = format!(
        "{}/{}",
        if source.agent_path.is_empty() {
            "/root"
        } else {
            &source.agent_path
        },
        target
    );
    snapshot
        .sessions
        .iter()
        .find(|candidate| {
            candidate.provider == source.provider
                && (candidate.id == target
                    || candidate.key == target
                    || (((!candidate.agent_path.is_empty()
                        && (candidate.agent_path == target
                            || candidate.agent_path == scoped_path))
                        || (target == "/root" && candidate.parent.is_none()))
                        && same_family(snapshot, source, candidate)))
        })
        .map(|s| s.key.clone())
}

fn root_key(snapshot: &Snapshot, session: &Session) -> String {
    let mut current = session;
    let mut seen = HashSet::new();
    while seen.insert(current.key.clone()) {
        let Some(parent) = &current.parent else {
            break;
        };
        let Some(next) = snapshot.sessions.iter().find(|s| &s.key == parent) else {
            return parent.clone();
        };
        current = next;
    }
    current.key.clone()
}
fn same_family(snapshot: &Snapshot, a: &Session, b: &Session) -> bool {
    root_key(snapshot, a) == root_key(snapshot, b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::tempdir;
    #[test]
    fn tails_partial_records_and_recovers_from_truncation_and_bad_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        fs::write(
            &path,
            b"{\"type\":\"user\",\"sessionId\":\"a\",\"message\":{\"content\":\"hello",
        )
        .unwrap();
        let mut c = Collector::new(SourceConfig {
            roots: vec![(Provider::Claude, dir.path().into())],
            max_sessions: 10,
            max_events: 10,
        });
        assert!(c.refresh().sessions[0].events.is_empty());
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "\"}}}}").unwrap();
        writeln!(file, "bad json").unwrap();
        let s = c.refresh();
        assert_eq!(s.sessions[0].events.len(), 1);
        assert_eq!(s.sessions[0].malformed, 1);
        assert_eq!(c.refresh().sessions[0].events.len(), 1);
        fs::write(
            &path,
            "{\"type\":\"user\",\"message\":{\"content\":\"new\"}}\n",
        )
        .unwrap();
        let s = c.refresh();
        assert_eq!(s.sessions[0].events.len(), 1);
        assert_eq!(s.sessions[0].events[0].input, "new");
    }
}
