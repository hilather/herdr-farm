# TEST-TICK-1 sandbox validation

Commands used `--locked --offline -j 3`. The complete state-store run used
`cargo test --features state-store --no-fail-fast`; clippy used
`cargo clippy --features state-store --all-targets`. The default-feature
all-targets check also passed. No new unit tests were added: timing acceptance
uses real CLI processes and public store APIs.

The complete run finished 72 groups: **1,292 passed, 316 failed**,
with **1,584.34 seconds** summed test-group time (build time excluded).
Five failures were subsequently resolved by fixes or focused reruns. The remaining
311 failures comprise 304 Unix socket restrictions (including fixture startup
consequences), four TCP socket restrictions, and three pre-existing copy fixture
failures. This is a sandbox result, not a successful outside-sandbox full run.

An untouched HEAD archive was tested with a separate target directory. Its eight
priority groups had exactly the same usual socket failures as the updated groups.
Its three copy fixture failures also reproduce: temporary projects inherit the
sandbox's read-only `/tmp/.git`, invalidating their assumed git resource footprint.
The full-run admission stress test hit a SQLite deadline under load and passed
an isolated rerun. A later canonical budget run hit transient SQLite disk I/O;
its isolated rerun passed. The CLI submission workflow now retries busy ingress
through the public CLI/store entry points, and its focused rerun passed.

## Before and after

Baseline means the untouched archive in this same sandbox. Socket-blocked groups
cannot establish the speed of successful workflows; the steward must run these
outside the sandbox with cargo-nextest. Times below exclude compilation.

| Group | Baseline seconds | Updated seconds | Updated passed / failed | Interpretation |
| --- | ---: | ---: | ---: | --- |
| artifacts | 0.32 | 0.07 | 1 / 5 | Socket restrictions; see list below |
| delivery | 0.0 | 0.0 | 0 / 1 | Socket restrictions; see list below |
| recovery | 0.04 | 0.02 | 0 / 5 | Socket restrictions; see list below |
| ticker_jobs | 0.07 | 0.04 | 0 / 8 | Socket restrictions; see list below |
| ticker | 2.51 | 0.56 | 1 / 4 | Socket restrictions; see list below |
| pull_requests | 0.0 | 0.0 | 0 / 1 | Socket restrictions; see list below |
| cli | 159.37 | 57.67 | 58 / 24 | Socket restrictions; see list below |
| canonical_worker | 182.15 | 140.09 | 5 / 31 | Socket restrictions; see list below |
| telemetry_query | 66.43 | 11.15 | 16 / 0 | All pass |
| ticker_timing | new | 16.1 | 4 / 0 | All pass |

CLI improved from 159.37 s to 57.67 s with the same 24 socket failures.
Telemetry query improved from 66.43 s to 11.15 s with all 16 tests passing.
The new timing suite includes an intentionally unscaled 15-second production
cadence check, so its elapsed time is not a speed benchmark. The owner's historical
roughly 50-minute run is an outside-sandbox measurement and is not directly
comparable to this restricted run.

## Complete-run group timings

These are the original full-run results before the focused corrections above.

| Group | Seconds | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: | ---: |
| lib | 194.62 | 527 | 85 | 13 |
| bin | 190.15 | 283 | 82 | 0 |
| actions | 0.13 | 1 | 0 | 0 |
| adopt | 0.02 | 0 | 6 | 0 |
| agents | 0.12 | 2 | 0 | 0 |
| artifacts | 0.14 | 1 | 5 | 0 |
| assignment_policies | 147.84 | 6 | 0 | 0 |
| barriers | 17.85 | 5 | 0 | 0 |
| canonical_worker | 198.83 | 5 | 31 | 0 |
| capabilities | 5.81 | 3 | 0 | 0 |
| cleanup | 0.0 | 0 | 3 | 0 |
| cli | 72.72 | 55 | 27 | 1 |
| controller | 0.03 | 0 | 12 | 0 |
| delegated_reservation | 6.94 | 4 | 0 | 0 |
| delivery | 0.0 | 0 | 1 | 0 |
| doctor | 0.55 | 2 | 0 | 0 |
| factory_harness | 102.34 | 19 | 0 | 0 |
| inbox | 0.0 | 0 | 2 | 0 |
| live_phase_a | 0.0 | 0 | 0 | 8 |
| memory_barriers | 16.21 | 6 | 0 | 0 |
| memory_control | 19.65 | 13 | 0 | 0 |
| memory_read_sets | 6.98 | 4 | 0 | 0 |
| memory_regressions | 13.69 | 14 | 0 | 0 |
| migration | 2.1 | 7 | 0 | 0 |
| operator_launch | 29.64 | 7 | 4 | 0 |
| overview | 0.12 | 2 | 3 | 0 |
| paths | 0.31 | 4 | 0 | 0 |
| plans | 11.21 | 9 | 0 | 0 |
| profiles | 14.93 | 6 | 0 | 0 |
| projects | 0.18 | 4 | 0 | 0 |
| pull_requests | 0.0 | 0 | 1 | 0 |
| quality_certification | 33.77 | 7 | 1 | 0 |
| reconcile | 0.0 | 0 | 3 | 0 |
| recovery | 0.02 | 0 | 5 | 0 |
| remote | 0.19 | 1 | 1 | 0 |
| rename | 0.08 | 2 | 0 | 0 |
| replay_suite | 43.5 | 6 | 2 | 0 |
| reservations | 0.01 | 0 | 5 | 0 |
| review_signer | 12.95 | 3 | 0 | 0 |
| routine_files | 0.05 | 2 | 1 | 0 |
| routines | 3.16 | 1 | 1 | 0 |
| satisfaction | 14.23 | 9 | 0 | 0 |
| scheduling | 0.01 | 0 | 7 | 0 |
| store_records | 1.86 | 3 | 0 | 0 |
| telemetry | 20.4 | 19 | 1 | 0 |
| telemetry_accounting | 36.46 | 32 | 1 | 0 |
| telemetry_certification | 21.56 | 13 | 0 | 0 |
| telemetry_claude | 22.73 | 13 | 0 | 0 |
| telemetry_collect | 21.07 | 6 | 0 | 0 |
| telemetry_compare | 5.83 | 6 | 0 | 0 |
| telemetry_conformance | 16.39 | 22 | 0 | 0 |
| telemetry_export | 3.02 | 7 | 0 | 0 |
| telemetry_gemini | 10.73 | 8 | 0 | 0 |
| telemetry_health | 23.74 | 14 | 1 | 0 |
| telemetry_live | 0.0 | 0 | 0 | 5 |
| telemetry_muse | 10.77 | 8 | 0 | 0 |
| telemetry_opencode | 20.78 | 5 | 0 | 0 |
| telemetry_operations | 16.31 | 14 | 0 | 0 |
| telemetry_otlp | 35.22 | 21 | 4 | 0 |
| telemetry_quality | 9.78 | 8 | 0 | 0 |
| telemetry_query | 25.21 | 15 | 1 | 0 |
| telemetry_review | 30.95 | 20 | 0 | 0 |
| telemetry_routines | 11.32 | 2 | 0 | 0 |
| telemetry_scale | 31.22 | 4 | 0 | 13 |
| telemetry_views | 4.7 | 7 | 0 | 0 |
| telemetry_workspace | 23.06 | 9 | 1 | 0 |
| threads | 0.0 | 0 | 7 | 0 |
| ticker | 0.47 | 1 | 4 | 0 |
| ticker_jobs | 0.03 | 0 | 8 | 0 |
| ticker_timing | 15.98 | 4 | 0 | 0 |
| update_packages | 3.53 | 2 | 0 | 0 |
| worker_login_share | 0.14 | 9 | 0 | 0 |

## Unix socket restriction (265)

