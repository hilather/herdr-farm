# Coordinator wake on submit

Unfinished worker-result inbox items queue a canonical coordinator wake. With
`nudge = true`, only owned native coordinators reporting idle or done receive a
prompt. Busy coordinators persist a batch-specific probe backoff of 5, 10, then at
most 15 seconds (scaled in test labs), without adding a wake attempt. Passes
before that deadline do not probe or persist a wake. A changed batch or an
observed idle/done coordinator resets the backoff; idle is checked at least
every 15 seconds, preserving the wake-within-60-seconds target. Context reads
mark SEEN and do not consume this DONE-based wake policy.

Wake delivery holds the project effect guard and a shared root barrier, following
the supervised notification path. Foreground wakes use the same supervised calls.
Transferred locks remain held until descendants exit; the wake does not acquire
exclusive root ownership or suppress all controller services.

Detached ticker start waits for its child's published lock ownership, child exit,
or a five-second deadline. An immediate stop can therefore see the started ticker.
Bounded test runs refuse an existing ticker with a nonzero exit instead of
reporting success with zero passes. After the requested passes, they wait up to
60 seconds for pending copy/effect jobs, settling and logging completions before
executor shutdown. A stop file still cancels promptly. Continuous production
runs retain their existing shutdown behavior.

Replaying a worker submission means submitting the same retained submission file.
Recapturing creates a new result ID and legitimately permits another wake.
Confirmed batches are deduplicated across restart; ambiguous native replies fence
overlapping items until DONE. Claims persisted before a native call still become
uncertain on restart: the current retained claim does not distinguish a pre-call
stop from a lost reply. Removing that ambiguity needs a durable call-start marker
and end-to-end interruption coverage; this fix preserves the conservative fence.

No canonical schema or dependency changes are required. Wake claims and retry
state remain in `.state/canonical-wake.json`, retained with project backups.

Context marks only the inbox IDs it rendered SEEN. Head conflicts retry the
read-modify-write at most eight times under project ownership, within the
command’s 30-second SEEN deadline. Concurrent arrivals remain unseen and
already SEEN/DONE notices remain unchanged.

Wake offers live only in the copy/effect queue; busy deferrals write only the
retained wake JSON, never a canonical event. Identical retained payloads are
not rewritten. Controller observation freshness can refresh the observation
row without a `runtime.observed` event; only changed observation state advances
the canonical head.
