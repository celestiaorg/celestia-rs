//! Observable state of the sequential sampling worker.
use serde_json::{Value, json};

#[derive(Default, Clone)]
pub(super) struct Sampling {
    pub(super) sampled: u64,
    scheduled: u64,
    job: Option<Job>,
    pub(super) network: u64,
    pub(super) done: bool,
    pub(super) retries: u64,
}

#[derive(Clone)]
struct Job {
    id: u64,
    from: u64,
    to: u64,
    from_hash: String,
    to_hash: String,
    start: ctypes::Time,
    end: ctypes::Time,
}

impl Sampling {
    pub(super) fn begin_header(
        &mut self,
        previous: Option<&super::Verified>,
        next: &super::Verified,
    ) {
        self.start(next.height);
        if let Some(previous) = previous {
            self.job = Some(Job {
                id: self.job.as_ref().map_or(1, |job| job.id.saturating_add(1)),
                from: previous.height + 1,
                to: next.height,
                from_hash: hex::encode_upper(&previous.hash),
                to_hash: hex::encode_upper(&next.hash),
                start: light::verifier::now(),
                end: ctypes::Time::ZERO,
            });
        }
    }

    pub(super) fn sync_target(&self) -> u64 {
        self.job.as_ref().map_or(0, |job| job.to)
    }

    pub(super) fn sync_state(&self) -> Value {
        let job = self.job.as_ref();
        json!({
            "height": self.sampled, "id": job.map_or(0, |j| j.id),
            "from_height": job.map_or(0, |j| j.from), "to_height": self.sync_target(),
            "from_hash": job.map_or("", |j| j.from_hash.as_str()), "to_hash": job.map_or("", |j| j.to_hash.as_str()),
            "start": job.map_or(ctypes::Time::ZERO, |j| j.start).canonical_string(),
            "end": job.map_or(ctypes::Time::ZERO, |j| j.end).canonical_string()
        })
    }

    pub(super) fn start(&mut self, height: u64) {
        self.scheduled = height;
        self.network = self.network.max(height);
        self.done = false;
        self.retries = 0;
    }

    pub(super) fn complete(&mut self, height: u64) {
        self.start(height);
        self.sampled = height;
        if let Some(job) = &mut self.job {
            job.end = light::verifier::now();
        }
    }

    pub(super) fn json(&self) -> Value {
        let h = self.scheduled;
        let working = h > self.sampled;
        let mut value = json!({
            "head_of_sampled_chain":self.sampled, "head_of_catchup":h,
            "network_head_height":self.network, "catch_up_done":self.done,
            "concurrency":u8::from(working), "is_running":working || self.done
        });
        if working {
            value["workers"] = json!([{"job_type":"catchup","current":h,"from":h,"to":h}]);
            if self.retries != 0 {
                value["failed"] = json!({h.to_string():self.retries});
            }
        }
        value
    }
}