- `adopt::a_busy_adopted_agent_is_prompted_by_the_ticker_once_it_is_done`
- `adopt::a_pane_is_the_coordinator_only_while_ids_directory_and_name_all_match`
- `adopt::adopt_refuses_a_pane_that_a_migrated_project_already_binds`
- `adopt::adopt_refuses_panes_without_an_agent_or_already_adopted_before_recording_anything`
- `adopt::adopt_workspace_without_an_agent_creates_no_project`
- `adopt::adopting_ready_agents_briefs_and_prompts_each_in_its_own_directory`
- `artifacts::a_brief_waits_for_a_pending_live_copy`
- `artifacts::live_copies_get_a_receipt_and_review_notice_each_and_a_new_execution_is_announced_again`
- `artifacts::merged_finalization_keeps_what_each_source_shape_proves`
- `artifacts::remote_resolve_needs_the_helper_and_streams_exact_bytes_once`
- `artifacts::resolve_preserves_each_version_once_and_a_lost_source_keeps_the_last_snapshot`
- `bin::brief_jobs::launch::tests::ambiguous_busy_missing_capability_and_missing_terminal_do_not_claim`
- `bin::brief_jobs::launch::tests::blocked_launch_retains_exclusion_and_cancels_its_subprocess`
- `bin::brief_jobs::launch::tests::changed_configuration_and_cancellation_after_claim_leave_uncertainty_without_start`
- `bin::brief_jobs::launch::tests::lost_foreign_and_malformed_acknowledgements_never_repeat_launch`
- `bin::brief_jobs::launch::tests::restored_config_path_cannot_authorize_arguments_parsed_from_other_bytes`
- `bin::brief_jobs::tests::blocked_sender_preserves_exclusion_while_neighbor_observation_progresses`
- `bin::brief_jobs::tests::busy_ambiguous_stale_and_replaced_socket_targets_do_not_claim`
- `bin::brief_jobs::tests::cancellation_after_claim_leaves_recovery_and_no_effect`
- `bin::brief_jobs::tests::concrete_sender_confirms_matching_agent_and_never_replays`
- `bin::brief_jobs::tests::conflicting_aliases_corrupt_neighbors_and_canonical_references_refuse`
- `bin::brief_jobs::tests::lost_malformed_and_foreign_acknowledgements_remain_uncertain`
- `bin::brief_jobs::tests::nonconflicting_canonical_neighbor_allows_supervised_brief`
- `bin::brief_jobs::tests::remote_sender_uses_frozen_bridge_and_recovers_without_replay`
- `bin::brief_jobs::tests::ticker_defers_send_and_restart_confirms_or_recovers_without_replay`
- `bin::canonical_controller::observations::tests::canonical_initial_schema_sql_obeys_original_job_deadline`
- `bin::canonical_controller::observations::tests::canonical_post_probe_sql_cannot_restart_its_budget`
- `bin::canonical_controller::observations::tests::canonical_rotation_registry_refuses_overflow_without_forgetting_present_projects`
- `bin::canonical_controller::observations::tests::canonical_worker_batches_leave_root_exclusive_notifications_a_turn`
- `bin::canonical_controller::observations::tests::canonical_worker_cancellation_excludes_mutations_and_preserves_snapshot`
- `bin::canonical_controller::observations::tests::canonical_worker_commits_fresh_evidence_without_trusting_queue_consumption`
- `bin::canonical_controller::observations::tests::canonical_worker_negative_liveness_is_invalidated_by_rebinding`
- `bin::canonical_controller::observations::tests::canonical_worker_refuses_expiry_and_changed_configuration_without_committing`
- `bin::canonical_controller::observations::tests::canonical_worker_repeated_negative_completions_clear_exit_veto`
- `bin::canonical_controller::observations::tests::canonical_worker_slow_probe_does_not_block_legacy_status`
- `bin::canonical_controller::tests::blocked_operation_preserves_live_reachability_and_capacity`
- `bin::canonical_notification_jobs::tests::canonical_notification_confirms_once_with_claim_and_inherited_ownership`
- `bin::canonical_notification_jobs::tests::canonical_notification_consumption_or_explicit_retirement_resolves_overlap`
- `bin::canonical_notification_jobs::tests::canonical_notification_frozen_selection_and_expired_work_never_send`
- `bin::canonical_notification_jobs::tests::canonical_notification_hint_cannot_authorize_a_paused_delivery`
- `bin::canonical_notification_jobs::tests::canonical_notification_only_verified_native_negatives_allow_retry`
- `bin::canonical_notification_jobs::tests::canonical_notification_overlapping_pending_workers_cannot_both_send`
- `bin::canonical_notification_jobs::tests::canonical_notification_queue_completion_cannot_certify_delivery`
- `bin::canonical_notification_jobs::tests::canonical_notification_ticker_defers_effect_without_blocking_legacy_status`
- `bin::canonical_notification_jobs::tests::canonical_notification_uncertainty_blocks_overlapping_batches_across_changed_authority`
- `bin::coordinator_jobs::notification::tests::blocked_notification_keeps_project_ownership_and_cancels`
- `bin::coordinator_jobs::notification::tests::explicit_notification_retry_does_not_authorize_a_replacement_socket`
- `bin::coordinator_jobs::notification::tests::legacy_reserved_notifications_become_uncertain_without_replaying`
- `bin::coordinator_jobs::notification::tests::notification_confirmed_delivery_is_not_replayed_and_preserves_unrelated_state`
- `bin::coordinator_jobs::notification::tests::notification_consumption_requires_seen_or_handled_evidence_not_disappearance`
- `bin::coordinator_jobs::notification::tests::notification_explicit_ack_suppresses_old_ids_but_new_items_can_progress`
- `bin::coordinator_jobs::notification::tests::notification_explicit_retry_is_linked_and_frozen`
- `bin::coordinator_jobs::notification::tests::notification_preflight_and_post_claim_changes_do_not_send`
- `bin::coordinator_jobs::notification::tests::notification_receipt_failure_retains_claim_for_reconciliation`
- `bin::coordinator_jobs::notification::tests::notification_uncertainty_blocks_new_items_and_mode_route_changes`
- `bin::coordinator_jobs::start::tests::blocked_coordinator_start_retains_ownership_and_cancels`
- `bin::coordinator_jobs::start::tests::coordinator_start_after_claim_cancellation_and_changes_preserve_uncertainty`
- `bin::coordinator_jobs::start::tests::coordinator_start_argument_bytes_cannot_be_swapped_and_errors_withhold_contents`
- `bin::coordinator_jobs::start::tests::coordinator_start_lost_or_mismatched_acknowledgements_block_automatic_start_and_prime`
- `bin::coordinator_jobs::start::tests::coordinator_start_refuses_a_thread_sharing_its_pane`
- `bin::coordinator_jobs::start::tests::coordinator_start_refuses_busy_foreign_ambiguous_and_unsupported_targets_before_claiming`
- `bin::coordinator_jobs::tests::blocked_coordinator_prime_allows_neighbor_progress_and_cancels`
- `bin::coordinator_jobs::tests::coordinator_open_reprime_queues_an_explicit_new_request_preserving_uncertainty`
- `bin::coordinator_jobs::tests::coordinator_prime_after_claim_changes_leave_recoverable_uncertainty`
- `bin::coordinator_jobs::tests::coordinator_prime_checks_same_project_threads_and_neighbor_aliases`
- `bin::coordinator_jobs::tests::coordinator_prime_preflight_refuses_busy_duplicate_unsupported_and_changed_authority`
- `bin::coordinator_jobs::tests::coordinator_update_preserves_previous_bytes_when_claim_exceeds_record_limit`
- `bin::coordinator_jobs::tests::lost_and_mismatched_prime_replies_recover_once_without_replay`
- `bin::coordinator_jobs::tests::malformed_coordinator_cannot_be_silently_replaced`
- `bin::coordinator_jobs::tests::ticker_queues_prime_without_a_synchronous_send_or_forged_receipt`
- `bin::local_observations::tests::blocked_observation_does_not_hold_effect_locks_and_stop_cancels_processes`
- `bin::local_observations::tests::collection_and_application_reject_changed_authority_and_expired_samples`
- `bin::local_observations::tests::collection_is_complete_bounded_and_distinguishes_negative_from_invalid`
- `bin::local_observations::tests::expiry_without_entering_runner_never_establishes_a_failed_session`
- `bin::local_observations::tests::failed_retries_keep_classification_but_route_changes_and_partial_samples_do_not`
- `bin::local_observations::tests::late_success_survives_the_actual_fifteen_second_ticker_cadence`
- `bin::local_observations::tests::queued_binding_changes_discard_completion_and_recollect_current_records`
- `bin::local_observations::tests::slow_session_cannot_block_healthy_status_or_fall_back_to_synchronous_observation`
- `bin::runtime_ownership::tests::adoption_blocks_canonical_legacy_corrupt_and_alias_conflicts`
- `bin::runtime_ownership::tests::adoption_counts_live_worker_and_resume_requires_fresh_post_adoption_evidence`
- `bin::runtime_ownership::tests::ownership_refuses_older_schema_neighbor_and_owned_coordinator_rebind`
- `bin::runtime_ownership::tests::recovery_plan_is_read_only_and_keeps_live_or_changed_workers_reserved`
- `bin::runtime_ownership::tests::relinquishment_is_atomic_fenced_and_never_reuses_claim_generation`
- `bin::runtime_ownership::tests::relinquishment_retains_worker_and_uncertain_attempt_capacity`
- `bin::runtime_ownership::tests::remote_outage_remains_unknown_and_retains_lost_capacity_across_repeated_polling`
- `bin::runtime_ownership::tests::replaced_session_and_changed_agent_pause_without_releasing_capacity`
- `bin::token_jobs::tests::blocked_refresh_retains_ownership_and_cancels_while_neighbor_can_observe`
- `bin::token_jobs::tests::lost_wrong_or_changed_receipts_never_become_execution_evidence`
- `bin::token_jobs::tests::remote_refresh_uses_frozen_route_and_refuses_changed_destination`
- `bin::token_jobs::tests::stale_ambiguous_and_foreign_targets_refuse_before_refresh`
- `cleanup::canonical_references_keep_the_worktree_but_an_unrelated_binding_does_not`
- `cleanup::canonical_references_made_after_removal_block_the_reopen`
- `cleanup::reopen_restores_the_removed_worktree_unless_its_branch_path_or_owner_changed`
- `cli::canonical_ownership_cli_adopts_recorded_coordinator_without_prompting`
- `cli::controller_captures_uncommitted_worker_edits_for_submission_and_verification`
- `cli::hot_paths_skip_the_whole_store_check_and_the_ticker_checks_off_its_pass_then_pauses_admission_and_effects_on_corruption`
- `cli::integration_releases_project_ownership_during_the_candidate_check`
- `cli::launch_reserve_records_operator_reason`
- `cli::native_ticker_claims_legacy_routine_and_restart_delivers_without_rerun`
- `cli::operator_verify_releases_project_ownership_during_the_check`
- `cli::rejected_reservation_writes_no_decision`
- `cli::ticker_auto_chain_releases_verified_integrated_and_fan_in_dependents`
- `cli::ticker_auto_integrates_two_results_serially_and_recovers_stale_and_crash`
- `cli::ticker_auto_verification_releases_project_ownership_during_the_check`
- `cli::ticker_auto_verifies_once_and_recovers_after_kill`
- `cli::ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `cli::ticker_canonical_observations_commit_cancel_and_restart_in_the_shared_pool`
- `cli::ticker_coordinator_prime_confirms_or_recovers_once_across_restart`
- `cli::ticker_coordinator_start_then_prime_recover_without_replaying_start`
- `cli::ticker_local_and_remote_launches_acknowledge_once_and_recover_lost_replies`
- `cli::ticker_native_briefs_confirm_or_recover_uncertainty_without_replay`
- `cli::ticker_native_copy_publishes_announces_and_does_not_recopy_after_restart`
- `cli::ticker_native_merged_finalization_resolves_and_replays_notice_after_restart`
- `cli::ticker_notifications_recover_across_restart_and_reconcile_through_cli`
- `cli::ticker_remote_briefs_confirm_or_recover_uncertainty_without_replay`
- `cli::ticker_tokens_use_supervised_local_remote_and_coordinator_refreshes_after_restart`
- `controller::a_malformed_ambiguous_notification_blocks_new_notifications`
- `controller::a_notification_is_claimed_before_it_is_shown_and_never_shown_twice`
- `controller::a_notification_is_refused_while_the_project_safety_settings_are_invalid`
- `controller::a_notification_retry_is_not_delivered_before_it_is_due`
- `controller::a_store_error_that_is_not_a_full_disk_or_busy_database_does_not_pause_admission`
- `controller::interval_slots_are_anchored_at_the_start_counted_in_bulk_and_never_rescheduled`
- `controller::missed_slots_are_skipped_or_coalesced_and_a_revision_never_reuses_an_occurrence`
- `controller::the_ticker_runs_routines_only_while_active_and_never_revives_a_disabled_revision`
- `controller::ticker_delivers_a_notification_once_only_while_active_and_unleased`
- `controller::ticker_notifies_an_expired_wait_once_and_reserves_nothing`
- `controller::ticker_reserves_a_ready_dependent_once_only_with_factory_admission_on`
- `controller::ticker_runs_an_approved_routine_once_beside_one_edited_after_approval`
- `delivery::a_lost_start_is_reported_once_and_only_a_restart_starts_the_agent_again`
- `inbox::ticker_items_are_listed_seen_once_and_moved_to_done`
- `inbox::unreadable_schedules_are_config_errors_and_healthy_routines_continue`
- `lib::canonical_worker::tests::a_starting_agent_whose_own_display_state_moves_is_named_once_but_identity_drift_is_refused`
- `lib::canonical_worker::tests::barrier_revocation_during_prompt_preserves_stale_delivery_and_stop_recovery`
- `lib::canonical_worker::tests::brief_preparation_and_delivery_do_not_decode_unrelated_approval_history`
- `lib::canonical_worker::tests::brief_preparation_expiry_and_failed_commit_leave_no_delivery_obligation`
- `lib::canonical_worker::tests::brief_rendering_accounts_selected_inputs_before_decode_or_claim`
- `lib::canonical_worker::tests::brief_rendering_shares_deadline_and_does_not_scan_retained_history`
- `lib::canonical_worker::tests::busy_foreign_and_changed_executable_workers_are_not_claimed`
- `lib::canonical_worker::tests::conflicting_legacy_socket_alias_prevents_brief_claim`
- `lib::canonical_worker::tests::controller_recovers_missing_initial_brief_without_sending_or_duplicating_it`
- `lib::canonical_worker::tests::corrupt_target_payload_cannot_hide_a_retained_resource_using_a_terminated_peer`
- `lib::canonical_worker::tests::creation_and_recovery_refuse_terminal_replacement_during_process_observation`
- `lib::canonical_worker::tests::creation_claim_and_recovery_intent_roll_back_together_before_native_effects`
- `lib::canonical_worker::tests::creation_retains_observed_supervisor_after_authority_changes`
- `lib::canonical_worker::tests::direct_workers_require_positive_visible_readiness_not_managed_launch_flag`
- `lib::canonical_worker::tests::exit_before_any_identity_observation_keeps_uncertain_creation_reserved`
- `lib::canonical_worker::tests::exited_worker_reconciles_after_handles_are_lost_without_claiming_task_success`
- `lib::canonical_worker::tests::gate_release_boundary_is_one_use_and_does_not_confirm_a_start`
- `lib::canonical_worker::tests::gate_release_refuses_changed_target_authority_and_failed_commit_is_atomic`
- `lib::canonical_worker::tests::gate_selection_claim_and_render_share_bounded_reads`
- `lib::canonical_worker::tests::last_moment_brief_preflight_blocks_changed_authority_without_submission`
- `lib::canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `lib::canonical_worker::tests::launch_advancement_refuses_unusable_environment_before_creation`
- `lib::canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped`
- `lib::canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate`
- `lib::canonical_worker::tests::live_gate_proof_refuses_the_same_child_after_exec`
- `lib::canonical_worker::tests::lost_creation_recovers_exact_resource_without_config_or_another_claim`
- `lib::canonical_worker::tests::lost_or_foreign_brief_acknowledgments_retain_claim_and_never_repeat`
- `lib::canonical_worker::tests::lost_workspace_reply_recovers_exact_live_marker_even_after_revocation_and_expiry`
- `lib::canonical_worker::tests::malformed_neighbor_identity_never_panics_claims_or_sends_a_brief`
- `lib::canonical_worker::tests::native_brief_original_deadline_bounds_a_stalled_submission`
- `lib::canonical_worker::tests::native_gate_refusal_does_not_consume_the_release_opportunity_or_send_input`
- `lib::canonical_worker::tests::native_gate_submission_is_once_even_when_the_reply_is_lost`
- `lib::canonical_worker::tests::native_start_confirmation_requires_real_exec_and_exact_agent_then_replays_read_only`
- `lib::canonical_worker::tests::new_workspace_creation_and_lost_layout_reply_are_one_use`
- `lib::canonical_worker::tests::observed_resource_exit_before_target_commit_remains_recoverable`
- `lib::canonical_worker::tests::pane_conflicts_remain_effect_fences_with_ten_thousand_retired_neighbors`
- `lib::canonical_worker::tests::prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects`
- `lib::canonical_worker::tests::readiness_lost_after_claim_prevents_prompt_and_retains_one_use_claim`
- `lib::canonical_worker::tests::recovery_target_commit_failure_retains_original_claim_and_can_be_reobserved`
- `lib::canonical_worker::tests::resource_creation_ignores_cold_approval_history_without_replaying`
- `lib::canonical_worker::tests::resource_creation_selection_and_render_are_history_bounded`
- `lib::canonical_worker::tests::resource_preparation_rejects_changed_config_before_consuming_approval_or_creating`
- `lib::canonical_worker::tests::resource_recovery_selects_its_evidence_without_scanning_history`
- `lib::canonical_worker::tests::retained_pane_projection_excludes_reused_history_and_tracks_recovery_boundaries`
- `lib::canonical_worker::tests::retained_pane_projection_tracks_source_mutations_and_preserves_workspaces`
- `lib::canonical_worker::tests::revoked_barrier_routes_a_real_supervised_stop_and_recovers_after_commit_failure`
- `lib::canonical_worker::tests::staged_pane_selection_ignores_other_panes_and_preserves_provenance_fences`
- `lib::canonical_worker::tests::staged_pane_selection_reads_the_real_schema42_without_new_indexes`
- `lib::canonical_worker::tests::staged_repository_stop_requires_and_records_preserved_partial_files`
- `lib::canonical_worker::tests::staged_stop_commit_failure_retains_capacity_and_recovers_without_creation`
- `lib::canonical_worker::tests::staged_stop_requires_output_evidence_and_records_an_empty_directory_as_absent`
- `lib::canonical_worker::tests::start_selection_is_bounded_and_cancellable_with_retained_history`
- `lib::canonical_worker::tests::stock_herdr_launch_execs_the_launcher_in_place_of_the_shell`
- `lib::canonical_worker::tests::stock_herdr_unconfirmed_launch_closes_its_workspace_and_counts_nothing`
- `lib::canonical_worker::tests::stop_before_brief_atomically_retires_send_and_commit_failure_keeps_capacity`
- `lib::canonical_worker::tests::supervised_root_creation_and_commit_losses_recover_without_bootstrap_or_replay`
- `lib::canonical_worker::tests::supervised_root_recovery_needs_exact_process_and_preserves_expired_authority`
- `lib::canonical_worker::tests::target_inventory_rejects_rehashed_inputs_and_broken_launch_operation_links`
- `lib::canonical_worker::tests::target_retention_uses_selected_history_and_original_control`
- `lib::canonical_worker::tests::termination_does_not_decode_unrelated_approval_history`
- `lib::canonical_worker::tests::termination_selection_is_bounded_and_cancellable_with_retained_history`
- `lib::canonical_worker::tests::uncertain_creation_refuses_foreign_socket_duplicate_panes_and_changed_command`
- `lib::canonical_worker::tests::uncertain_initial_brief_never_becomes_a_fresh_preparation_hint`
- `lib::canonical_worker::tests::workers_sharing_one_herdr_server_are_named_independently_and_recover_unapplied_names`
- `lib::canonical_worker::tests::workspace_acknowledgment_loss_retains_uncertainty_without_layout_or_recreation`
- `lib::canonical_worker::tests::workspace_layout_commit_failure_resumes_without_recreating_workspace`
- `lib::canonical_worker::tests::workspace_receipt_commit_failure_never_submits_layout_or_recreates`
- `lib::profile_preparation::tests::launch_ingress::canonical_worktree_paths_and_aliases_block_before_approval_consumption`
- `lib::profile_preparation::tests::launch_ingress::draft_signature_and_reservation_preserve_the_exact_brief_and_one_use_boundary`
- `lib::profile_preparation::tests::launch_ingress::lost_worktree_receipt_commit_recovers_after_approval_revocation_without_git_add`
- `lib::profile_preparation::tests::launch_ingress::preparation_capture_releases_sqlite_and_rechecks_selected_state`
- `lib::profile_preparation::tests::launch_ingress::preparation_stop_preserves_created_repositories_and_records_uncreated_plans_in_order`
- `lib::profile_preparation::tests::launch_ingress::repository_observation_ignores_replacement_refs_and_refuses_lazy_fetch`
- `lib::profile_preparation::tests::launch_ingress::signed_worktree_creation_retains_exact_checkout_and_recovers_without_replay`
- `lib::profile_preparation::tests::launch_ingress::started_worktree_proof_allows_output_but_rejects_reassociation`
- `lib::profile_preparation::tests::launch_ingress::unsigned_or_changed_launch_inputs_never_create_a_reservation`
- `lib::profile_preparation::tests::launch_ingress::worktree_creation_supports_multiple_repositories_binary_files_and_symlinks`
- `lib::profile_preparation::tests::launch_ingress::worktree_inventory_retains_uncertain_paths_and_refuses_corrupt_provenance`
- `lib::profile_preparation::tests::launch_ingress::worktree_only_stop_fences_late_launch_and_preserves_uncertain_resources`
- `lib::profile_preparation::tests::launch_ingress::worktree_preparation_uses_selected_history_through_receipt_commit`
- `lib::profile_preparation::tests::launch_ingress::worktree_refusals_precede_approval_consumption_and_effects`
- `lib::profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance`
- `lib::runner::socket::tests::reply_is_bounded_and_requires_a_complete_utf8_line`
- `lib::runner::socket::tests::trickle_reply_cannot_restart_deadline`
- `overview::overview_prints_groups_in_display_order_and_names_the_pane_that_needs_you`
- `overview::unfocus_reads_herdr_stdout_reports_its_stderr_on_failure_and_survives_a_missing_binary`
- `overview::unfocus_sends_one_line_over_one_connection_and_reads_one_reply_line`
- `pull_requests::pull_requests_match_the_threads_origin_and_branch_and_notices_name_only_new_commenters`
- `reconcile::reconcile_records_pane_evidence_from_what_each_session_lists`
- `reconcile::reconcile_records_worktree_evidence_only_from_complete_git_listings`
- `reconcile::recovery_plans_advise_without_retrying_ambiguous_effects_or_releasing_lost_capacity`
- `recovery::a_declined_toast_is_retried_after_restart_only_once_its_backoff_is_due`
- `recovery::a_gh_outage_outlasts_restarts_and_gives_one_item_each_way`
- `recovery::each_toast_claims_the_sorted_unseen_items_and_leaves_out_seen_and_handled_ones`
- `recovery::merged_finalization_retries_failed_copies_and_is_withdrawn_by_reopen_or_a_new_report`
- `recovery::remote_merged_finalization_waits_for_the_helper_and_an_explicit_resolve`
- `remote::remote_briefs_use_only_a_trustworthy_route_and_a_quoted_bridge`
- `reservations::an_exact_approval_reserves_its_launch_without_any_delegation_record`
- `reservations::budget_limits_gate_reservation_and_cancellation_never_refunds`
- `reservations::cancellation_releases_capacity_only_without_a_launch_claim`
- `reservations::dependent_reserves_once_after_verified_evidence_and_never_without_a_grant`
- `reservations::unproven_attempts_keep_their_slots_through_cancellation`
- `routine_files::ticker_writes_items_for_due_routines_and_unusable_files`
- `routines::a_changed_routine_script_is_not_run_but_the_binding_is_still_observed`
- `scheduling::a_succeeded_predecessor_without_a_verified_result_keeps_its_dependent_blocked`
- `scheduling::drafts_beside_a_reserved_holder_refuse_only_overlapping_claims`
- `scheduling::every_unterminated_attempt_holds_capacity_and_a_lower_cap_revokes_nothing`
- `scheduling::owner_draft_and_reserve_refuse_an_overlapping_write_scope_with_admission_off`
- `scheduling::queue_blockers_follow_reservations_unused_grants_and_live_attempts`
- `scheduling::queue_order_ages_and_a_requeue_keeps_the_original_age`
- `scheduling::reconcile_observes_exactly_the_bindings_with_unfinished_work`
- `telemetry::attempts_show_attention_summary`
- `telemetry_accounting::attention_intervals_union_and_censor`
- `telemetry_health::recommendations_and_notices_change_no_canonical_state_and_no_dispatch`
- `telemetry_workspace::thread_start_records_the_dispatch_reason_and_the_sidebar_suffix`
- `threads::prompt_refuses_a_bare_shell_a_blocked_or_unknown_agent_and_sends_otherwise`
- `threads::report_review_ack_and_resolve_copy_home`
- `threads::reprime_updates_only_the_priming_fields_of_the_coordinator_record`
- `threads::restart_follows_what_the_record_reached`
- `threads::start_restart_and_adopt_write_briefs_branches_and_launch_line`
- `threads::thread_list_groups_every_record_and_live_state`
- `threads::thread_start_uses_agent_arguments_only_for_the_kind_they_are_bound_to`
- `ticker::ticker_announces_review_only_for_a_fresh_copy_of_the_changed_report`
- `ticker::ticker_holds_remote_briefs_and_launches_without_a_saved_session_contract`
- `ticker::ticker_launch_cap_fails_exhausted_threads_but_not_an_acknowledged_last_start`
- `ticker::ticker_primes_ready_coordinators_and_leaves_other_sessions_alone`
- `ticker_jobs::a_due_legacy_routine_is_claimed_once_and_delivered_after_its_command_ends`
- `ticker_jobs::a_failed_coordinator_start_backs_off_while_another_project_works`
- `ticker_jobs::a_primed_coordinator_is_primed_again_only_after_an_explicit_reprime`
- `ticker_jobs::a_thread_start_is_confirmed_on_acknowledgement_without_waiting_for_the_agent`
- `ticker_jobs::open_after_a_coordinator_start_keeps_its_claim_and_never_starts_again`
- `ticker_jobs::remote_machines_poll_on_their_own_deadlines_and_only_long_outages_are_reported`
- `ticker_jobs::token_refreshes_cool_down_while_another_coordinator_starts_and_primes`
- `ticker_jobs::token_refreshes_follow_each_panes_current_group_and_write_no_execution_state`

