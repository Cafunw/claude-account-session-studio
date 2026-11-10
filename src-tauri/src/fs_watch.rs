//! Signals when a new `local_*.json` session pointer appears, so the 4 s watcher tick can sync
//! within seconds. Modify events are ignored on purpose: Desktop rewrites pointers every turn.
use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Debounce {
    last_event: Option<Instant>,
    // Desktop saves every pointer as tmp -> delete -> rename, so only names never seen are new chats.
    known: HashSet<String>,
}

impl Debounce {
    fn observe(&mut self, event: &Event, now: Instant) {
        let relevant_kind = matches!(
            event.kind,
            EventKind::Create(_)
                | EventKind::Modify(ModifyKind::Name(RenameMode::To | RenameMode::Both | RenameMode::Any))
        );
        if !relevant_kind {
            return;
        }
        for name in event.paths.iter().filter_map(|p| p.file_name()?.to_str()) {
            if name.starts_with("local_") && name.ends_with(".json") && self.known.insert(name.to_string()) {
                self.last_event = Some(now);
            }
        }
    }

    /// True once, when the last relevant event is at least DEBOUNCE old.
    fn take_due(&mut self, now: Instant) -> bool {
        match self.last_event {
            Some(t) if now.duration_since(t) >= DEBOUNCE => {
                self.last_event = None;
                true
            }
            _ => false,
        }
    }
}

pub struct SessionWatch {
    _watcher: RecommendedWatcher,
    state: Arc<Mutex<Debounce>>,
}

impl SessionWatch {
    pub fn start(data_root: &Path) -> Result<Self, String> {
        let sessions = data_root.join("claude-code-sessions");
        let mut debounce = Debounce::default();
        for entry in walk_pointers(&sessions) {
            debounce.known.insert(entry);
        }
        let state = Arc::new(Mutex::new(debounce));
        let cb_state = state.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let (Ok(ev), Ok(mut d)) = (res, cb_state.lock()) {
                d.observe(&ev, Instant::now());
            }
        })
        .map_err(|e| e.to_string())?;
        watcher
            .watch(&sessions, RecursiveMode::Recursive)
            .map_err(|e| e.to_string())?;
        Ok(Self { _watcher: watcher, state })
    }

    /// True when a sync was requested and the debounce window has passed.
    pub fn take_due(&self) -> bool {
        self.state.lock().map(|mut d| d.take_due(Instant::now())).unwrap_or(false)
    }
}

/// File names of `<account>/<org>/local_*.json` under the sessions root.
fn walk_pointers(sessions: &Path) -> Vec<String> {
    let subdirs = |d: &Path| -> Vec<std::path::PathBuf> {
        std::fs::read_dir(d)
            .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for org in subdirs(sessions).iter().flat_map(|a| subdirs(a)) {
        for e in std::fs::read_dir(org).into_iter().flatten().flatten() {
            if let Some(n) = e.file_name().to_str().filter(|n| n.starts_with("local_") && n.ends_with(".json")) {
                out.push(n.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::CreateKind;

    fn ev(kind: EventKind, name: &str) -> Event {
        Event::new(kind).add_path(format!("x/acc/org/{name}").into())
    }

    #[test]
    fn filters_and_debounces() {
        let t0 = Instant::now();
        let mut d = Debounce::default();
        d.observe(&ev(EventKind::Modify(ModifyKind::Any), "local_a.json"), t0);
        d.observe(&ev(EventKind::Create(CreateKind::File), "other.json"), t0);
        assert!(!d.take_due(t0 + Duration::from_secs(10)));

        d.known.insert("local_known.json".into());
        d.observe(&ev(EventKind::Modify(ModifyKind::Name(RenameMode::To)), "local_known.json"), t0);
        assert!(!d.take_due(t0 + Duration::from_secs(10)));

        d.observe(&ev(EventKind::Create(CreateKind::File), "local_a.json"), t0);
        d.observe(&ev(EventKind::Create(CreateKind::File), "local_a.json"), t0 + Duration::from_secs(1)); // known now
        assert!(!d.take_due(t0 + Duration::from_secs(2)));
        // a rename-to event resets the window
        d.observe(&ev(EventKind::Modify(ModifyKind::Name(RenameMode::To)), "local_b.json"), t0 + Duration::from_secs(2));
        assert!(!d.take_due(t0 + Duration::from_secs(4)));
        assert!(d.take_due(t0 + Duration::from_secs(5)));
        assert!(!d.take_due(t0 + Duration::from_secs(20))); // fires once
    }
}
