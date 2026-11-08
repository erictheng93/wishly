# Security Review: Wishlist

## Scope

Repository-wide target with focused static coverage of identity, wallet, public sharing and client credentials.

- Scan mode: repository
- Target kind: git_revision
- Target ID: target_sha256_5375d5a314f98844adebec22a0e7a333d3902ec6371b97e5e86220e134cd3e34
- Revision: b929c4fa2fe4ca6cffafc79e47865bfb19c80d84
- Inventory strategy: repository
- Included paths: .
- Excluded paths: none

Limitations and exclusions:
- No runtime testing, provider verification or exhaustive per-file review.

### Scan Summary

| Field | Value |
| --- | --- |
| Scan outcome | completed |
| Reportable findings | 6 |
| Severity mix | medium: 6 |
| Confidence mix | high: 6 |
| Coverage | partial |
| Validation mode | not recorded |

Canonical artifacts: `scan-manifest.json`, `findings.json`, and `coverage.json`. This report is a deterministic projection of those files.

## Threat Model

Wishly accepts browser traffic through Nuxt and axum. Public wishlist readers, guests, authenticated members, organization operators and staff have different access to identity, fulfillment records and wallet balances. PostgreSQL persists authorization and ledger state. Media storage and outbound notifications cross service boundaries. Staff session MFA, object-level authorization, visibility filters, and transaction integrity protect these assets. Production configuration and external service permissions are unverified.

Repository: Wishlist
Version: b929c4fa2fe4ca6cffafc79e47865bfb19c80d84

## Findings

