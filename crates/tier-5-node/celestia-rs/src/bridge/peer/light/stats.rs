//! Observable state of the sequential sampling worker.
use serde_json::{Value, json};

#[derive(Default, Clone)]
pub(super) struct Sampling {
    sampled: u64,
    scheduled: u64,
    pub(super) network: u64,
    pub(super) done: bool,
    pub(super) retries: u64,
}

impl Sampling {
    pub(super) fn start(&mut self, height: u64) {
        self.scheduled = height;
        self.network = self.network.max(height);
        self.done = false;
        self.retries = 0;
    }

    pub(super) fn complete(&mut self, height: u64) {
        self.start(height);
        self.sampled = height;
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
