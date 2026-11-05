#!/usr/bin/env python3
"""Exercise scripts/vps/wishly-autodeploy.sh against a fake curl: no GHCR, Dokploy, or root needed."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("vps") / "wishly-autodeploy.sh"
SECRETS = {"GHCR_TOKEN": "ghp_secret_pat", "DOKPLOY_API_KEY": "dokploy_secret_key"}

# Fake curl: answers the three endpoints from env, logs argv and stdin of every call.
FAKE_CURL = r'''#!/usr/bin/env python3
import json, os, sys
stdin = sys.stdin.read() if not sys.stdin.isatty() else ""
url = next(a for a in sys.argv[1:] if a.startswith("http"))
with open(os.environ["CURL_LOG"], "a") as log:
    log.write(json.dumps({"argv": sys.argv[1:], "stdin": stdin, "url": url}) + "\n")
if "/token" in url:
    print('{"token":"bearer-xyz"}')
elif "/manifests/latest" in url:
    print("HTTP/2 200\r\ndocker-content-digest: " + os.environ["FAKE_DIGEST"] + "\r\n\r")
elif url.endswith("/api/application.deploy"):
    sys.exit(int(os.environ.get("FAKE_DEPLOY_EXIT", "0")))
'''


class AutodeployTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        bin_dir = self.dir / "bin"
        bin_dir.mkdir()
        (bin_dir / "curl").write_text(FAKE_CURL)
        (bin_dir / "flock").write_text("#!/bin/sh\nexit 0\n")  # macOS 沒有 flock；鎖本身不在測試範圍
        for name in ("curl", "flock"):
            (bin_dir / name).chmod(0o755)
        env_file = self.dir / "autodeploy.env"
        env_file.write_text(f"GHCR_USER=bot\nGHCR_TOKEN={SECRETS['GHCR_TOKEN']}\n"
                            f"DOKPLOY_API_KEY={SECRETS['DOKPLOY_API_KEY']}\nDOKPLOY_APP_ID=app-123\n")
        self.state = self.dir / "state"
        self.log = self.dir / "curl.log"
        self.env = dict(os.environ, PATH=f"{bin_dir}:{os.environ['PATH']}", CURL_LOG=str(self.log),
                        WISHLY_AUTODEPLOY_ENV=str(env_file), WISHLY_AUTODEPLOY_STATE=str(self.state))

    def run_once(self, digest, deploy_exit=0):
        self.log.unlink(missing_ok=True)
        env = dict(self.env, FAKE_DIGEST=digest, FAKE_DEPLOY_EXIT=str(deploy_exit))
        result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True, capture_output=True)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []
        return result, calls

    def deploys(self, calls):
        return [c for c in calls if c["url"].endswith("/api/application.deploy")]

    def recorded(self):
        return (self.state / "deployed-digest").read_text().strip()

    def test_first_run_records_without_deploying_then_deploys_only_on_change(self):
        result, calls = self.run_once("sha256:aaa")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.deploys(calls), [])
        self.assertEqual(self.recorded(), "sha256:aaa")

        result, calls = self.run_once("sha256:aaa")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.deploys(calls), [])

        result, calls = self.run_once("sha256:bbb")
        self.assertEqual(result.returncode, 0, result.stderr)
        [deploy] = self.deploys(calls)
        self.assertEqual(deploy["url"], "http://localhost:3000/api/application.deploy")
        self.assertIn("x-api-key: " + SECRETS["DOKPLOY_API_KEY"], deploy["stdin"])
        body = json.loads(deploy["argv"][deploy["argv"].index("-d") + 1])
        self.assertEqual(body["applicationId"], "app-123")
        self.assertEqual(self.recorded(), "sha256:bbb")

    def test_failed_deploy_is_retried_on_next_run(self):
        self.run_once("sha256:aaa")
        result, calls = self.run_once("sha256:bbb", deploy_exit=22)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.deploys(calls)), 1)
        self.assertEqual(self.recorded(), "sha256:aaa")
        result, calls = self.run_once("sha256:bbb")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.deploys(calls)), 1)
        self.assertEqual(self.recorded(), "sha256:bbb")

    def test_missing_digest_fails_without_touching_state(self):
        self.run_once("sha256:aaa")
        result, calls = self.run_once("")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.deploys(calls), [])
        self.assertEqual(self.recorded(), "sha256:aaa")

    def test_secrets_never_appear_in_process_arguments(self):
        self.run_once("sha256:aaa")
        _, calls = self.run_once("sha256:bbb")
        self.assertTrue(calls)
        for call in calls:
            for secret in SECRETS.values():
                self.assertNotIn(secret, " ".join(call["argv"]))


if __name__ == "__main__":
    unittest.main()
