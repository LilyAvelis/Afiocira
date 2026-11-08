//! API-only metadata bridge. Lazy worker, bounded work/results, no gallery hook.
use super::*;
use photo_core::metadata::{MetadataJob, MetadataRequest};

#[derive(Default)]
pub(super) struct State {
    worker: Option<Worker>,
    jobs: VecDeque<MetadataJob>,
    next_id: u64,
}
struct Worker {
    tasks: mpsc::SyncSender<(u64, MetadataRequest)>,
    results: Receiver<MetadataJob>,
}

impl State {
    pub(super) fn busy(&self) -> bool {
        self.jobs.iter().any(|j| j.pending)
    }
    pub(super) fn submit(
        &mut self,
        request: MetadataRequest,
        ctx: &egui::Context,
    ) -> Result<MetadataJob, String> {
        self.poll();
        // One outstanding request, at most 16 retained receipts. No implicit
        // retry after an uncertain save; inspect the receipt/revision instead.
        if self.jobs.iter().any(|j| j.pending) {
            return Err("Metadata worker is busy".into());
        }
        let worker = match &self.worker {
            Some(worker) => worker,
            None => {
                let home = photo_registry::default_data_home().map_err(|e| e.message)?;
                let (tasks, incoming) = mpsc::sync_channel::<(u64, MetadataRequest)>(1);
                let (outgoing, results) = mpsc::sync_channel(1);
                let ctx = ctx.clone();
                std::thread::Builder::new()
                    .name("afio-metadata".into())
                    .spawn(move || {
                        let registry = photo_registry::Registry::new(home);
                        while let Ok((request_id, request)) = incoming.recv() {
                            let path = request.path().to_owned();
                            let (snapshot, error) = match registry.execute(request) {
                                Ok(mut snapshot) => {
                                    snapshot.embedded =
                                        crate::embedded_metadata::inspect(&snapshot.path);
                                    (Some(snapshot), None)
                                }
                                Err(error) => (None, Some(error)),
                            };
                            if outgoing
                                .send(MetadataJob {
                                    request_id,
                                    path,
                                    pending: false,
                                    snapshot,
                                    error,
                                })
                                .is_err()
                            {
                                break;
                            }
                            ctx.request_repaint();
                        }
                    })
                    .map_err(|e| e.to_string())?;
                self.worker = Some(Worker { tasks, results });
                self.worker.as_ref().unwrap()
            }
        };
        let request_id = self
            .next_id
            .checked_add(1)
            .ok_or("Metadata request IDs exhausted")?;
        let job = MetadataJob {
            request_id,
            path: request.path().to_owned(),
            pending: true,
            snapshot: None,
            error: None,
        };
        worker
            .tasks
            .try_send((request_id, request))
            .map_err(|e| e.to_string())?;
        self.next_id = request_id;
        self.jobs.push_back(job.clone());
        while self.jobs.len() > 16 {
            self.jobs.pop_front();
        }
        Ok(job)
    }

    pub(super) fn poll(&mut self) {
        if let Some(worker) = &self.worker {
            while let Ok(result) = worker.results.try_recv() {
                if let Some(job) = self
                    .jobs
                    .iter_mut()
                    .find(|j| j.request_id == result.request_id)
                {
                    *job = result;
                }
            }
        }
    }

    pub(super) fn status(&mut self, request_id: u64) -> Result<MetadataJob, String> {
        self.poll();
        self.jobs
            .iter()
            .find(|j| j.request_id == request_id)
            .cloned()
            .ok_or_else(|| {
                "Unknown or expired metadata receipt (last 16 requests in this session only)".into()
            })
    }
}
