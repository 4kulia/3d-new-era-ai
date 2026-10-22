import http.client
import ipaddress
import socket
import ssl
import threading
import time
from urllib.parse import urljoin, urlsplit

LIMIT = 20 * 1024 * 1024
HOPS = 3
DEADLINE_S = 60.0
CONNECT_S = 10.0
READ_S = 30.0
CONTEXT = ssl.create_default_context()
AGENT = "ia3d-generator/1"
REDIRECTS = {301, 302, 303, 307, 308}
NAT64 = ipaddress.IPv6Network("64:ff9b::/96")
SIXTOFOUR = ipaddress.IPv6Network("2002::/16")


class RefusedError(Exception):
    def __init__(self, message: str, *, permanent: bool) -> None:
        super().__init__(message)
        self.permanent = permanent


def refused(message: str) -> RefusedError:
    return RefusedError(message, permanent=True)


def trouble(message: str) -> RefusedError:
    return RefusedError(message, permanent=False)


def embedded(ip: ipaddress.IPv6Address) -> ipaddress.IPv4Address | None:
    if ip.ipv4_mapped is not None:
        return ip.ipv4_mapped
    if ip in NAT64:
        return ipaddress.IPv4Address(int(ip) & 0xFFFFFFFF)
    if ip in SIXTOFOUR:
        return ip.sixtofour
    if ip.teredo is not None:
        return ip.teredo[1]
    if int(ip) >> 32 == 0 and int(ip) > 1:
        return ipaddress.IPv4Address(int(ip))
    return None


def checked(address: str) -> ipaddress.IPv4Address | ipaddress.IPv6Address:
    ip = ipaddress.ip_address(address.split("%", 1)[0])
    if isinstance(ip, ipaddress.IPv6Address):
        inner = embedded(ip)
        if inner is not None:
            checked(str(inner))
            return ip
    if not ip.is_global or ip.is_multicast:
        raise refused(f"{address} is not on the public internet, and an image is fetched only from there")
    return ip


def public_address(host: str, port: int) -> str:
    try:
        found = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)
    except socket.gaierror as err:
        raise trouble(f"could not find {host}: {err}") from err
    addresses = [checked(str(info[4][0])) for info in found]
    if not addresses:
        raise trouble(f"{host} has no address")
    first = sorted(addresses, key=lambda ip: ip.version)[0]
    return str(first) if first.version == 4 else f"[{first}]"


class Cutoff:
    def __init__(self, seconds: float) -> None:
        self.deadline = time.monotonic() + seconds
        self.lock = threading.Lock()
        self.sockets: list[socket.socket] = []
        self.fired = False
        self.timer = threading.Timer(seconds, self.fire)
        self.timer.daemon = True
        self.timer.start()

    def fire(self) -> None:
        with self.lock:
            self.fired = True
            watched = list(self.sockets)
        for sock in watched:
            cut(sock)

    def watch(self, sock: socket.socket) -> None:
        with self.lock:
            self.sockets.append(sock)
            fired = self.fired
        if fired:
            cut(sock)

    def passed(self) -> bool:
        return self.fired or time.monotonic() >= self.deadline

    def left(self, url: str) -> float:
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise refused(f"{url} took over {DEADLINE_S:.0f} s")
        return remaining

    def cancel(self) -> None:
        self.timer.cancel()


def cut(sock: socket.socket) -> None:
    try:
        socket.socket.shutdown(sock, socket.SHUT_RDWR)
    except OSError:
        return


def opened(address: str, port: int, host: str, cutoff: Cutoff, *, tls: bool) -> socket.socket:
    raw = socket.create_connection((address.strip("[]"), port), timeout=min(CONNECT_S, cutoff.left(host)))
    raw.settimeout(min(READ_S, cutoff.left(host)))
    cutoff.watch(raw)
    if not tls:
        return raw
    secured = CONTEXT.wrap_socket(raw, server_hostname=host, do_handshake_on_connect=False)
    cutoff.watch(secured)
    secured.do_handshake()
    return secured


def read_body(response: http.client.HTTPResponse, url: str, cutoff: Cutoff) -> bytes:
    length = response.getheader("content-length")
    if length is not None and length.isdigit() and int(length) > LIMIT:
        raise refused(f"{url} is over {LIMIT // 1024 // 1024} MB")
    body = bytearray()
    while True:
        chunk = response.read1(64 * 1024)
        if cutoff.passed():
            raise refused(f"{url} took over {DEADLINE_S:.0f} s")
        if not chunk:
            return bytes(body)
        body.extend(chunk)
        if len(body) > LIMIT:
            raise refused(f"{url} is over {LIMIT // 1024 // 1024} MB")


def exchange(sock: socket.socket, host: str, target: str, url: str, cutoff: Cutoff) -> tuple[int, str | None, bytes]:
    connection = http.client.HTTPConnection(host)
    connection.sock = sock
    try:
        connection.request("GET", target, headers={"Host": host, "User-Agent": AGENT, "Accept": "image/*"})
        response = connection.getresponse()
        if response.status in REDIRECTS:
            return response.status, response.getheader("location"), b""
        if response.status != 200:
            return response.status, None, b""
        return response.status, None, read_body(response, url, cutoff)
    except (OSError, http.client.HTTPException) as err:
        if cutoff.passed():
            raise refused(f"{url} took over {DEADLINE_S:.0f} s") from err
        raise trouble(f"could not fetch {url}: {err}") from err
    finally:
        connection.close()


def download(url: str) -> bytes:
    cutoff = Cutoff(DEADLINE_S)
    try:
        for _ in range(HOPS + 1):
            parts = urlsplit(url)
            if parts.scheme != "https":
                raise refused(f"{url}: an image is fetched over https only")
            host = parts.hostname
            if not host:
                raise refused(f"{url} names no host")
            port = parts.port or 443
            address = public_address(host, port)
            try:
                sock = opened(address, port, host, cutoff, tls=True)
            except ssl.SSLCertVerificationError as err:
                raise refused(f"{url}: its certificate is not trusted: {err.verify_message}") from err
            except OSError as err:
                if cutoff.passed():
                    raise refused(f"{url} took over {DEADLINE_S:.0f} s") from err
                raise trouble(f"could not fetch {url}: {err}") from err
            target = (parts.path or "/") + (f"?{parts.query}" if parts.query else "")
            status, location, body = exchange(sock, host if parts.port is None else f"{host}:{port}", target, url, cutoff)
            if status in REDIRECTS:
                if not location:
                    raise refused(f"{url} redirects nowhere")
                url = urljoin(url, location)
                continue
            if 400 <= status < 500 and status not in {408, 429}:
                raise refused(f"{url} answered {status}")
            if status != 200:
                raise trouble(f"{url} answered {status}")
            return body
        raise refused(f"{url} redirects more than {HOPS} times")
    finally:
        cutoff.cancel()
