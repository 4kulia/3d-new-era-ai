import socket
import socketserver
import ssl
import subprocess
import tempfile
import threading
import time
import unittest
from contextlib import suppress
from unittest.mock import patch

import fetch


class Dribble(socketserver.BaseRequestHandler):
    answer = b"HTTP/1.1 200 OK\r\nContent-Length: 40\r\n\r\n" + b"x" * 40
    headers_slowly = True

    def handle(self) -> None:
        self.request.recv(4096)
        split = self.answer.index(b"\r\n\r\n") + 4
        fast, slow = (b"", self.answer) if self.headers_slowly else (self.answer[:split], self.answer[split:])
        try:
            self.request.sendall(fast)
            for byte in slow:
                self.request.sendall(bytes([byte]))
                time.sleep(0.1)
        except OSError:
            return


class SlowBody(Dribble):
    headers_slowly = False


class AddressTest(unittest.TestCase):
    def test_an_address_off_the_public_internet_is_refused_in_every_form(self) -> None:
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.178.33",
            "169.254.169.254",
            "100.76.21.14",
            "0.0.0.0",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
            "2002:a00:1::1",
            "2001:0:4136:e378:8000:63bf:f5ff:fffe",
            "fe80::1",
            "fd00::1",
            "ff02::1",
        ]:
            with self.subTest(address=address), self.assertRaises(fetch.RefusedError) as refused:
                fetch.checked(address)
            self.assertTrue(refused.exception.permanent, address)

    def test_an_address_on_the_public_internet_is_let_through(self) -> None:
        for address in ["8.8.8.8", "185.70.186.229", "2a00:1450::1", "64:ff9b::808:808", "2002:808:808::1"]:
            with self.subTest(address=address):
                fetch.checked(address)


class DeadlineTest(unittest.TestCase):
    def cut_off(self, handler: type[socketserver.BaseRequestHandler]) -> None:
        server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
        server.daemon_threads = True
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            started = time.monotonic()
            cutoff = fetch.Cutoff(1.0)
            url = "http://127.0.0.1/photo.jpg"
            with self.assertRaises(fetch.RefusedError) as cut:
                sock = fetch.opened("127.0.0.1", server.server_address[1], "127.0.0.1", cutoff, tls=False)
                fetch.exchange(sock, "127.0.0.1", "/photo.jpg", url, cutoff)
            cutoff.cancel()
            self.assertLess(time.monotonic() - started, 2.0)
            self.assertTrue(cut.exception.permanent)
            self.assertIn("took over", str(cut.exception))
        finally:
            server.shutdown()
            server.server_close()

    def test_a_status_line_and_headers_dribbled_in_are_cut_off_at_the_deadline(self) -> None:
        self.cut_off(Dribble)

    def test_a_photo_that_trickles_in_is_cut_off_at_the_deadline(self) -> None:
        self.cut_off(SlowBody)


class CertificateTest(unittest.TestCase):
    def test_a_photo_host_with_an_untrusted_certificate_is_refused_at_once(self) -> None:
        with tempfile.TemporaryDirectory() as folder:
            key, cert = f"{folder}/key.pem", f"{folder}/cert.pem"
            subprocess.run(
                [
                    *("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1"),
                    *("-subj", "/CN=photos.example", "-keyout", key, "-out", cert),
                ],
                check=True,
                capture_output=True,
            )
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(cert, key)
            listener = socket.create_server(("127.0.0.1", 0))
            port = listener.getsockname()[1]

            def serve() -> None:
                conn, _ = listener.accept()
                with suppress(OSError), context.wrap_socket(conn, server_side=True):
                    pass

            threading.Thread(target=serve, daemon=True).start()
            try:
                with (
                    patch.object(fetch, "public_address", return_value="127.0.0.1"),
                    self.assertRaises(fetch.RefusedError) as cut,
                ):
                    fetch.download(f"https://photos.example:{port}/photo.jpg")
                self.assertTrue(cut.exception.permanent)
                self.assertIn("certificate is not trusted", str(cut.exception))
            finally:
                listener.close()


if __name__ == "__main__":
    unittest.main()