## Resolved on rerun (5)

- `cli::ticker_canonical_routine_admits_from_hint_and_restart_keeps_one_execution`
- `cli::ticker_integrates_two_local_results_and_recovers_lost_reply_without_telemetry`
- `cli::ticker_run_outlasts_a_transient_lock_probe_and_names_a_real_holder`
- `lib::admission::tests::admission_resumes_after_a_full_page_of_unsigned_candidates`
- `telemetry_query::ticker_records_operating_passes_pause_resume_and_restart`

## Pre-existing copy fixture failures (3)

- `bin::copy_jobs::tests::artifact_transfer_scopes_nested_thread_dirs_to_one_common_git_dir`
- `bin::copy_jobs::tests::final_worker_recovers_without_sender_and_finishes_copy_after_eligibility_loss`
- `bin::copy_jobs::tests::retained_recovery_never_refetches_and_config_withdrawal_preserves_intent`

## Unix socket fixture startup restriction (39)

Child fixture servers could not create AF_UNIX listeners; their callers failed awaiting socket readiness or native probe startup.

- `canonical_worker::a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `canonical_worker::a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `canonical_worker::a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `canonical_worker::a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `canonical_worker::a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `canonical_worker::a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `canonical_worker::a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `canonical_worker::a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `canonical_worker::a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `canonical_worker::accepted_editing_worker_completes_automatically_after_integration`
- `canonical_worker::accepted_verify_only_editing_worker_completes_without_integration_automation`
- `canonical_worker::an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `canonical_worker::an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `canonical_worker::an_isolated_worker_submits_only_through_its_own_spool`
- `canonical_worker::an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `canonical_worker::an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `canonical_worker::canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_worker::canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`
- `canonical_worker::canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `canonical_worker::canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `canonical_worker::canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `canonical_worker::editing_worker_requires_operator_completion_when_automation_is_off`
- `canonical_worker::launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `canonical_worker::rejected_editing_worker_stays_running_and_can_resubmit`
- `canonical_worker::review_assignment_launches_with_blind_brief_and_records_session`
- `canonical_worker::ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `canonical_worker::ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `canonical_worker::ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `canonical_worker::ticker_recovers_a_lost_creation_reply_without_creating_again`
- `canonical_worker::ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `canonical_worker::ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `cli::outcome_success_path`
- `operator_launch::launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `operator_launch::launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `operator_launch::verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `operator_launch::verify_interaction_tolerates_agents_writing_into_their_execution_home`
- `quality_certification::a_sandboxed_worker_cannot_elevate_its_own_report`
- `replay_suite::launched_replay_candidate_cannot_read_hidden_checks_and_is_verified_by_them`
- `replay_suite::ordinary_task_on_the_source_repository_still_launches_without_replay_hides`

## TCP socket restriction (4)

- `telemetry_otlp::devin_two_processes_with_same_session_and_sequence_count_both_requests_over_http`
- `telemetry_otlp::http_auth_limits_malformed_and_replay`
- `telemetry_otlp::http_protobuf_attempt_token_binding_auth_and_project_token_unchanged`
- `telemetry_otlp::http_request_rate_is_bounded`

## Limits

Unix listeners and socket pairs return `Operation not permitted`; TCP fixture
listeners are also denied. No socket workaround was attempted. The steward must
run the listed workflows outside this sandbox before claiming the whole-suite
speedup. No owner agent data, owner project roots, running ticker/server, or real
agent CLI was used. No push was performed.

## TEST-TICK-1b regression follow-up

Read the complete steward `out/tick-alone-failures.log`, including every panic
and repeated cancellation/lock diagnostic. This follow-up preserves production
values and external execution budgets. No schema, retention classification, or
backup classification changed. Timer families now share factors/floors; see
[test timing policies](test-tick-timing.md). New guarantees extend E2E CLI/public
store workflows; no unit tests or source-text assertions were added.

Each reported regression is accounted for below. Pattern 1 is synthetic nominal
time; pattern 2 is relative deadline/cooldown ordering; pattern 3 is waiting for
observable state rather than a fixed number of passes.

| Suite / test | Pattern and correction |
| --- | --- |
| canonical_worker / canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination | 2: restore the unscaled native-call quiet window before observation admission can preempt the advisory batch; retain full wall-clock wait deadline |
| canonical_worker / canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal | 2: same advisory window; terminal identity assertions unchanged |
| canonical_worker / canonical_attempt_sidebar_clears_after_termination_in_an_active_project | 2: same advisory window; lifecycle cleanup assertions unchanged |
| canonical_worker / canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting | 2: same advisory window; real sampler scheduling and stored attention interval share its floor to avoid artificial gaps, with full wait deadline |
| canonical_worker / canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request | 2: same advisory window; restart still asserts no historical cleanup/native request |
| canonical_worker / editing_worker_requires_operator_completion_when_automation_is_off | 3: wait for Running before stopping the first ticker; retain the later quiet-pass assertions that automation did not complete it |
| telemetry / attempts_show_attention_summary | 1: omit scale from every CLI in the lab, including shared Fixture commands; retain exact nominal minute summaries |
| telemetry_accounting / attention_intervals_union_and_censor | 1: omit scale from the lab CLI; retain all exact intervals, censoring, gaps and unions |
| controller / a_notification_retry_is_not_delivered_before_it_is_due | 2: scale the retry spacing expectation; additionally enqueue a valid future notification through the public store API, observe two real passes while it is pending, and assert its eventual show timestamp is at/after its persisted due time |
| threads / thread_list_groups_every_record_and_live_state | 1: unscaled lab for nominal 10/120/500-second state fixtures |
| threads / restart_follows_what_the_record_reached | 1: unscaled CLI and ticker in the entire nominal startup-history lab |
| ticker_jobs / a_failed_coordinator_start_backs_off_while_another_project_works | 2: common pass/cooldown factor; scale the existing 40-second gap tolerance and retain other-project interleaving |
| ticker_jobs / token_refreshes_cool_down_while_another_coordinator_starts_and_primes | 2: same pass/cooldown family and scaled gap tolerance; retain exactly-once start/prime and interleaving assertions |
| ticker_jobs / remote_machines_poll_on_their_own_deadlines_and_only_long_outages_are_reported | 2: shared pass/poll/retry factor; assert every actual poll/retry gap against its deadline instead of assuming external probes finish before a fixed call count; outage/state assertions unchanged |

The existing socket-free `ticker_timing` workflow now checks the 500 ms main
cadence floor through metrics and bounded CLI passes. Its public-store restart
workflow checks 20 ms then 40 ms persisted retry deadlines even at a tiny factor,
and refuses claims before the deadline. This replaces the requested unit
ordering test in accordance with AGENTS.md and the task's explicit no-unit rule.

Validation commands all used `--locked --offline -j 3`:

- `cargo test --features state-store --test ticker_timing`: 4 passed (16.03 s).
- `cargo test --features state-store --test telemetry --test telemetry_accounting -- --skip attempts_show_attention_summary --skip attention_intervals_union_and_censor`: 19 + 32 passed (20.78 s + 39.04 s).
- `cargo test --features state-store --test canonical_worker --test controller --test telemetry --test telemetry_accounting --test threads --test ticker_jobs --test ticker_timing --no-run`: all seven suites compile.
- `cargo clippy --features state-store --all-targets`: completed; existing warnings, no diagnostics on changed lines.

Socket-only failures in this follow-up: **none executed**. All 14 reported
regressions above require Unix socket fixture binds and are compile-only here,
as requested. Their outside-sandbox execution remains with the steward. The
historical socket-only failure lists earlier in this document remain the prior
sandbox evidence; this follow-up does not claim those workflows passed.

## CI-PORTABILITY-1: sandbox suites in the CI merge gate

The `ci` nextest profile now runs the complete default test selection (four
partitions, four test threads per runner, one retry). There are no CI-specific
exclusions. The existing workflow already supplies `TMPDIR=${{ runner.temp }}`
and enables AppArmor unprivileged user namespaces; no workflow change is needed.
A retry pass is still reported as flaky rather than hiding the first failure.

The checkout on GitHub runners is under `/home/runner/work`. The worker sandbox
makes owner homes recursively read-only and hides owner secrets. Tests that
pin fixtures to `CARGO_TARGET_TMPDIR` put them under that protected checkout,
even when the runner supplies an external `TMPDIR`. Use the runtime temporary
directory for sandbox-writable fixtures. The sandbox already preserves needed
fixture subtrees when privatizing `/tmp`; neither read-only anchors nor secret
hiding need to change.

| Suite | Fixture-path finding and disposition |
| --- | --- |
| `operator_launch` | Changed its lab root from `CARGO_TARGET_TMPDIR` to `std::env::temp_dir()`; its runtime directory already honored `TMPDIR`. |
| `worker_login_share` | Changed the owner/project/execution-home fixture root to `std::env::temp_dir()`; credentials remain synthetic and the declared fixture owner home selects login lookup. |
| `thread_sandbox` | Changed the lab root to `std::env::temp_dir()`. This binary was already enabled in CI, despite the task's initial description. Extended its existing public isolation workflow with a fake owner checkout at `owner/work/herdr-farm/target/tmp`. |
| `canonical_worker` | Already uses `tempfile::tempdir()` and honors runner `TMPDIR`; the whole-binary exclusion was broader than the fixture-path problem. Socket-backed launch workflows require outside-sandbox validation. |
| `replay_suite` | Its shared `support::replay::Lab` already uses `tempfile::tempdir()`; hidden-check protections remain unchanged. Socket-backed launches require outside-sandbox validation. |

The fake-owner workflow sets `HERDR_PROJECTS_OWNER_HOME`, rejects writes and
execution-home creation under the fake checkout with `Read-only file system`,
then commits in the intended worktree and persists a report in the intended
execution home outside that declared owner. It also checks that neither the
forbidden directory nor the forbidden file was created. Existing secret-read,
Git isolation and restart assertions remain. This exercises public isolation
APIs and real gated processes, without source-text assertions or new unit tests.
Local cargo commands use `TMPDIR=$PWD/target/tmp`, not `/dev/shm`.

Precisely the newly enabled CI selection is every test in `canonical_worker`,
`operator_launch`, `worker_login_share`, and `replay_suite`, plus these eight
previously excluded named tests (not seven):

- `quality_certification::a_sandboxed_worker_cannot_elevate_its_own_report`
- `cli::outcome_success_path`
- `cli::ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `lib::admission::tests::admission_resumes_after_a_full_page_of_unsigned_candidates`
- `lib::canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `bin::finalization_delivery::tests::cancelled_finalization_and_receipt_io_preserve_unresolved_intent`
- `bin::canonical_finalization_jobs::tests::stop_snapshot_finalizes_without_source_in_foreground_and_queued_paths`
- `telemetry_health::ticker_health_requires_operator_opt_in_obeys_interval_and_never_notifies`

No product, schema, dependency, retention, or backup classification changes.
GitHub Actions cannot be run from this worker; the steward must push and inspect
all four partitions before treating this as confirmed runner validation.

The health exclusion had a separate reproducible cause: its pass helper stopped
when the accounting ledger changed, but health now runs later in deferred lanes.
With test-scaled shutdown draining, the process can exit before that lane runs.
The existing E2E helper now waits for both accounting and the expected persisted
tick evaluation count before requesting stop, with a generous 60-second wait
deadline. This changes no production interval or external execution budget.

The admission paging test initially exceeded the public API's 2-second read
budget while a broad library test run and builds competed with the focused
suite. Two attempts failed with `state store: Deadline`; after the broad run
finished, the focused test passed (11.48 seconds for the whole fixture/workflow).
This is not a socket-only failure and is recorded separately. It does not
justify a permanent runner exclusion; the existing CI retry policy remains.

### Local validation and sandbox limits

All cargo commands used `TMPDIR=$PWD/target/tmp` and
`--locked --offline -j 3`. The fixture scripts stand in for agents; no real
agent CLI, owner credentials, owner project root, or running owner ticker/server
was used. No `/dev/shm` scratch and no push.

| Check | Result |
| --- | --- |
| Five sandbox integration suites, `cargo test ... --no-run` | All compiled. |
| Final fake-owner `thread_sandbox` binary | 2 passed, including the EROFS reproduction and successful intended writes. |
| `worker_login_share` | 1 passed. |
| `operator_launch` | 12 passed, 5 socket-startup failures. |
| `canonical_worker` | 6 passed, 32 socket-startup failures. |
| `replay_suite` | 6 passed, 2 socket-startup failures. |
| Finalization cancellation/receipt I/O | Passed through cargo and a focused test-binary invocation. |
| Foreground/queued stop snapshots | Passed through cargo and a focused test-binary invocation. |
| Corrected health interval workflow | Passed through cargo (43.39 s under load) and focused execution (8.17 s). |
| Admission paging focused rerun after broad run | Passed (11.48 s); earlier deadline failures recorded above. |
| `cargo clippy ... --features state-store --all-targets` | Completed successfully; existing warnings, no diagnostics in changed test files. |

A broader existing library check also ran: **523 passed, 89 failed, 13 ignored**.
Of its failures, 84 were explicit Unix socket `Operation not permitted` errors.
The other five are not classified as socket-only and no unrelated fixes were
made: `admission::tests::integration_backlog_returns_capacity_full_and_terminal_rows_do_not`
(deadline), the admission paging test above (deadline, later passed),
`source_tree::tests::cleanup_budget_accepts_a_maximum_live_library_plus_stage_metadata`
(source-read deadline),
`store::barriers::tests::ancestor_memory_expiry_during_later_release_rolls_back_publication`
(memory completion blocked), and
`store::controlled::tests::core_snapshot_accounting_is_shared_across_tables_and_resets_per_snapshot`
(expected limit assertion). The broad cargo command stopped at the library
failure; its CLI and quality selections are instead exercised by focused nextest.

The consolidated focused nextest command selected the five sandbox binaries
and all eight formerly excluded named tests: **74 run, 30 passed, 44 failed**
(1604 unselected). Of the 44 failures, **43 were socket-only**, listed below;
the remaining admission deadline subsequently passed as recorded above.
The CI profile also passed a nextest compile-only run.

### Socket-only failures in the focused portability run (43)

- `canonical_worker::a_brief_swallowed_by_the_agent_is_redelivered_and_confirmed_only_once_accepted`
- `canonical_worker::a_brief_the_agent_never_accepts_is_left_ambiguous_not_confirmed`
- `canonical_worker::a_hidden_path_covering_the_execution_home_refuses_the_launch_before_creation`
- `canonical_worker::a_launch_reaches_running_while_another_holder_takes_the_shared_root_intermittently`
- `canonical_worker::a_legacy_thread_holding_the_planned_worktree_blocks_its_creation`
- `canonical_worker::a_proven_worker_end_keeps_the_project_admitted_but_an_unexplained_pane_loss_pauses_it`
- `canonical_worker::a_sandboxed_reviewer_uses_its_worker_channel_through_the_spool`
- `canonical_worker::a_subdirectory_binding_runs_in_the_same_subdirectory_of_the_new_worktree`
- `canonical_worker::a_worker_branch_reaching_a_corrupt_quarantined_object_is_refused`
- `canonical_worker::accepted_editing_worker_completes_automatically_after_integration`
- `canonical_worker::accepted_verify_only_editing_worker_completes_without_integration_automation`
- `canonical_worker::an_isolated_codex_worker_commits_through_codex_workspace_write_sandbox`
- `canonical_worker::an_isolated_worker_cannot_read_owner_secrets_or_lift_the_hiding_but_still_commits_and_submits`
- `canonical_worker::an_isolated_worker_submits_only_through_its_own_spool`
- `canonical_worker::an_operator_finishes_a_worker_that_never_submitted_and_the_result_lands_automatically`
- `canonical_worker::an_untracked_working_directory_is_refused_before_the_approval_is_used`
- `canonical_worker::canonical_attempt_sidebar_clears_after_termination_in_an_active_project`
- `canonical_worker::canonical_attempt_sidebar_does_not_publish_to_a_replaced_terminal`
- `canonical_worker::canonical_attempt_sidebar_refreshes_and_clears_on_pause_and_termination`
- `canonical_worker::canonical_attempt_sidebar_restart_offers_no_historical_cleanup_or_native_request`
- `canonical_worker::canonical_attempt_sidebar_uses_collected_usage_and_observed_waiting`
- `canonical_worker::dedicated_worker_refuses_remote_manifest_before_its_brief`
- `canonical_worker::editing_worker_requires_operator_completion_when_automation_is_off`
- `canonical_worker::launch_sets_the_intended_permission_mode_over_a_stale_one_in_the_home`
- `canonical_worker::rejected_editing_worker_stays_running_and_can_resubmit`
- `canonical_worker::review_assignment_launches_with_blind_brief_and_records_session`
- `canonical_worker::ticker_does_not_dispatch_a_launch_cancelled_before_creation`
- `canonical_worker::ticker_launches_and_briefs_once_then_stops_a_cancelled_worker_while_paused_and_revoked`
- `canonical_worker::ticker_launches_nothing_on_a_server_without_the_launch_contract_or_while_paused`
- `canonical_worker::ticker_recovers_a_lost_creation_reply_without_creating_again`
- `canonical_worker::ticker_retires_a_cancelled_gated_worker_without_starting_it`
- `canonical_worker::ticker_stops_the_dedicated_herdr_server_of_a_finished_task`
- `cli::outcome_success_path`
- `cli::ticker_canonical_notification_confirms_or_retains_ambiguity_after_owner_death`
- `lib::canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `operator_launch::launch_run_retries_after_termination_but_refuses_an_unobserved_live_worker`
- `operator_launch::launch_run_with_a_dedicated_server_after_verify_interaction_reserves_both_kinds`
- `operator_launch::stale_profile_evidence_is_refreshed_by_launch_run`
- `operator_launch::verify_interaction_produces_launchable_evidence_for_codex_and_claude_from_the_cli`
- `operator_launch::verify_interaction_tolerates_agents_writing_into_their_execution_home`
- `quality_certification::a_sandboxed_worker_cannot_elevate_its_own_report`
- `replay_suite::launched_replay_candidate_cannot_read_hidden_checks_and_is_verified_by_them`
- `replay_suite::ordinary_task_on_the_source_repository_still_launches_without_replay_hides`

