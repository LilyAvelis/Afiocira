//! Ordered process-private requests. Publishing is deliberately distinct from acknowledgement.
use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

const CAPACITY: usize = 64;
const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
pub(super) struct Protocol {
    root: PathBuf,
    state: Mutex<State>,
}
#[derive(Default)]
struct State {
    serial: u64,
    acknowledged: u64,
    pending: VecDeque<Pending>,
    error: Option<String>,
}
struct Pending {
    id: u64,
    sent: Instant,
    cancelled: bool,
    response: Option<PathBuf>,
}
impl Protocol {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            state: Mutex::new(State::default()),
        }
    }
    fn path(&self, kind: &str, id: u64) -> PathBuf {
        self.root.join(format!("folder.txt.{kind}.{id}"))
    }
    pub fn submit(&self, operation: &str, mut request: serde_json::Value) -> Result<u64, String> {
        self.poll();
        let mut state = self.state.lock().map_err(|_| "Yazi protocol lock failed")?;
        if let Some(error) = &state.error {
            return Err(error.clone());
        }
        if state.pending.len() >= CAPACITY {
            return Err(
                "Yazi private request queue is full; retry after the navigator responds".into(),
            );
        }
        let id = state
            .serial
            .checked_add(1)
            .ok_or("Yazi request sequence exhausted")?;
        request["id"] = id.into();
        request["operation"] = operation.into();
        let bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err("Yazi request exceeds the private protocol size limit".into());
        }
        let temporary = self.path("request-tmp", id);
        fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
        if let Err(error) = fs::rename(&temporary, self.path("request", id)) {
            let _ = fs::remove_file(temporary);
            return Err(error.to_string());
        }
        state.serial = id;
        state.pending.push_back(Pending {
            id,
            sent: Instant::now(),
            cancelled: false,
            response: request["target"].as_str().map(PathBuf::from),
        });
        Ok(id)
    }
    pub fn cancel(&self, id: u64) {
        if let Ok(mut state) = self.state.lock()
            && let Some(pending) = state.pending.iter_mut().find(|p| p.id == id)
        {
            // Keep occupying a queue slot until the consumer retires the ID.
            pending.cancelled = true;
            if let Err(e) = fs::write(self.path("cancel", id), b"cancelled") {
                state.error = Some(format!("Yazi snapshot cancellation failed: {e}"));
            }
        }
    }
    pub fn acknowledged(&self, id: u64) -> bool {
        self.poll();
        self.state.lock().is_ok_and(|s| s.acknowledged >= id)
    }
    pub fn poll(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        while let Some(pending) = state.pending.front() {
            let id = pending.id;
            let path = self.path("ack", id);
            let Ok(bytes) = fs::read(&path) else {
                if !pending.cancelled
                    && pending.sent.elapsed() > Duration::from_secs(8)
                    && state.error.is_none()
                {
                    state.error = Some(format!(
                        "Yazi private request {id} timed out; restart the navigator before retrying mutations"
                    ));
                }
                break;
            };
            // The consumer must see its own acknowledgement before we collect it.
            if self.path("processing", id).exists() {
                break;
            }
            let Ok(ack) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                break;
            };
            if ack["id"].as_u64() != Some(id) {
                state.error = Some("Yazi acknowledgement has the wrong request ID".into());
                break;
            }
            if pending.cancelled
                && let Some(response) = &pending.response
            {
                let _ = fs::remove_file(response);
            }
            if !pending.cancelled && ack["ok"] != true {
                state.error = Some(format!(
                    "Yazi private request {id} failed: {}",
                    ack["error"].as_str().unwrap_or("unknown error")
                ));
            }
            state.acknowledged = id;
            state.pending.pop_front();
            for kind in ["ack", "cancel", "request", "processing"] {
                let _ = fs::remove_file(self.path(kind, id));
            }
        }
        // Retiring a cancelled read may itself wait for a paused consumer. It
        // must not hide an expired live mutation queued behind that read.
        if state.error.is_none()
            && let Some(expired) = state.pending.iter().find(|pending| {
                !pending.cancelled
                    && pending.sent.elapsed() > Duration::from_secs(8)
                    && !self.path("ack", pending.id).is_file()
            })
        {
            state.error = Some(format!(
                "Yazi private request {} timed out; restart the navigator before retrying mutations",
                expired.id
            ));
        }
    }
    pub fn take_error(&self) -> Option<String> {
        self.poll();
        // A delivery failure poisons the stream: never silently continue after an uncertain mutation.
        self.state.lock().ok().and_then(|state| state.error.clone())
    }
    pub fn pending_request_id(&self) -> Option<u64> {
        self.poll();
        self.state
            .lock()
            .ok()
            .and_then(|state| state.pending.front().map(|pending| pending.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str) -> (PathBuf, Protocol) {
        let root =
            std::env::temp_dir().join(format!("afio-protocol-{name}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        (root.clone(), Protocol::new(&root))
    }
    #[test]
    fn replace_and_cursor_requests_are_immutable_ordered_and_acknowledged() {
        let (root, p) = fixture("ordered");
        let a = p
            .submit(
                "apply",
                serde_json::json!({"replace":true,"marked":["a","b"]}),
            )
            .unwrap();
        let b = p
            .submit("apply", serde_json::json!({"replace":false,"cursor":"b"}))
            .unwrap();
        let c = p
            .submit("apply", serde_json::json!({"replace":true,"marked":["a"]}))
            .unwrap();
        assert!(a < b && b < c);
        assert!(
            fs::read_to_string(p.path("request", a))
                .unwrap()
                .contains("\"b\"")
        );
        assert!(!p.acknowledged(a));
        fs::write(p.path("ack", a), format!("{{\"id\":{a},\"ok\":true}}")).unwrap();
        assert!(p.acknowledged(a));
        assert!(!p.acknowledged(b));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancellation_keeps_identity_and_bounded_capacity() {
        let (root, p) = fixture("cancel");
        let old = p.submit("snapshot", serde_json::json!({})).unwrap();
        p.cancel(old);
        let new = p.submit("snapshot", serde_json::json!({})).unwrap();
        assert_ne!(old, new);
        for _ in 2..CAPACITY {
            p.submit("emit", serde_json::json!({})).unwrap();
        }
        assert!(p.submit("emit", serde_json::json!({})).is_err());
        assert!(p.path("cancel", old).is_file());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelled_front_does_not_hide_a_later_mutation_timeout() {
        let (root, p) = fixture("cancel-timeout");
        let read = p.submit("snapshot", serde_json::json!({})).unwrap();
        p.cancel(read);
        let mutation = p.submit("apply", serde_json::json!({})).unwrap();
        p.state.lock().unwrap().pending.back_mut().unwrap().sent =
            Instant::now() - Duration::from_secs(9);
        let error = p.take_error().unwrap();
        assert!(error.contains(&format!("request {mutation} timed out")));
        assert!(
            p.submit("emit", serde_json::json!({})).is_err(),
            "uncertain stream must fail closed"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
