#!/usr/bin/env python3
"""Run a green test, one mutant, and the restored test. No dependencies beyond Python/cargo."""
import difflib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent
API = "apps/api/src/"
CHECK_TEST = "crowdfunding_checks_reject_invalid_money_and_lifecycle_shapes"
QUANTITY_TEST = "quantities_cannot_be_negative_zero_or_oversold"
LEDGER_TEST = "wallet_and_ledger_enforce_nonnegative_balance_and_append_only"
CDN_TEST = "privacy_archive_and_moderation_send_cdn_purge"
# path, enclosing Rust function (None for SQL), exact old text, replacement, binary, test
CASES = {
    "cdn-private": (API + "wishlists.rs", "update", "crate::cdn::purge_wishlists(vec![w.slug.trim().to_string()]);", "", "cdn_invalidation", CDN_TEST),
    "cdn-archive": (API + "wishlists.rs", "archive", "crate::cdn::purge_wishlists(vec![w.slug.trim().to_string()]);", "", "cdn_invalidation", CDN_TEST),
    "cdn-hidden": (API + "reports.rs", "moderate", "crate::cdn::purge_wishlists(vec![slug.clone()]);", "", "cdn_invalidation", CDN_TEST),
    "cdn-account-delete": (API + "account.rs", "delete_me", "crate::cdn::purge_wishlists(slugs);", "", "account_deletion", "account_delete_purges_public_lists_after_commit"),
    "cdn-close-due": (API + "wishlists.rs", "close_due", "crate::cdn::purge_wishlists(rows.iter().map(|r| r.1.trim().to_string()).collect());", "", "closes_at", "close_due_purges_due_public_lists_and_keeps_future_lists_cached"),
    "ledger-update": ("db/migrations/0004_crowdfund.sql", None, "BEFORE UPDATE OR DELETE ON point_ledger", "BEFORE DELETE ON point_ledger", "migration_constraints", LEDGER_TEST),
    "ledger-delete": ("db/migrations/0004_crowdfund.sql", None, "BEFORE UPDATE OR DELETE ON point_ledger", "BEFORE UPDATE ON point_ledger", "migration_constraints", LEDGER_TEST),
    "wallet-for-update": (API + "contributions.rs", "create", "SELECT status::text FROM point_wallets WHERE id = $1 FOR UPDATE", "SELECT status::text FROM point_wallets WHERE id = $1", "concurrency", "wallet_freeze_vs_pledge_checks_status_under_lock"),
    "reallocate-wallet-for-update": (API + "contributions.rs", "reallocate", "SELECT status::text FROM point_wallets w WHERE w.id = $1 FOR UPDATE", "SELECT status::text FROM point_wallets w WHERE w.id = $1", "concurrency", "wallet_freeze_vs_reallocate_checks_status_under_lock"),
    "claim-wishlist-for-share": (API + "claims.rs", "create", "SELECT id FROM wishlists WHERE id = $1 FOR SHARE", "SELECT id FROM wishlists WHERE id = $1", "concurrency", "archive_vs_waiting_claim_cannot_accept_after_archive_commit"),
    "idempotency-claim-on-conflict": (API + "idempotency.rs", "begin", "ON CONFLICT (scope, key) DO NOTHING RETURNING key", "RETURNING key", "concurrency", "same_key_claim_requests_overlap_and_claim_once"),
    "idempotency-pledge-on-conflict": (API + "idempotency.rs", "begin", "ON CONFLICT (scope, key) DO NOTHING RETURNING key", "RETURNING key", "concurrency", "same_key_pledge_requests_overlap_and_debit_once"),
    "delete-item-for-update": (API + "wishlists.rs", "item_delete", "SELECT id FROM wishlist_items WHERE id=$1 FOR UPDATE", "SELECT id FROM wishlist_items WHERE id=$1", "concurrency", "force_delete_vs_claim_update_uses_item_then_claim_lock_order"),
    "reallocation-source-first": (API + "contributions.rs", "reallocate", "ORDER BY id FOR UPDATE", "ORDER BY CASE WHEN id = $1 THEN 0 ELSE 1 END FOR UPDATE", "concurrency", "opposite_reallocations_lock_lowest_item_first_without_deadlock"),
    "topup-for-update": (API + "topup.rs", "credit", "WHERE id = $1 FOR UPDATE", "WHERE id = $1", "topup", "webhook_blocked_on_row_lock_does_not_credit_twice"),
    "topup-query-skip": (API + "topup.rs", "confirm", "if !sig_ok || !paid {", "if false {", "topup", "query_confirm_mismatch_does_not_credit_and_is_retryable"),
    "topup-query-sig": (API + "topup.rs", "confirm", "let sig_ok = bool::from(given.as_bytes().ct_eq(check_mac(&q, cfg).as_bytes()));", "let sig_ok = given.len() > 0;", "topup", "query_confirm_mismatch_does_not_credit_and_is_retryable"),
    "lot-debit-order": (API + "points.rs", "debit", "WHEN 'adjustment' THEN 2 ELSE 3 END", "WHEN 'adjustment' THEN 3 ELSE 2 END", "point_lots", "pledge_deducts_lots_in_order_splits_ledger_and_withdraw_restores_original_lots"),
    "lot-debit-allocations": (API + "points.rs", "debit", 'if e.kind == "pledge" {', 'if e.kind == "never" {', "point_lots", "pledge_deducts_lots_in_order_splits_ledger_and_withdraw_restores_original_lots"),
    "lot-return-order": (API + "points.rs", "return_to_allocations", "WHEN 'topup' THEN 0", "WHEN 'topup' THEN 4", "point_lots", "partial_refund_and_failed_release_return_to_original_lots_topup_first"),
    "lot-return-allocation-update": (API + "points.rs", "return_to_allocations", "SET refunded_points = refunded_points + $3", "SET refunded_points = refunded_points + 0 * $3", "point_lots", "partial_refund_and_failed_release_return_to_original_lots_topup_first"),
    "lot-reallocate-allocations": (API + "contributions.rs", "reallocate", "UPDATE contribution_lot_allocations SET contribution_id = $2 WHERE contribution_id = $1", "UPDATE contribution_lot_allocations SET contribution_id = contribution_id WHERE contribution_id = $1 AND $2::uuid IS NOT NULL", "point_lots", "reallocation_moves_allocations_so_release_still_returns_original_lots"),
    "lot-source-uk": ("db/migrations/0022_point_lots.sql", None, "CREATE UNIQUE INDEX point_lots_source_uk", "CREATE INDEX point_lots_source_uk", "point_lots", "topup_creates_cash_refundable_lot_and_duplicate_credit_is_blocked"),
    "lot-ledger-ref-uk": ("db/migrations/0022_point_lots.sql", None, "(wallet_id, entry_type, ref_type, ref_id, lot_id)", "(wallet_id, entry_type, ref_type, ref_id)", "point_lots", "ledger_ref_unique_is_per_lot"),
    "lot-balance-reconciliation": ("apps/api/tests/common/mod.rs", "assert_points_reconciled", "lot_sum_off.is_empty()", "true", "point_lots", "lot_balance_reconciliation_detects_drift"),
    "lot-ledger-reconciliation": ("apps/api/tests/common/mod.rs", "assert_points_reconciled", "lot_ledger_off.is_empty()", "true", "point_lots", "lot_ledger_reconciliation_detects_drift"),
    "allocation-reconciliation": ("apps/api/tests/common/mod.rs", "assert_points_reconciled", "alloc_off.is_empty()", "true", "point_lots", "allocation_reconciliation_detects_drift"),
    "cash-refund-refundable-only": (API + "points.rs", "cash_refund_debit", "WHERE wallet_id = $1 AND cash_refundable AND remaining_points > 0", "WHERE wallet_id = $1 AND remaining_points > 0", "wallet_ops", "cash_refund_takes_only_refundable_lots_and_is_idempotent"),
    "cash-refund-restore-lots": (API + "points.rs", "cash_refund_restore", "SET remaining_points = remaining_points + $2 WHERE id = $1", "SET remaining_points = remaining_points + 0 * $2 WHERE id = $1", "wallet_ops", "staff_processes_cash_refunds_and_rejection_restores_original_lots"),
    "cash-refund-state-guard": (API + "wallet_ops.rs", "cash_refund_patch", "ELSE status IN ('requested', 'processing') END", "ELSE true END", "wallet_ops", "staff_processes_cash_refunds_and_rejection_restores_original_lots"),
    "cash-refund-reject-once": (API + "wallet_ops.rs", "cash_refund_patch", "ELSE status IN ('requested', 'processing') END", "ELSE true END", "wallet_ops", "concurrent_rejections_restore_points_once"),
    "wallet-debt-state-guard": (API + "wallet_ops.rs", "debt_patch", "WHERE id = $1 AND status = 'open'", "WHERE id = $1", "wallet_ops", "charge_back_recovers_remaining_lot_records_debt_and_is_idempotent"),
    "charge-back-idempotent": (API + "wallet_ops.rs", "charge_back", 'if status != "paid" && status != "disputed" {', "if false {", "wallet_ops", "charge_back_recovers_remaining_lot_records_debt_and_is_idempotent"),
    "topup-dispute-row-lock": (API + "wallet_ops.rs", "lock_topup", "FROM topups WHERE id = $1 FOR UPDATE", "FROM topups WHERE id = $1", "wallet_ops", "concurrent_lost_waits_on_topup_row_lock_and_charges_back_once"),
    "topup-dispute-state-guard": (API + "wallet_ops.rs", "topup_dispute", "if !applied && !already {", "if false {", "wallet_ops", "staff_dispute_rejects_bad_state_input_and_non_staff"),
    "claim-lock-timeout": (API + "claims.rs", "create", "crate::begin_contended(&st.pool).await?", "st.pool.begin().await?", "db_overload", "claim_waiting_on_hot_item_gives_up_with_503"),
    "claim-patch-lock-timeout": (API + "claims.rs", "apply", "crate::begin_contended(&st.pool).await?", "st.pool.begin().await?", "db_overload", "claim_status_change_waiting_on_hot_item_gives_up_with_503"),
    "pledge-lock-timeout": (API + "contributions.rs", "create", "crate::begin_contended(&st.pool).await?", "st.pool.begin().await?", "db_overload", "pledge_waiting_on_hot_item_gives_up_with_503"),
    "item-changed-on-conflict": (API + "notify.rs", "enqueue_item_changed", "ON CONFLICT DO NOTHING", "", "item_change_notice", "edits_coalesce_into_one_pending_mail_per_active_donor"),
    "item-changed-uk": ("db/migrations/0029_item_change_notice.sql", None, "CREATE UNIQUE INDEX notifications_item_changed_uk", "CREATE INDEX notifications_item_changed_uk", "item_change_notice", "edits_coalesce_into_one_pending_mail_per_active_donor"),
    "activity-on-conflict": (API + "session.rs", None, "ON CONFLICT DO NOTHING", "", "activity_days", "stale_session_with_existing_row_still_advances_last_seen"),
    "activity-not-exists": (API + "session.rs", None, " OR NOT EXISTS (SELECT 1 FROM user_activity_days a WHERE a.user_id = u.id AND a.day = (now() AT TIME ZONE 'Asia/Taipei')::date)", "", "activity_days", "fresh_session_but_missing_today_row_reinserts"),
}
# Disable exactly one named constraint while retaining valid migration SQL.
for name, migration, constraint, test in [
    ("check-qty-needed", "0001_mvp.sql", "items_qty_needed_pos", QUANTITY_TEST),
    ("check-qty-range", "0001_mvp.sql", "items_qty_claimed_range", QUANTITY_TEST),
    ("check-wallet-balance", "0004_crowdfund.sql", "point_wallets_balance_nonneg", LEDGER_TEST),
    ("check-pledged-nonnegative", "0004_crowdfund.sql", "items_points_nonneg", CHECK_TEST),
    ("check-pledged-cap", "0004_crowdfund.sql", "items_pledged_le_target", CHECK_TEST),
    ("check-crowdfund-shape", "0004_crowdfund.sql", "items_crowdfund_shape", CHECK_TEST),
    ("check-funded-full", "0004_crowdfund.sql", "items_funded_is_full", CHECK_TEST),
    ("check-expired-time", "0004_crowdfund.sql", "items_expired_has_time", CHECK_TEST),
    ("check-crowdfund-no-claims", "0004_crowdfund.sql", "items_crowdfund_no_claims", CHECK_TEST),
    ("check-concierge-only", "0004_crowdfund.sql", "items_concierge_only", CHECK_TEST),
]:
    CASES[name] = ("db/migrations/" + migration, None, constraint, None, "migration_constraints", test)
