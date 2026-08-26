//! Admission control between the two things that each hold multiple GB.
//!
//! FPV runs local narration (a chat model resident in Ollama) and local
//! image rendering (`sd-cli`, which maps a checkpoint plus encoders) as
//! independent subsystems. `LOCAL_RENDER_LOCK` in `image::sdcpp` keeps
//! renders from overlapping EACH OTHER, but nothing stopped a render from
//! starting while a narration turn was mid-generation — and on a 16 GB
//! machine those two peaks together are swap at best.
//!
//! Local Waifu has the same shape of problem and the same answer
//! (`resource_coordinator`: "admission control for work that competes with
//! a live call"). Its trigger is a voice call, which FPV does not have, so
//! only the principle ports, not the file.
//!
//! An `RwLock` rather than a `Mutex` because the two sides are not
//! symmetric:
//!
//! - narration takes a READ slot, so narration turns still run
//!   concurrently with each other exactly as before,
//! - a render takes the WRITE slot, so it waits for every in-flight
//!   narration and holds off any that would start during it.
//!
//! Tokio's `RwLock` is write-preferring, so a queued render is not starved
//! by a steady stream of narration turns.
//!
//! Cloud work never enters here: a BYOK narration or a cloud image call
//! competes for nobody's RAM, and making it queue would be a cost with no
//! benefit. The gate sits at the local entry points only.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

static LOCAL_COMPUTE: RwLock<()> = RwLock::const_new(());

/// Admission for a local inference call. Held for the duration of the
/// call; concurrent narration turns share it.
pub async fn narration_slot() -> RwLockReadGuard<'static, ()> {
    LOCAL_COMPUTE.read().await
}

/// Admission for a local render. Excludes every local inference call for
/// as long as it is held.
pub async fn render_slot() -> RwLockWriteGuard<'static, ()> {
    LOCAL_COMPUTE.write().await
}

#[cfg(test)]
mod tests {
    use super::{narration_slot, render_slot};
    use std::time::Duration;
    use tokio::time::timeout;

    /// One test, not two: the gate is a process-wide static, so separate
    /// `#[tokio::test]`s would run in parallel threads and contend with
    /// each other rather than with what they mean to assert.
    #[tokio::test]
    async fn narration_shares_the_slot_but_a_render_waits_for_it() {
        // Narration turns must still overlap each other — the gate is not
        // meant to serialize the app's normal path with itself.
        let first = narration_slot().await;
        let second = timeout(Duration::from_millis(50), narration_slot())
            .await
            .expect("a second narration turn must not wait behind the first");

        // A render, however, waits: this is the peak the gate exists to
        // keep from coinciding.
        assert!(
            timeout(Duration::from_millis(50), render_slot())
                .await
                .is_err(),
            "a render started while narration was generating"
        );

        drop(first);
        drop(second);
        let render = timeout(Duration::from_millis(50), render_slot())
            .await
            .expect("the render must proceed once narration is done");

        // And the reverse: nothing local starts under a live render.
        assert!(
            timeout(Duration::from_millis(50), narration_slot())
                .await
                .is_err(),
            "narration started while a render held the machine"
        );
        drop(render);
    }
}
