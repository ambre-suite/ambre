#!/usr/bin/env python3
"""What `dev_stack.py` writes into a certificate, and what its probe refuses.

The certificates are written by the `openssl` of the host into a temporary
directory, and checked by `openssl verify`. The probe runs against a local HTTPS
server under such a certificate. The run against the dev container itself is the
CI `devcontainer` job, through `just devcontainer-check`.

Run: `just test-scripts`, or `python3 -m unittest discover -s scripts`.
"""

from __future__ import annotations

import http.server
import ssl
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))

import dev_stack  # noqa: E402  -- the module under test
from dev_stack import Answer, Paths, StackError  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TRUNK = '[serve.headers]\n"Cross-Origin-Opener-Policy" = "same-origin"\n"Cross-Origin-Embedder-Policy" = "credentialless"\n'
ISOLATED = {"Cross-Origin-Opener-Policy": "same-origin", "Cross-Origin-Embedder-Policy": "credentialless"}


class TheSubjectNames(unittest.TestCase):
    def test_no_host_names_loopback_alone(self) -> None:
        self.assertEqual(dev_stack.subject_names(None), ["DNS:localhost", "IP:127.0.0.1"])

    def test_a_private_address_is_added(self) -> None:
        self.assertEqual(dev_stack.subject_names("192.168.1.20")[-1], "IP:192.168.1.20")

    def test_loopback_is_not_named_twice(self) -> None:
        self.assertEqual(dev_stack.subject_names("127.0.0.1"), ["DNS:localhost", "IP:127.0.0.1"])

    def test_a_local_name_is_added(self) -> None:
        self.assertEqual(dev_stack.subject_names("laptop.local")[-1], "DNS:laptop.local")

    def test_a_public_address_is_refused(self) -> None:
        with self.assertRaisesRegex(StackError, "outside the private ranges"):
            dev_stack.subject_names("8.8.8.8")

    def test_a_shared_address_is_refused(self) -> None:
        with self.assertRaises(StackError):
            dev_stack.subject_names("100.64.0.1")

    def test_an_ipv6_address_is_refused(self) -> None:
        with self.assertRaises(StackError):
            dev_stack.subject_names("fd00::1")

    def test_a_public_name_is_refused(self) -> None:
        with self.assertRaisesRegex(StackError, "`.local` name"):
            dev_stack.subject_names("example.com")


class TheAuthority(unittest.TestCase):
    def test_it_signs_the_private_ranges_alone(self) -> None:
        constraints = next(e for e in dev_stack.authority_extensions() if e.startswith("nameConstraints"))
        for net in ("10.0.0.0/255.0.0.0", "172.16.0.0/255.240.0.0", "192.168.0.0/255.255.0.0"):
            self.assertIn(f"permitted;IP:{net}", constraints)
        self.assertTrue(constraints.startswith("nameConstraints=critical,"))

    def test_it_signs_servers_and_no_other_authority(self) -> None:
        self.assertIn("basicConstraints=critical,CA:TRUE,pathlen:0", dev_stack.authority_extensions())


def verify(paths: Paths) -> str:
    done = subprocess.run(
        ["openssl", "verify", "-CAfile", str(paths.authority), str(paths.cert)],
        capture_output=True, text=True, check=False,
    )
    return done.stdout + done.stderr


def names(paths: Paths) -> str:
    return subprocess.run(
        ["openssl", "x509", "-in", str(paths.cert), "-noout", "-ext", "subjectAltName"],
        capture_output=True, text=True, check=True,
    ).stdout


