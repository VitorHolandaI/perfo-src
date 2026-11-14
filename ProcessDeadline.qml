// Ends a helper that outlives its budget.
//
// The replay helpers run one at a time behind a guard: `HistoryPage` sets
// `isFetchingWindow` before starting one and clears it in `onExited`. A helper
// that never exits therefore holds that guard for the life of the process, and
// the page stops updating without an error anywhere. The same shape wedged the
// NVIDIA sampler (src/data/nvidia_smi/mod.rs).
//
// `running` follows the process rather than being started by hand, so a new
// call site cannot forget to arm it. Quickshell 0.3.2 exposes no kill method on
// Process, only the writable `running`, and clearing that is what the page
// already does to stop a helper; it raises `exited`, so the existing handler
// does the cleanup and this file holds no recovery logic of its own.

import QtQuick

Timer {
  /// The helper to watch.
  required property var process

  /// A name for the log line, so a timeout says which helper gave up.
  property string label: "helper"

  /// Budget in milliseconds. Generous on purpose: this is a last resort for a
  /// wedged helper, not a latency target. A first replay of a large recording
  /// builds both sidecars before answering, which is seconds of honest work.
  property int budgetMs: 30000

  interval: budgetMs
  repeat: false
  running: deadline.process !== null && deadline.process.running

  id: deadline

  onTriggered: {
    if (!deadline.process.running) {
      return
    }
    console.warn("perfo: " + deadline.label + " exceeded " + deadline.budgetMs
                 + " ms and was ended")
    // Raises `exited`, which releases the page's guard.
    deadline.process.running = false
  }
}
