# Security Audit Report: Wishly (`erictheng93/wishly`)

**Target Repository**: `erictheng93/wishly`  
**Audit Framework**: Cloudflare Security Audit Specification  
**Audit Run**: `wishly/run-1`  
**Date**: 2026-11-20  
**Overall Posture**: **Strong** with isolated edge-case defects identified.

---

## Executive Summary

A comprehensive source code security audit was conducted on the `erictheng93/wishly` repository, covering the Rust backend API (`apps/api`), the Nuxt 3 web frontend and Tauri 2 desktop client (`apps/web`), and the PostgreSQL schema migrations.

The codebase exhibits a mature security architecture with defensive programming practices, including advisory locks to prevent race conditions on balance mutations, constant-time comparisons on secret hashes, strict role-based access control (RBAC), database-level row and column constraints, and envelope encryption for sensitive personally identifiable information (PII).

Three confirmed findings were identified during the audit:
1. **[Medium / P2] `topup.rs`**: Incomplete Case-Sensitivity Handling in Payment MAC Calculation Filter.
2. **[Medium / P2] `uploads.rs`**: Unbounded In-Memory Buffering in Media Upload Confirmation Handler.
3. **[Low / P3] `native.client.ts`**: Inconsistent Protocol Scheme Handling in Native Client External Navigation.

Remediation patches and issue tracking details are detailed below.

---

## Scope & Architecture Review

| Component | Path | Language / Stack | Core Security Responsibilities |
| :--- | :--- | :--- | :--- |
| **API Server** | `apps/api/src` | Rust, Axum, SQLx, Tokio | Authentication, RBAC, payment processing, file upload management, Web Push notifications, crowdfunding ledger. |
| **Database Migrations** | `db/migrations` | PostgreSQL | ACID transactions, advisory locking, check constraints, foreign key cascades, triggers. |
| **Frontend & Native Client** | `apps/web` | Nuxt 3, Vue 3, Tauri 2 | Session handling, OAuth PKCE flows, external URL opening, secure storage integration. |

---

## Positive Security Controls Verified

During the audit, multiple critical subsystems were examined and verified to have robust defenses in place:

1. **Authentication & Session Management (`apps/api/src/auth.rs`, `session.rs`)**:
   - Argon2id password hashing with global concurrency limits and dummy hashing against timing attacks.
   - Cryptographically signed OAuth state cookies (`check_state`) with strict expiration timestamps and single-use validation.
   - Safe redirection parsing appending directly to base origin (`app_url()`), mitigating cross-domain open redirects.
2. **Payment Double-Confirmation (`apps/api/src/topup.rs`)**:
   - Out-of-band `QueryTradeInfo` verification prior to ledger credit.
   - Row-level database locking (`FOR UPDATE`) preventing concurrent balance double-crediting.
   - Audit logging for all payment anomalies (`topup.amount_mismatch`, `topup.query_mismatch`).
3. **Data Protection & Sealed Encryption (`apps/api/src/user_addresses.rs`, `sealed.rs`)**:
   - User shipping and home addresses are field-level encrypted using AES-256-GCM.
   - PII is masked before transmission in API responses and omitted from audit logs.
4. **Server-Side Request Forgery (SSRF) Protection (`apps/api/src/push.rs`)**:
   - Web Push endpoints are strictly validated against an allowlist of trusted vendor domains (FCM, Apple APNs, Mozilla Push, Windows Push).
   - Port restrictions (443 only) and userinfo stripping prevent proxy tunneling and SSRF.
5. **Staff Access Control & MFA (`apps/api/src/admin.rs`, `org_admin.rs`)**:
   - Role enforcement using const-generics (`Staff<ADMIN>`, `Staff<SUPPORT>`, `Staff<MODERATOR>`).
   - Mandatory MFA enforcement on staff operations.
   - Guard against self-demotion or removal of the last remaining administrator (`protect_last_admin`).

---

## Findings Summary

| ID | Title | Severity | Location | CWE | Status |
| :--- | :--- | :---: | :--- | :--- | :---: |
| **SEC-01** | Incomplete Case-Sensitivity Handling in Payment MAC Calculation Filter | **Medium (P2)** | `apps/api/src/topup.rs:132` | CWE-178, CWE-347 | Confirmed |
| **SEC-02** | Unbounded In-Memory Buffer in Media Upload Confirmation Handler | **Medium (P2)** | `apps/api/src/uploads.rs:958` | CWE-400, CWE-770 | Confirmed |
| **SEC-03** | Inconsistent Protocol Scheme Handling in Native Client External Navigation | **Low (P3)** | `apps/web/app/plugins/native.client.ts:17` | CWE-749 | Confirmed |