class TheCertificates(unittest.TestCase):
    def test_the_server_certificate_chains_to_the_authority(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = Paths(Path(tmp))
            self.assertTrue(dev_stack.write_certificates(paths, "10.0.0.7"))
            self.assertIn(": OK", verify(paths))
            self.assertIn("IP Address:10.0.0.7", names(paths))

    def test_a_second_run_keeps_both(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = Paths(Path(tmp))
            dev_stack.write_certificates(paths, None)
            authority = paths.authority.read_bytes()
            self.assertFalse(dev_stack.write_certificates(paths, None))
            self.assertEqual(paths.authority.read_bytes(), authority)

    def test_a_new_host_rewrites_the_server_certificate_alone(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = Paths(Path(tmp))
            dev_stack.write_certificates(paths, "192.168.1.2")
            authority = paths.authority.read_bytes()
            self.assertTrue(dev_stack.write_certificates(paths, "192.168.1.3"))
            self.assertEqual(paths.authority.read_bytes(), authority)
            self.assertIn("IP Address:192.168.1.3", names(paths))
            self.assertIn(": OK", verify(paths))

    def test_a_public_address_signed_by_the_authority_fails_verification(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = Paths(Path(tmp))
            dev_stack.write_certificates(paths, None)
            # Forge what the script refuses to write: a server certificate for a
            # public address, signed by the authority. The name constraints of the
            # authority must make every verifier refuse it.
            paths.names.unlink()
            forged = ["DNS:localhost", "IP:8.8.8.8"]
            with mock.patch.object(dev_stack, "subject_names", lambda host: forged):
                dev_stack.write_certificates(paths, None)
            self.assertIn("permitted subtree violation", verify(paths))

    def test_the_keys_are_readable_by_their_owner_alone(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = Paths(Path(tmp))
            dev_stack.write_certificates(paths, None)
            self.assertEqual(paths.key.stat().st_mode & 0o077, 0)
            self.assertEqual(paths.authority_key.stat().st_mode & 0o077, 0)


class TheIsolationHeaders(unittest.TestCase):
    def test_they_are_read_from_trunk_toml(self) -> None:
        self.assertEqual(dev_stack.isolation_headers((ROOT / "Trunk.toml").read_text()), ISOLATED)

    def test_a_missing_header_fails(self) -> None:
        with self.assertRaisesRegex(StackError, "Cross-Origin-Embedder-Policy"):
            dev_stack.isolation_headers('[serve.headers]\n"Cross-Origin-Opener-Policy" = "same-origin"\n')


class TheBreaches(unittest.TestCase):
    def test_an_isolated_page_passes(self) -> None:
        page = Answer(200, {k.lower(): v for k, v in ISOLATED.items()})
        self.assertEqual(dev_stack.page_breaches(page, ISOLATED), [])

    def test_a_page_with_no_embedder_policy_fails(self) -> None:
        page = Answer(200, {"cross-origin-opener-policy": "same-origin"})
        found = dev_stack.page_breaches(page, ISOLATED)
        self.assertEqual(len(found), 1)
        self.assertIn("Cross-Origin-Embedder-Policy", found[0])

    def test_a_relay_that_refuses_fails(self) -> None:
        self.assertIn("HTTP 502", dev_stack.relay_breaches(Answer(502, {}))[0])


class Handler(http.server.BaseHTTPRequestHandler):
    """The page with the headers of `headers`, and the relay route with `relay`."""

    headers_to_send: dict[str, str] = {}
    relay = 200

    def do_GET(self) -> None:  # noqa: N802 -- the name `http.server` calls
        status = self.relay if self.path == dev_stack.RELAY_PROBE else 200
        self.send_response(status)
        for name, value in self.headers_to_send.items():
            self.send_header(name, value)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, *args: object) -> None:
        pass


class TheProbe(unittest.TestCase):
    """The probe against a local HTTPS server under a certificate of the stack."""

    def serve(self, headers: dict[str, str], relay: int) -> str:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.paths = Paths(Path(tmp.name))
        dev_stack.write_certificates(self.paths, None)
        handler = type("H", (Handler,), {"headers_to_send": headers, "relay": relay})
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.paths.cert, self.paths.key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return f"https://localhost:{server.server_address[1]}"

    def test_an_isolated_stack_passes(self) -> None:
        origin = self.serve(ISOLATED, 200)
        self.assertEqual(dev_stack.probe(origin, self.paths.authority, TRUNK), [])

    def test_a_stack_with_no_isolation_fails(self) -> None:
        origin = self.serve({}, 200)
        self.assertEqual(len(dev_stack.probe(origin, self.paths.authority, TRUNK)), 2)

    def test_a_relay_that_is_down_fails(self) -> None:
        origin = self.serve(ISOLATED, 502)
        self.assertIn("HTTP 502", dev_stack.probe(origin, self.paths.authority, TRUNK)[0])

    def test_another_authority_fails(self) -> None:
        origin = self.serve(ISOLATED, 200)
        with tempfile.TemporaryDirectory() as tmp:
            other = Paths(Path(tmp))
            dev_stack.write_certificates(other, None)
            with self.assertRaisesRegex(StackError, "CERTIFICATE_VERIFY_FAILED"):
                dev_stack.probe(origin, other.authority, TRUNK)

    def test_a_missing_authority_is_waited_for(self) -> None:
        with self.assertRaisesRegex(StackError, "no authority yet"):
            dev_stack.probe("https://localhost:1", Path("/nonexistent/ca.pem"), TRUNK)

    def test_a_silent_origin_is_one_breach(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            paths = Paths(Path(tmp))
            dev_stack.write_certificates(paths, None)
            found = dev_stack.probe_until("https://localhost:1", paths.authority, TRUNK, 0)
        self.assertEqual(len(found), 1)
        self.assertIn("https://localhost:1/", found[0])


if __name__ == "__main__":
    unittest.main()