for name, function, condition in [
    ("claims-reconciliation", "assert_claims_reconciled", "drift.is_empty()"),
    ("wallet-reconciliation", "assert_points_reconciled", "bad.is_empty()"),
    ("contribution-reconciliation", "assert_points_reconciled", "drift.is_empty()"),
]:
    CASES[name] = ("apps/api/tests/common/mod.rs", function, condition, "true", "test_helpers", "fixtures_preserve_options_quantities_and_reconcile_domain_state")


FAILURE_MARKERS = {name: "CDN purge missing for wishlist" for name in CASES if name.startswith("cdn-")}
FAILURE_MARKERS.update({
    "ledger-update": "DB must reject SQLSTATE 23001 constraint unnamed",
    "ledger-delete": "DB must reject SQLSTATE 23001 constraint unnamed",
    "wallet-for-update": "frozen wallet was debited by the racing pledge",
    "reallocate-wallet-for-update": 'expected 1 blocked statements containing "FROM point_wallets w WHERE w.id"',
    "claim-wishlist-for-share": "claim accepted after archive committed",
    "idempotency-claim-on-conflict": "claim idempotency ON CONFLICT missing: SQLSTATE 23505 constraint idempotency_keys_pkey",
    "idempotency-pledge-on-conflict": "pledge idempotency ON CONFLICT missing: SQLSTATE 23505 constraint idempotency_keys_pkey",
    "delete-item-for-update": "item deletion without item lock caused a PostgreSQL deadlock: pg_stat_database.deadlocks",
    "reallocation-source-first": "reallocation source-first lock order caused a PostgreSQL deadlock: pg_stat_database.deadlocks",
    "topup-for-update": "webhook credited twice or failed after concurrent credit",
    "topup-query-skip": "二次確認被略過：未付款 TradeStatus=0",
    "topup-query-sig": "二次確認被略過：回應簽章錯誤",
    "topup-dispute-row-lock": "charge_back applied twice",
    "topup-dispute-state-guard": "pending 不可",
    "claim-lock-timeout": "請求一直卡在等鎖：交易沒有設 lock_timeout",
    "claim-patch-lock-timeout": "請求一直卡在等鎖：交易沒有設 lock_timeout",
    "pledge-lock-timeout": "請求一直卡在等鎖：交易沒有設 lock_timeout",
    "item-changed-on-conflict": "item PATCH failed: true",
    "item-changed-uk": "one pending item_changed per donor",
    "activity-on-conflict": "ON CONFLICT DO NOTHING missing: last_seen_at not advanced",
    "activity-not-exists": "NOT EXISTS clause missing: today's row not re-inserted",
    "lot-debit-order": "扣點順序必須是 bonus",
    "lot-debit-allocations": "pledge 必須寫入 allocations",
    "lot-return-order": "差額退回必須先回補可退現金的 topup 批次",
    "lot-return-allocation-update": "point_lots_remaining_range",
    "lot-reallocate-allocations": "分配必須跟著新認捐",
    "lot-source-uk": "point_lots_source_uk must reject a second topup lot",
    "lot-ledger-ref-uk": "point_ledger_ref_uk must allow one pledge split across lots",
    "lot-balance-reconciliation": "lot reconciliation accepted a drifted balance",
    "lot-ledger-reconciliation": "lot reconciliation accepted a drifted lot",
    "allocation-reconciliation": "allocation reconciliation accepted a drift",
    "cash-refund-refundable-only": "現金退款只能扣 cash_refundable 批次",
    "cash-refund-restore-lots": "rejected 必須依 lot_allocations 退回原批次",
    "cash-refund-state-guard": "cash refund state guard",
    "cash-refund-reject-once": "cash refund reject must restore exactly once",
    "wallet-debt-state-guard": "wallet debt state guard",
    "charge-back-idempotent": "charge_back must be idempotent",
    "claims-reconciliation": "claims reconciliation accepted a drifted cache",
    "wallet-reconciliation": "wallet reconciliation accepted a drifted balance",
    "contribution-reconciliation": "contribution reconciliation accepted a drifted cache",
})
for name, (_, _, constraint, replacement, _, _) in CASES.items():
    if replacement is None:
        FAILURE_MARKERS[name] = f"DB must reject SQLSTATE 23514 constraint {constraint}"