---

## Detailed Findings

### SEC-01: Incomplete Case-Sensitivity Handling in Payment MAC Calculation Filter
- **Fingerprint**: `wishly.api.topup.check_mac_case_sensitivity`
- **File**: `apps/api/src/topup.rs:130–135`, `487–498`
- **Severity**: Medium (P2)
- **CWE**: CWE-178: Improper Handling of Case Sensitivity / CWE-347: Improper Verification of Cryptographic Signature
- **Description**:
  In `check_mac()`, parameters are filtered using:
  ```rust
  let mut p: Vec<&(String, String)> = params
      .iter()
      .filter(|(k, _)| k != "CheckMacValue")
      .collect();
  p.sort_by_key(|(k, _)| k.to_lowercase());
  ```
  While parameter sorting is case-insensitive, the exclusion filter is strictly case-sensitive. If a payment gateway or client sends a case-varied parameter (e.g. `checkmacvalue`, `Checkmacvalue`), it is not excluded and enters the SHA-256 payload string, corrupting MAC verification. In addition, `param(&params, "CheckMacValue")` in `verified_form()` uses exact case string equality.
- **Remediation**:
  Use `!k.eq_ignore_ascii_case("CheckMacValue")` in `check_mac()`, and implement case-insensitive parameter retrieval for `CheckMacValue`.

---

### SEC-02: Unbounded In-Memory Buffer in Media Upload Confirmation Handler
- **Fingerprint**: `wishly.api.uploads.unbounded_s3_streaming_dos`
- **File**: `apps/api/src/uploads.rs:950–961`
- **Severity**: Medium (P2)
- **CWE**: CWE-400: Uncontrolled Resource Consumption / CWE-770: Allocation of Resources Without Limits or Throttling
- **Description**:
  In `confirm()`, the upload size check only executes if `res.content_length()` is `Some`:
  ```rust
  if res.content_length().is_some_and(|l| l as usize > MAX_BYTES) {
      let _ = s3_req("DELETE", &inc, None).await;
      return Err(AppError::invalid("/object_key", "IMAGE_REJECTED", "檔案超過 5 MB"));
  }
  let bytes = res.bytes().await...;
  ```
  If upstream S3 / R2 storage omits `Content-Length` or uses chunked transfer encoding, `res.content_length()` returns `None`, skipping the check. `res.bytes().await` then buffers the entire uploaded payload into RAM without limits. In contrast, `avatar_bytes()` in the same file correctly reads chunks sequentially and enforces `MAX_BYTES` dynamically.
- **Remediation**:
  Adopt the chunked streaming pattern used in `avatar_bytes()` to stream S3 GET responses and enforce the 5MB `MAX_BYTES` threshold incrementally as chunks arrive.

---

### SEC-03: Inconsistent Protocol Scheme Handling in Native Client External Navigation
- **Fingerprint**: `wishly.web.native_client.unrestricted_external_navigation`
- **File**: `apps/web/app/plugins/native.client.ts:14–18`, `apps/web/app/utils/native.ts:46–49`
- **Severity**: Low (P3)
- **CWE**: CWE-749: Exposed Dangerous Method or Function in Native/Desktop Hybrid Application
- **Description**:
  In `native.client.ts`, an anchor click listener intercepts all `a[target="_blank"]` links and calls `openExternal(a.href)` without validating URL schemes. While Tauri capabilities (`apps/web/src-tauri/capabilities/default.json`) restrict `opener:allow-open-url` to `https:`, `mailto:`, `sms:`, and `line:`, standard `http:` links fail with unhandled plugin permission errors, and non-Tauri fallbacks lack protocol sanitization.
- **Remediation**:
  Validate external URLs against an explicit scheme allowlist (`['https:', 'http:', 'mailto:', 'sms:', 'line:']`) prior to dispatching to `openExternal()`.

---

## Verification & Artifacts

- **Findings Schema Validation**:
  ```bash
  node /config/.gemini/config/skills/security-audit/validate-findings.cjs /config/security-audit-skill/wishly/run-1/findings.json
  # Result: PASS: 3 findings valid
  ```
- **Tracking Issues**:
  All findings are opened as GitHub issues on `erictheng93/wishly` with appropriate labels (`security`, `P1`/`P2`/`P3`, `area:...`, `rust`) and complete suggested remediation solutions added in the comment section.