| Finding | Severity | Confidence | Detailed write-up |
| --- | --- | --- | --- |
| [Legacy guest recovery cookie lacks Secure](#finding-1) | medium | high | inline below |
| [Sharing a wishlist reveals restricted donors’ names and points](#finding-2) | medium | high | inline below |
| [Late top-up settlement can exceed spending controls](#finding-3) | medium | high | inline below |
| [Browser reports logout success after revocation fails](#finding-4) | medium | high | inline below |
| [A stolen session can persist account access through identity linking](#finding-5) | medium | high | inline below |
| [Staff recovery hashes provide limited offline guessing resistance](#finding-6) | medium | high | inline below |

### Confidence Scale

| Label | Meaning |
| --- | --- |
| high | Direct evidence supports the finding with no material unresolved blocker. |
| medium | Evidence supports a plausible issue, but material runtime or reachability proof remains. |
| low | Evidence is incomplete and the item is retained only for explicit follow-up. |

<a id="finding-1"></a>

### [1] Legacy guest recovery cookie lacks Secure

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Verified against current source. Deployment and runtime behavior were not tested. |
| Category | guest-cookie |
| CWE | CWE-614 |
| Affected lines | apps/web/app/composables/useGuest.ts:30 |

#### Summary

Recovery copies a management bearer into a year-long cookie. An HTTP request that the browser does not upgrade can disclose it. Production HSTS was not checked.

#### Root Cause

Recovery copies a management bearer into a year-long cookie. An HTTP request that the browser does not upgrade can disclose it. Production HSTS was not checked.

**Legacy guest recovery cookie lacks Secure** — `apps/web/app/composables/useGuest.ts:30`

Recovery copies a management bearer into a year-long cookie. An HTTP request that the browser does not upgrade can disclose it. Production HSTS was not checked.

```
safe(() => { document.cookie = `ws_guest_js=${t}; Path=/; Max-Age=31536000; SameSite=Lax`; persisted ||= readCookie() === t })
```

#### Validation

Offline static review. Recovery copies a management bearer into a year-long cookie. An HTTP request that the browser does not upgrade can disclose it. Production HSTS was not checked.

#### Dataflow

The canonical finding records the affected path at apps/web/app/composables/useGuest.ts:30, but no expanded source-to-sink narrative was recorded.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

#### Severity

**Medium** — Recovery copies a management bearer into a year-long cookie. An HTTP request that the browser does not upgrade can disclose it. Production HSTS was not checked.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Use the server-issued Secure, HttpOnly cookie and expire the duplicate cookie.

<a id="finding-2"></a>

### [2] Sharing a wishlist reveals restricted donors’ names and points

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Verified against current source. Deployment and runtime behavior were not tested. |
| Category | contribution-privacy |
| CWE | CWE-200 |
| Affected lines | apps/api/src/public.rs:369 |

#### Summary

The shared wishlist groups pledges by donor without checking each pledge’s visibility. With contributor names enabled and surprise mode unlocked, private and friends-only pledges can appear by name, including in mixed-visibility totals.

#### Root Cause

The shared wishlist groups pledges by donor without checking each pledge’s visibility. With contributor names enabled and surprise mode unlocked, private and friends-only pledges can appear by name, including in mixed-visibility totals.

**Sharing a wishlist reveals restricted donors’ names and points** — `apps/api/src/public.rs:369`

The shared wishlist groups pledges by donor without checking each pledge’s visibility. With contributor names enabled and surprise mode unlocked, private and friends-only pledges can appear by name, including in mixed-visibility totals.

```
"SELECT c.item_id, c.user_id, c.is_anonymous, c.donor_name, c.points FROM contributions c JOIN wishlist_items i ON i.id = c.item_id
```

#### Validation

Offline static review. The shared wishlist groups pledges by donor without checking each pledge’s visibility. With contributor names enabled and surprise mode unlocked, private and friends-only pledges can appear by name, including in mixed-visibility totals.

#### Dataflow

The canonical finding records the affected path at apps/api/src/public.rs:369, but no expanded source-to-sink narrative was recorded.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

#### Severity

**Medium** — The shared wishlist groups pledges by donor without checking each pledge’s visibility. With contributor names enabled and surprise mode unlocked, private and friends-only pledges can appear by name, including in mixed-visibility totals.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Apply per-pledge visibility before grouping. Test a donor with both public and private pledges.

<a id="finding-3"></a>

### [3] Late top-up settlement can exceed spending controls

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Verified against current source. Deployment and runtime behavior were not tested. |
| Category | topup-reservation |
| CWE | CWE-841 |
| Affected lines | apps/api/src/topup.rs:641 |

#### Summary

Creation reserves capacity for pending orders. Local cancellation releases it while a subsequent genuine payment can still credit the wallet. The wallet balance ceiling and new-account daily limit can both be exceeded.

#### Root Cause

Creation reserves capacity for pending orders. Local cancellation releases it while a subsequent genuine payment can still credit the wallet. The wallet balance ceiling and new-account daily limit can both be exceeded.

**Late top-up settlement can exceed spending controls** — `apps/api/src/topup.rs:641`

Creation reserves capacity for pending orders. Local cancellation releases it while a subsequent genuine payment can still credit the wallet. The wallet balance ceiling and new-account daily limit can both be exceeded.

```
if !matches!(status.as_str(), "pending" | "expired" | "failed") {
```

#### Validation

Offline static review. Creation reserves capacity for pending orders. Local cancellation releases it while a subsequent genuine payment can still credit the wallet. The wallet balance ceiling and new-account daily limit can both be exceeded.

#### Dataflow

The canonical finding records the affected path at apps/api/src/topup.rs:641, but no expanded source-to-sink narrative was recorded.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

#### Severity

**Medium** — Creation reserves capacity for pending orders. Local cancellation releases it while a subsequent genuine payment can still credit the wallet. The wallet balance ceiling and new-account daily limit can both be exceeded.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Keep reservations until provider closure or hold excess funds for reconciliation under the wallet lock.

<a id="finding-4"></a>

### [4] Browser reports logout success after revocation fails

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Verified against current source. Deployment and runtime behavior were not tested. |
| Category | logout-failure |
| CWE | CWE-613 |
| Affected lines | apps/web/app/composables/useAuth.ts:29 |

#### Summary

If the logout request never reaches the API, the HttpOnly cookie and server session stay valid. The browser clears its user state and opens the login screen, putting a shared-device user’s account at risk.

#### Root Cause

If the logout request never reaches the API, the HttpOnly cookie and server session stay valid. The browser clears its user state and opens the login screen, putting a shared-device user’s account at risk.

**Browser reports logout success after revocation fails** — `apps/web/app/composables/useAuth.ts:29`

If the logout request never reaches the API, the HttpOnly cookie and server session stay valid. The browser clears its user state and opens the login screen, putting a shared-device user’s account at risk.

```
try { await api('/auth/logout', { method: 'POST', noRedirect: true }) } catch {}
```

#### Validation

Offline static review. If the logout request never reaches the API, the HttpOnly cookie and server session stay valid. The browser clears its user state and opens the login screen, putting a shared-device user’s account at risk.

#### Dataflow

The canonical finding records the affected path at apps/web/app/composables/useAuth.ts:29, but no expanded source-to-sink narrative was recorded.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

#### Severity

**Medium** — If the logout request never reaches the API, the HttpOnly cookie and server session stay valid. The browser clears its user state and opens the login screen, putting a shared-device user’s account at risk.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Handle transport and server failures explicitly and offer retry. Clear the UI after acknowledged logout or confirmed session expiry.

<a id="finding-5"></a>

### [5] A stolen session can persist account access through identity linking

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Verified against current source. Deployment and runtime behavior were not tested. |
| Category | identity-reauthentication |
| CWE | CWE-304 |
| Affected lines | apps/api/src/identities.rs:77 |

#### Summary

Linking proves control of the new provider and checks that the current session is live. It does not require fresh proof from an existing factor. The added login identity can outlast revocation of that session.

#### Root Cause

Linking proves control of the new provider and checks that the current session is live. It does not require fresh proof from an existing factor. The added login identity can outlast revocation of that session.

**A stolen session can persist account access through identity linking** — `apps/api/src/identities.rs:77`

Linking proves control of the new provider and checks that the current session is live. It does not require fresh proof from an existing factor. The added login identity can outlast revocation of that session.

```
let live: Option<Uuid> = sqlx::query_scalar("SELECT id FROM sessions WHERE user_id=$1 AND token_hash=$2 AND revoked_at IS NULL AND expires_at > now() FOR SHARE")
```

#### Validation

Offline static review. Linking proves control of the new provider and checks that the current session is live. It does not require fresh proof from an existing factor. The added login identity can outlast revocation of that session.

#### Dataflow

The canonical finding records the affected path at apps/api/src/identities.rs:77, but no expanded source-to-sink narrative was recorded.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

#### Severity

**Medium** — Linking proves control of the new provider and checks that the current session is live. It does not require fresh proof from an existing factor. The added login identity can outlast revocation of that session.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Require recent session-bound authentication with an existing factor before linking or unlinking a provider.

<a id="finding-6"></a>

### [6] Staff recovery hashes provide limited offline guessing resistance

| Field | Value |
| --- | --- |
| Severity | medium |
| Confidence | high |
| Confidence rationale | Verified against current source. Deployment and runtime behavior were not tested. |
| Category | recovery-code-storage |
| CWE | CWE-916 |
| Affected lines | apps/api/src/security.rs:241 |

#### Summary

Staff recovery codes contain 40 random bits and use a fast unkeyed digest. Database hash exposure permits offline verification. Staff access additionally requires a valid first-factor session and an unused code.

#### Root Cause

Staff recovery codes contain 40 random bits and use a fast unkeyed digest. Database hash exposure permits offline verification. Staff access additionally requires a valid first-factor session and an unused code.

**Staff recovery hashes provide limited offline guessing resistance** — `apps/api/src/security.rs:241`

Staff recovery codes contain 40 random bits and use a fast unkeyed digest. Database hash exposure permits offline verification. Staff access additionally requires a valid first-factor session and an unused code.

```
let mut b = [0u8; 5];
```

#### Validation

Offline static review. Staff recovery codes contain 40 random bits and use a fast unkeyed digest. Database hash exposure permits offline verification. Staff access additionally requires a valid first-factor session and an unused code.

#### Dataflow

The canonical finding records the affected path at apps/api/src/security.rs:241, but no expanded source-to-sink narrative was recorded.

#### Reachability

Reachability was not recorded beyond the canonical finding summary and affected locations.

#### Severity

**Medium** — Staff recovery codes contain 40 random bits and use a fast unkeyed digest. Database hash exposure permits offline verification. Staff access additionally requires a valid first-factor session and an unused code.

Additional runtime or deployment evidence could raise or lower this severity.

#### Remediation

Rotate existing recovery codes and generate at least 128 random bits per code.

## Reviewed Surfaces

| Surface | Risk Area | Outcome | Notes |
| --- | --- | --- | --- |
| Sharing a wishlist reveals restricted donors’ names and points | not recorded | Reported | No additional canonical notes were recorded. |
| Late top-up settlement can exceed spending controls | not recorded | Reported | No additional canonical notes were recorded. |
| Browser reports logout success after revocation fails | not recorded | Reported | No additional canonical notes were recorded. |
| Legacy guest recovery cookie lacks Secure | not recorded | Reported | No additional canonical notes were recorded. |
| Staff recovery hashes provide limited offline guessing resistance | not recorded | Reported | No additional canonical notes were recorded. |
| A stolen session can persist account access through identity linking | not recorded | Reported | No additional canonical notes were recorded. |

## Open Questions And Follow Up

- Files outside the focused source reviews have not received an individual security audit.
  - Follow-up prompt: Review deferred unit deferred-933c5650684dab2d and close its stated proof gap.