INFRA_FAILURES = (
    "INTERNAL_ERROR", "error: could not compile", "Connection refused", "connection refused",
    "pool timed out", "PoolTimedOut", "TooManyConnections", "remaining connection slots",
    "failed to initialize test database", "failed to acquire a connection",
)


def mutate(source, function, old, new):
    start, end = 0, len(source)
    if function:
        functions = list(re.finditer(r"^(?:pub )?(?:async )?fn (\w+)\b", source, re.M))
        matches = [i for i, match in enumerate(functions) if match[1] == function]
        if len(matches) != 1:
            raise ValueError(f"expected one function {function}, found {len(matches)}")
        index = matches[0]
        start = functions[index].start()
        end = functions[index + 1].start() if index + 1 < len(functions) else len(source)
    segment = source[start:end]
    if new is None:
        matches = list(re.finditer(r"\bCONSTRAINT\s+" + re.escape(old) + r"\s+CHECK\s*\(", segment))
        if len(matches) != 1:
            raise ValueError(f"expected one CHECK {old}, found {len(matches)}")
        opening = matches[0].end() - 1
        closing, depth = opening, 1
        while depth:
            closing += 1
            depth += (segment[closing] == "(") - (segment[closing] == ")")
        segment = segment[:opening] + "(true)" + segment[closing + 1:]
    else:
        if segment.count(old) != 1:
            raise ValueError(f"expected exactly one mutation anchor: {old}")
        segment = segment.replace(old, new, 1)
    return source[:start] + segment + source[end:]


