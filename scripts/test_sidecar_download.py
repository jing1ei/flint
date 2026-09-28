"""Offline regression tests for the macOS downloader (stdlib, curl, unzip)."""
import hashlib
import http.server
import io
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
import zipfile


SCRIPT = Path(__file__).with_name("fetch-sidecars.sh")
TRIPLE = "aarch64-apple-darwin"


def archive(name, exit_code=0):
    output = io.BytesIO()
    with zipfile.ZipFile(output, "w") as bundle:
        bundle.writestr(name, f"#!/bin/sh\nprintf '{name} version fixture\\n'\nexit {exit_code}\n")
        bundle.writestr("padding", b"x" * 12000)
    return output.getvalue()


class DownloaderTests(unittest.TestCase):
    def test_default_download_matches_requested_target_not_host(self):
        commands = self.root / "commands"
        commands.mkdir()
        curl = commands / "curl"
        curl.write_text('#!/bin/sh\nfor arg do printf "%s\\n" "$arg"; done > "$CAPTURE_ARGS"\nexit 22\n')
        curl.chmod(0o755)
        capture = self.root / "args"
        env = {**os.environ, "TARGET_TRIPLE": "x86_64-apple-darwin",
               "SIDECAR_CACHE_DIR": str(self.cache), "CAPTURE_ARGS": str(capture),
               "PATH": str(commands) + os.pathsep + os.environ['PATH']}
        env.pop('FFMPEG_URL', None)
        env.pop('FFPROBE_URL', None)
        result = subprocess.run(['sh', str(self.root / 'scripts' / SCRIPT.name)],
                                env=env, capture_output=True, text=True, timeout=20)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('https://evermeet.cx/ffmpeg/getrelease/ffmpeg/zip', capture.read_text())

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "scripts").mkdir()
        (self.root / "src-tauri" / "binaries").mkdir(parents=True)
        shutil.copyfile(SCRIPT, self.root / "scripts" / SCRIPT.name)
        self.ffmpeg = self.root / "ffmpeg.zip"
        self.ffmpeg.write_bytes(archive("ffmpeg"))
        self.payload = archive("ffprobe")
        self.mode = "interrupt_once"
        self.calls = []
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                header = self.headers.get("Range")
                owner.calls.append(header)
                offset = int(header.removeprefix("bytes=").removesuffix("-")) if header else 0
                if owner.mode == "no_range" and header:
                    offset = 0
                if owner.mode == "range_changed" and header:
                    self.send_response(416)
                    self.end_headers()
                    return
                body = owner.payload[offset:]
                self.send_response(206 if offset else 200)
                self.send_header("Content-Length", str(len(body)))
                if offset:
                    self.send_header("Content-Range", f"bytes {offset}-{len(owner.payload)-1}/{len(owner.payload)}")
                self.end_headers()
                interrupted = owner.mode == "interrupt_always" or (
                    owner.mode in ("interrupt_once", "timeout_once") and len(owner.calls) == 1
                )
                self.wfile.write(body[:max(1, len(body) // 2)] if interrupted else body)
                if owner.mode == "timeout_once" and len(owner.calls) == 1:
                    self.wfile.flush()
                    time.sleep(2)
                self.close_connection = True

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(thread.join, 2)
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.url = f"http://127.0.0.1:{self.server.server_port}/ffprobe.zip"
        self.cache = self.root / "cache"
        key = hashlib.sha256(f"ffprobe\n{TRIPLE}\n{self.url}\n".encode()).hexdigest()
        self.partial = self.cache / f"{key}.part"
        self.complete = self.cache / f"{key}.archive"

    def run_script(self, **overrides):
        env = {**os.environ, "TARGET_TRIPLE": TRIPLE, "FFMPEG_URL": self.ffmpeg.as_uri(),
               "FFPROBE_URL": self.url, "SIDECAR_CACHE_DIR": str(self.cache),
               "SIDECAR_DOWNLOAD_TIMEOUT": "10", "FORCE": "0", **overrides}
        return subprocess.run(["sh", str(self.root / "scripts" / SCRIPT.name)],
                              env=env, capture_output=True, text=True, timeout=45)

    def seed_partial(self):
        self.cache.mkdir()
        self.partial.write_bytes(self.payload[:2000])

    def test_interruption_resumes_and_complete_cache_is_reusable(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.calls[:2], [None, f"bytes={len(self.payload) // 2}-"])
        self.assertEqual(self.complete.read_bytes(), self.payload)
        self.assertFalse(self.partial.exists())
        (self.root / "src-tauri/binaries/ffprobe-aarch64-apple-darwin").unlink()
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls), 2, "cache must avoid another request")

    def test_exhausted_retries_keep_progress_for_next_invocation(self):
        self.mode = "interrupt_always"
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("partial data was kept", result.stderr)
        size = self.partial.stat().st_size
        self.assertGreater(size, 0)
        self.assertEqual(list(self.cache.glob("*.lock")), [])
        self.mode = "normal"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls[-1], f"bytes={size}-")

    def test_timeout_keeps_bytes_and_next_attempt_resumes(self):
        self.mode = "timeout_once"
        result = self.run_script(SIDECAR_DOWNLOAD_TIMEOUT="1")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("curl: (28)", result.stderr)
        self.assertEqual(self.calls, [None, f"bytes={len(self.payload) // 2}-"])
        self.assertEqual(self.complete.read_bytes(), self.payload)

    def test_server_without_range_support_restarts_cleanly(self):
        self.seed_partial()
        self.mode = "no_range"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls, ["bytes=2000-", None])
        self.assertEqual(self.complete.read_bytes(), self.payload)

    def test_changed_remote_range_restarts_cleanly(self):
        self.seed_partial()
        self.mode = "range_changed"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls, ["bytes=2000-", None])

    def test_corrupt_archive_is_evicted_without_installing(self):
        self.payload = b"PK\x03\x04invalid zip"
        self.mode = "normal"
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.complete.exists())
        self.assertFalse((self.root / "src-tauri/binaries/ffprobe-aarch64-apple-darwin").exists())

    def test_failed_forced_refresh_preserves_installed_binary(self):
        self.mode = "normal"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        dest = self.root / "src-tauri/binaries/ffprobe-aarch64-apple-darwin"
        original = dest.read_bytes()
        self.payload = b"invalid archive"
        result = self.run_script(FORCE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(dest.read_bytes(), original)

    def test_failed_version_command_does_not_replace_installed_binary(self):
        self.mode = "normal"
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        dest = self.root / "src-tauri/binaries/ffprobe-aarch64-apple-darwin"
        original = dest.read_bytes()
        self.payload = archive("ffprobe", exit_code=1)
        result = self.run_script(FORCE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(dest.read_bytes(), original)

    def test_check_rejects_version_output_from_a_failing_binary(self):
        for name in ("ffmpeg", "ffprobe"):
            dest = self.root / f"src-tauri/binaries/{name}-{TRIPLE}"
            dest.write_text(f"#!/bin/sh\nprintf '{name} version fixture\\n'\nexit 1\n")
            dest.chmod(0o755)
        result = subprocess.run(
            ["sh", str(self.root / "scripts" / SCRIPT.name), "--check"],
            env={**os.environ, "TARGET_TRIPLE": TRIPLE}, capture_output=True, text=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("Both sidecars are ready", result.stdout)


if __name__ == "__main__":
    unittest.main()