### Socket-only failures in the additional library run (84)

- `lib::canonical_worker::tests::a_starting_agent_whose_own_display_state_moves_is_named_once_but_identity_drift_is_refused`
- `lib::canonical_worker::tests::barrier_revocation_during_prompt_preserves_stale_delivery_and_stop_recovery`
- `lib::canonical_worker::tests::brief_preparation_and_delivery_do_not_decode_unrelated_approval_history`
- `lib::canonical_worker::tests::brief_preparation_expiry_and_failed_commit_leave_no_delivery_obligation`
- `lib::canonical_worker::tests::brief_rendering_accounts_selected_inputs_before_decode_or_claim`
- `lib::canonical_worker::tests::brief_rendering_shares_deadline_and_does_not_scan_retained_history`
- `lib::canonical_worker::tests::busy_foreign_and_changed_executable_workers_are_not_claimed`
- `lib::canonical_worker::tests::conflicting_legacy_socket_alias_prevents_brief_claim`
- `lib::canonical_worker::tests::controller_recovers_missing_initial_brief_without_sending_or_duplicating_it`
- `lib::canonical_worker::tests::corrupt_target_payload_cannot_hide_a_retained_resource_using_a_terminated_peer`
- `lib::canonical_worker::tests::creation_and_recovery_refuse_terminal_replacement_during_process_observation`
- `lib::canonical_worker::tests::creation_claim_and_recovery_intent_roll_back_together_before_native_effects`
- `lib::canonical_worker::tests::creation_retains_observed_supervisor_after_authority_changes`
- `lib::canonical_worker::tests::direct_workers_require_positive_visible_readiness_not_managed_launch_flag`
- `lib::canonical_worker::tests::exit_before_any_identity_observation_keeps_uncertain_creation_reserved`
- `lib::canonical_worker::tests::exited_worker_reconciles_after_handles_are_lost_without_claiming_task_success`
- `lib::canonical_worker::tests::gate_release_boundary_is_one_use_and_does_not_confirm_a_start`
- `lib::canonical_worker::tests::gate_release_refuses_changed_target_authority_and_failed_commit_is_atomic`
- `lib::canonical_worker::tests::gate_selection_claim_and_render_share_bounded_reads`
- `lib::canonical_worker::tests::last_moment_brief_preflight_blocks_changed_authority_without_submission`
- `lib::canonical_worker::tests::launch_advancement_recovers_each_boundary_then_delivers_brief_and_stops`
- `lib::canonical_worker::tests::launch_advancement_refuses_unusable_environment_before_creation`
- `lib::canonical_worker::tests::launch_advancement_selection_is_bounded_and_operation_scoped`
- `lib::canonical_worker::tests::launch_reconciliation_ignores_unrelated_history_without_releasing_a_gate`
- `lib::canonical_worker::tests::live_gate_proof_refuses_the_same_child_after_exec`
- `lib::canonical_worker::tests::lost_creation_recovers_exact_resource_without_config_or_another_claim`
- `lib::canonical_worker::tests::lost_or_foreign_brief_acknowledgments_retain_claim_and_never_repeat`
- `lib::canonical_worker::tests::lost_workspace_reply_recovers_exact_live_marker_even_after_revocation_and_expiry`
- `lib::canonical_worker::tests::malformed_neighbor_identity_never_panics_claims_or_sends_a_brief`
- `lib::canonical_worker::tests::native_brief_original_deadline_bounds_a_stalled_submission`
- `lib::canonical_worker::tests::native_gate_refusal_does_not_consume_the_release_opportunity_or_send_input`
- `lib::canonical_worker::tests::native_gate_submission_is_once_even_when_the_reply_is_lost`
- `lib::canonical_worker::tests::native_start_confirmation_requires_real_exec_and_exact_agent_then_replays_read_only`
- `lib::canonical_worker::tests::new_workspace_creation_and_lost_layout_reply_are_one_use`
- `lib::canonical_worker::tests::observed_resource_exit_before_target_commit_remains_recoverable`
- `lib::canonical_worker::tests::pane_conflicts_remain_effect_fences_with_ten_thousand_retired_neighbors`
- `lib::canonical_worker::tests::prepared_launch_selection_rotates_stages_and_excludes_cancelled_or_expired_effects`
- `lib::canonical_worker::tests::readiness_lost_after_claim_prevents_prompt_and_retains_one_use_claim`
- `lib::canonical_worker::tests::recovery_target_commit_failure_retains_original_claim_and_can_be_reobserved`
- `lib::canonical_worker::tests::resource_creation_ignores_cold_approval_history_without_replaying`
- `lib::canonical_worker::tests::resource_creation_selection_and_render_are_history_bounded`
- `lib::canonical_worker::tests::resource_preparation_rejects_changed_config_before_consuming_approval_or_creating`
- `lib::canonical_worker::tests::resource_recovery_selects_its_evidence_without_scanning_history`
- `lib::canonical_worker::tests::retained_pane_projection_excludes_reused_history_and_tracks_recovery_boundaries`
- `lib::canonical_worker::tests::retained_pane_projection_tracks_source_mutations_and_preserves_workspaces`
- `lib::canonical_worker::tests::revoked_barrier_routes_a_real_supervised_stop_and_recovers_after_commit_failure`
- `lib::canonical_worker::tests::staged_pane_selection_ignores_other_panes_and_preserves_provenance_fences`
- `lib::canonical_worker::tests::staged_pane_selection_reads_the_real_schema42_without_new_indexes`
- `lib::canonical_worker::tests::staged_repository_stop_requires_and_records_preserved_partial_files`
- `lib::canonical_worker::tests::staged_stop_commit_failure_retains_capacity_and_recovers_without_creation`
- `lib::canonical_worker::tests::staged_stop_requires_output_evidence_and_records_an_empty_directory_as_absent`
- `lib::canonical_worker::tests::start_selection_is_bounded_and_cancellable_with_retained_history`
- `lib::canonical_worker::tests::stock_herdr_launch_execs_the_launcher_in_place_of_the_shell`
- `lib::canonical_worker::tests::stock_herdr_unconfirmed_launch_closes_its_workspace_and_counts_nothing`
- `lib::canonical_worker::tests::stop_before_brief_atomically_retires_send_and_commit_failure_keeps_capacity`
- `lib::canonical_worker::tests::supervised_root_creation_and_commit_losses_recover_without_bootstrap_or_replay`
- `lib::canonical_worker::tests::supervised_root_recovery_needs_exact_process_and_preserves_expired_authority`
- `lib::canonical_worker::tests::target_inventory_rejects_rehashed_inputs_and_broken_launch_operation_links`
- `lib::canonical_worker::tests::target_retention_uses_selected_history_and_original_control`
- `lib::canonical_worker::tests::termination_does_not_decode_unrelated_approval_history`
- `lib::canonical_worker::tests::termination_selection_is_bounded_and_cancellable_with_retained_history`
- `lib::canonical_worker::tests::uncertain_creation_refuses_foreign_socket_duplicate_panes_and_changed_command`
- `lib::canonical_worker::tests::uncertain_initial_brief_never_becomes_a_fresh_preparation_hint`
- `lib::canonical_worker::tests::workers_sharing_one_herdr_server_are_named_independently_and_recover_unapplied_names`
- `lib::canonical_worker::tests::workspace_acknowledgment_loss_retains_uncertainty_without_layout_or_recreation`
- `lib::canonical_worker::tests::workspace_layout_commit_failure_resumes_without_recreating_workspace`
- `lib::canonical_worker::tests::workspace_receipt_commit_failure_never_submits_layout_or_recreates`
- `lib::profile_preparation::tests::launch_ingress::canonical_worktree_paths_and_aliases_block_before_approval_consumption`
- `lib::profile_preparation::tests::launch_ingress::draft_signature_and_reservation_preserve_the_exact_brief_and_one_use_boundary`
- `lib::profile_preparation::tests::launch_ingress::lost_worktree_receipt_commit_recovers_after_approval_revocation_without_git_add`
- `lib::profile_preparation::tests::launch_ingress::preparation_capture_releases_sqlite_and_rechecks_selected_state`
- `lib::profile_preparation::tests::launch_ingress::preparation_stop_preserves_created_repositories_and_records_uncreated_plans_in_order`
- `lib::profile_preparation::tests::launch_ingress::repository_observation_ignores_replacement_refs_and_refuses_lazy_fetch`
- `lib::profile_preparation::tests::launch_ingress::signed_worktree_creation_retains_exact_checkout_and_recovers_without_replay`
- `lib::profile_preparation::tests::launch_ingress::started_worktree_proof_allows_output_but_rejects_reassociation`
- `lib::profile_preparation::tests::launch_ingress::unsigned_or_changed_launch_inputs_never_create_a_reservation`
- `lib::profile_preparation::tests::launch_ingress::worktree_creation_supports_multiple_repositories_binary_files_and_symlinks`
- `lib::profile_preparation::tests::launch_ingress::worktree_inventory_retains_uncertain_paths_and_refuses_corrupt_provenance`
- `lib::profile_preparation::tests::launch_ingress::worktree_only_stop_fences_late_launch_and_preserves_uncertain_resources`
- `lib::profile_preparation::tests::launch_ingress::worktree_preparation_uses_selected_history_through_receipt_commit`
- `lib::profile_preparation::tests::launch_ingress::worktree_refusals_precede_approval_consumption_and_effects`
- `lib::profile_preparation::tests::launch_ingress::worktree_verification_and_stop_select_only_their_launch_provenance`
- `lib::runner::socket::tests::reply_is_bounded_and_requires_a_complete_utf8_line`
- `lib::runner::socket::tests::trickle_reply_cannot_restart_deadline`