def run(name, output):
    relative, function, old, new, binary, test = CASES[name]
    path = ROOT / relative
    directory = output / name
    directory.mkdir(parents=True, exist_ok=True)
    # A second interrupted run must never inherit a previous completed receipt.
    for artifact in ("baseline.log", "mutant.log", "restored.log", "mutant.patch", "result.json"):
        (directory / artifact).unlink(missing_ok=True)
    command = ["cargo", "test", "--manifest-path", "apps/api/Cargo.toml", "--locked", "--test", binary, test, "--", "--exact", "--nocapture"]
    result = {"mutation": name, "path": relative, "binary": binary, "test": test, "command": command,
              "result": "running", "phase": "preparing", "expected_failure_marker": FAILURE_MARKERS[name]}
    original, applied = None, False

    def save():
        (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")

    def check(phase):
        result["phase"] = phase
        save()
        with (directory / (phase + ".log")).open("w") as log:
            # Terminate cargo and its test/rustc children before restoring source.
            with subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True) as child:
                try:
                    exit_code = child.wait()
                except BaseException:
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    child.wait()
                    raise
        text = (directory / (phase + ".log")).read_text()
        result[phase + "_exit"] = exit_code
        save()
        return exit_code, text

    save()
    try:
        original = path.read_bytes()
        result["original_sha256"] = hashlib.sha256(original).hexdigest()
        changed = mutate(original.decode(), function, old, new).encode()
        result.update({
            "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            "rustc_version": subprocess.check_output(["rustc", "--version"], cwd=ROOT, text=True).strip(),
            "runner_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "original_sha256": hashlib.sha256(original).hexdigest(),
            "mutant_sha256": hashlib.sha256(changed).hexdigest(),
        })
        (directory / "mutant.patch").write_text("".join(difflib.unified_diff(
            original.decode().splitlines(True), changed.decode().splitlines(True),
            fromfile="a/" + relative, tofile="b/" + relative)))
        exit_code, text = check("baseline")
        if exit_code or "test result: ok. 1 passed;" not in text:
            result.update(result="invalid-baseline", classification_reason="baseline must run and pass exactly one test")
        else:
            result["phase"] = "applying"
            save()
            applied = True
            path.write_bytes(changed)
            exit_code, text = check("mutant")
            infra = next((marker for marker in INFRA_FAILURES if marker in text), None)
            expected = result["expected_failure_marker"] in text
            if exit_code == 0 and "test result: ok. 1 passed;" in text:
                result.update(mutant_result="survived", classification_reason="mutant test passed")
            elif infra:
                result.update(mutant_result="invalid-mutant", classification_reason=f"infrastructure/build failure: {infra}")
            elif exit_code and expected and "test result: FAILED. 0 passed; 1 failed;" in text:
                result.update(mutant_result="killed", classification_reason="the case-specific semantic assertion failed")
            else:
                result.update(mutant_result="invalid-mutant", classification_reason="missing expected semantic failure or exactly one failed test")
            result["phase"] = "restoring"
            save()
            path.write_bytes(original)
            applied = False
            result["restored_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
            if result["restored_sha256"] != result["original_sha256"]:
                raise ValueError("restored source hash differs from original")
            exit_code, text = check("restored")
            if exit_code or "test result: ok. 1 passed;" not in text:
                result.update(result="invalid-restored", classification_reason="restored test must pass exactly one test")
            else:
                result["result"] = result["mutant_result"]
        result["phase"] = "complete"
    except BaseException as error:
        result.update(result="interrupted" if isinstance(error, (KeyboardInterrupt, SystemExit)) else "error",
                      error=f"{type(error).__name__}: {error}")
        raise
    finally:
        try:
            if original is not None:
                if applied:
                    path.write_bytes(original)
                result["restored_sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
                result.setdefault("original_sha256", hashlib.sha256(original).hexdigest())
                result["source_restored"] = result["restored_sha256"] == result["original_sha256"]
                if not result["source_restored"]:
                    result.update(result="invalid-restored", classification_reason="source hash differs from original")
        except BaseException as error:
            result.update(result="restore-error", restore_error=f"{type(error).__name__}: {error}")
            raise
        finally:
            save()
    print(f"{name}: {result['result']} ({directory})", flush=True)
    return result["result"] == "killed"


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in {*CASES, "--list", "all"}:
        print("Usage: scripts/mutate.sh <name|all|--list>", file=sys.stderr)
        return 2
    if sys.argv[1] == "--list":
        print("\n".join(CASES))
        return 0
    output = Path(os.environ.get("MUTATION_OUTPUT_DIR", ROOT / "apps/api/target/mutations")).resolve()
    output.mkdir(parents=True, exist_ok=True)
    # Concurrent runs would mutate the same source and SQLx test databases.
    (ROOT / "apps/api/target").mkdir(parents=True, exist_ok=True)
    with (ROOT / "apps/api/target/mutation.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Another mutation run is active", file=sys.stderr)
            return 2
        names = list(CASES) if sys.argv[1] == "all" else [sys.argv[1]]
        return 0 if all([run(name, output) for name in names]) else 1


if __name__ == "__main__":
    # Ctrl-C/SIGTERM reaches finally and restores the original source.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    try:
        sys.exit(main())
    except (ValueError, OSError) as error:
        print(error, file=sys.stderr)
        sys.exit(2)
