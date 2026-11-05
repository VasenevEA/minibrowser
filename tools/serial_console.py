"""A shell on an embedded board over its serial console (a USB gadget port or
a USB-UART adapter): log in, run commands, upload files with base64 + gzip.

Environment: MB_PORT (default: the first /dev/cu.usbmodem* or /dev/ttyACM*),
MB_LOGIN (default "root:root"), MB_BAUD (default 115200).
"""
import base64, glob, gzip, os, termios, time


class Console:
    def __init__(self):
        port = os.environ.get("MB_PORT") or next(iter(sorted(glob.glob("/dev/cu.usbmodem*") + glob.glob("/dev/ttyACM*"))), None)
        if not port:
            raise OSError("no serial port found (set MB_PORT)")
        self.fd = os.open(port, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
        a = termios.tcgetattr(self.fd)
        baud = getattr(termios, "B" + os.environ.get("MB_BAUD", "115200"))
        a[4] = a[5] = baud
        a[3] &= ~(termios.ICANON | termios.ECHO)
        termios.tcsetattr(self.fd, termios.TCSANOW, a)

    def read_until(self, pats, t=8):
        buf, end = b"", time.time() + t
        while time.time() < end:
            try:
                buf += os.read(self.fd, 65536)
            except BlockingIOError:
                time.sleep(0.03)
            s = buf.decode(errors="replace")
            for p in pats:
                if p in s:
                    return p, s
        return None, buf.decode(errors="replace")

    def send(self, s):
        os.write(self.fd, s.encode())

    def shell(self):
        """Gets to a root prompt from wherever the console is, logging in if asked."""
        user, _, password = os.environ.get("MB_LOGIN", "root:root").partition(":")
        self.send("\x03\r")
        p, _ = self.read_until(["login:", "# "], 5)
        if p == "login:":
            self.send(user + "\r")
            self.read_until(["assword:"], 5)
            self.send(password + "\r")
            p, _ = self.read_until(["# "], 8)
        if p != "# ":
            raise OSError("no shell prompt")
        self.send("stty -echo\r")
        self.read_until(["# "], 3)

    def run(self, cmd, t=8):
        self.send(cmd + "\r")
        p, s = self.read_until(["# "], t)
        if p is None:
            raise OSError("timeout: " + cmd[:40])
        return s

    def put(self, data: bytes, path: str):
        b = base64.b64encode(gzip.compress(data)).decode()
        self.run("rm -f /tmp/.up.b64")
        for i in range(0, len(b), 700):
            self.run(f"echo '{b[i:i + 700]}' >> /tmp/.up.b64")
        self.run(f"base64 -d /tmp/.up.b64 | gunzip > {path}.new && mv {path}.new {path}; rm -f /tmp/.up.b64", 20)

    def close(self):
        try:
            self.send("stty echo\r")
        finally:
            os.close(self.fd)
