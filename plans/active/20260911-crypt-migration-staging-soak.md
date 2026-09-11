# Crypt aws-lc-rs migration — staging soak runbook

Release gate for the crypt OpenSSL→aws-lc-rs migration (PR #231, merged `0fec7c8`,
currently unreleased). This is operational validation that cannot run in CI: it proves
the migrated code preserves device identity against a real backend. A regression here is
a fleet-wide auth outage, so **no release containing #231 may ship until this passes.**

Companion: `plans/completed/20260910-crypt-aws-lc-rs.md` (the migration itself).

## Why this gate exists

The agent authenticates to the backend with a device RSA key: it signs an RS512 JWT
whose `kid` is the key fingerprint (SHA-256 of the SPKI DER); the backend looks the
device up by that fingerprint and verifies the signature with the stored public key.
Unit/golden tests already prove signatures and fingerprints are byte-identical across
the swap. What they cannot prove is the on-device upgrade seam: every device provisioned
by an OpenSSL agent has a **PKCS#1** private key on disk (`BEGIN RSA PRIVATE KEY`), and
the new aws-lc-rs code must keep reading it (label-dispatch path) and producing a
backend-accepted token. That end-to-end path needs a real device + real backend.

## Prerequisite (blocks the soak)

The migration is unreleased. Cut a prerelease from current `main` containing #231 —
e.g. `v0.10.3-beta.1` — via the normal release flow. Call its tag `$NEW` below.
The pre-migration baseline is `$OLD = v0.10.2` (latest stable, OpenSSL, 2026-09-06).

Confirm the split before starting:
- `$OLD` must NOT contain `0fec7c8` (`git tag --contains 0fec7c8` lists only `$NEW`).
- `$NEW` must contain it.

## Environment

Staging backend + two staging devices (or VMs) that can run the Debian agent:
`DEVICE_A` (upgrade path) and `DEVICE_B` (fresh-provision path). A staging provisioning
token. Install/provision use the repo's existing staging scripts
(`scripts/install/staging-install.sh` supports `--version=`, `staging-provision.sh`
handles enrolment).

## Procedure

### Test 1 — upgrade path (the load-bearing case)

1. On `DEVICE_A`, install the OLD agent pinned to the pre-migration release:
   `staging-install.sh --version=$OLD`, then `staging-provision.sh` with the token.
2. Confirm healthy baseline: device shows online in the staging backend/dashboard, a
   token was minted, and a test deployment syncs and applies.
3. Capture the on-disk key format — it must be PKCS#1:
   `head -1 /var/lib/miru/auth/private_key.pem` → `-----BEGIN RSA PRIVATE KEY-----`.
   Record the device's key fingerprint / `kid` from the backend for comparison.
4. Install the NEW agent over the top: `staging-install.sh --version=$NEW` (do NOT
   re-provision — the existing key must be reused). Restart the service.
5. **Pass criteria:**
   - Token refresh succeeds against the staging backend after upgrade (agent logs show
     a successful refresh; device stays online — no auth/401 errors).
   - The private key on disk is unchanged and still PKCS#1 (upgrade must not rewrite or
     re-key it); fingerprint/`kid` matches the value from step 3.
   - A new deployment still syncs and applies post-upgrade.

### Test 2 — fresh provision on the new binary

1. On a clean `DEVICE_B`, install NEW directly: `staging-install.sh --version=$NEW`,
   then `staging-provision.sh` with the token.
2. **Pass criteria:**
   - Provisioning succeeds; device registers and comes online.
   - The newly written private key is PKCS#8:
     `head -1 /var/lib/miru/auth/private_key.pem` → `-----BEGIN PRIVATE KEY-----`
     (public key stays SPKI `BEGIN PUBLIC KEY`).
   - Token mint + a deployment sync/apply succeed.

## Rollback / safety

Both tests run on staging only. Test 1's rollback is reinstalling `$OLD`; because the
new code writes PKCS#8 only on *fresh* keygen, an upgraded device keeps its original
PKCS#1 key, so downgrading is safe (old code reads PKCS#1). Test 2's device can be
de-provisioned and wiped.

## Sign-off

Record pass/fail for both tests, the `$OLD`/`$NEW` tags used, and the fingerprint match
from Test 1 step 5, in this file's outcome section (or link the run). Only then is a
release containing #231 clear to ship.

## Outcome

(Fill in when executed: date, tags, operator, per-test result, fingerprint comparison.)
