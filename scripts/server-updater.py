#!/usr/bin/env python3
"""Beans server updater: one pass that installs the latest signed Beans server release.

It reads a root-owned JSON config, asks GitHub for the latest stable release of bloodf/beans,
verifies beans-update.json against its detached Ed25519 signature with openssl and the Beans
public key, then downloads the server archive for this host's platform from a URL derived from
the fixed repository, the release tag, and the asset name. The archive's size and SHA-256 must
match the signed manifest before it is extracted into a private staging directory, and it may
hold only lorca, lorca-relay, models/v1.json, and marketplace/v1.json.

The relay goes first: its SQLite database is backed up through the SQLite backup interface as the database's owner (or the config acknowledges an external backup), its
binary and public catalogs are swapped atomically, and it must answer health with at least the
signed manifest's protocol and the artifact's version from a restarted process running the new binary. Each Runner
then holds new work back through `lorca update prepare` until it is idle, is stopped, swapped,
started, and verified the same way. A failure rolls that target back to its previous binary and
catalogs (never its database) and stops the rollout. The release's signed beans-server-updater.py
is checked before any service changes and replaces the updater only after every target succeeds.

Python 3 standard library and the openssl command only. Run as root, from the timer unit.
"""

import argparse
import base64
import fcntl
import hashlib
import http.client
import json
import os
import re
import shutil
import sqlite3
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

REPOSITORY = "bloodf/beans"
API_LATEST = "https://api.github.com/repos/" + REPOSITORY + "/releases/latest"
DOWNLOAD = "https://github.com/" + REPOSITORY + "/releases/download/{tag}/{name}"
MANIFEST = "beans-update.json"
SIGNATURE = MANIFEST + ".sig"
UPDATER_ASSET = "beans-server-updater.py"
MIN_PROTOCOL = 3

SEMVER = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
TAG = re.compile(r"^beans-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")
ASSET_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 ._-]{0,179}\Z")
LABEL = re.compile(r"^[a-z0-9][a-z0-9._-]{0,63}$")
SERVICE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9@._-]{0,200}\.service$")
INSTANCE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9-]{0,62}$")
TARGET_NAME = re.compile(r"^[a-z0-9][a-z0-9-]{0,62}$")
ED25519_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")

MAX_API_BYTES = 4 << 20
MAX_MANIFEST_BYTES = 1 << 20
MAX_SIGNATURE_BYTES = 1024
MAX_HEALTH_BYTES = 64 << 10
MAX_CONFIG_BYTES = 1 << 20
MAX_ARCHIVE_BYTES = 512 << 20
MAX_UPDATER_BYTES = 1 << 20
MAX_BINARY_BYTES = 384 << 20
MAX_CATALOG_BYTES = 16 << 20
ARCHIVE_FILES = {"lorca": "binary", "lorca-relay": "binary", "models/v1.json": "catalog", "marketplace/v1.json": "catalog"}
ARCHIVE_DIRS = {"models", "marketplace"}
CATALOGS = ("models/v1.json", "marketplace/v1.json")
ELF_MACHINE = {"linux-x86_64": 62, "linux-aarch64": 183}
HOST_PLATFORM = {"x86_64": "linux-x86_64", "amd64": "linux-x86_64", "aarch64": "linux-aarch64", "arm64": "linux-aarch64"}
PATH_ENV = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
EX_TEMPFAIL = 75
BACKUPS_KEPT = 3
POLL_SECONDS = 10
SERVICE_TIMEOUT = 300
USER_AGENT = "beans-server-updater"


class ConfigError(Exception):
    """The config, the key, or this host is not set up as the updater requires."""


class Failure(Exception):
    """The release or a target failed: the rollout stops and the version is recorded."""


class Retry(Exception):
    """Something outside the release kept this pass from finishing; the next pass tries again."""


def log(message):
    print("server-updater: " + message, file=sys.stderr, flush=True)


# MARK: - Files and config


def no_duplicates(pairs):
    seen = {}
    for key, value in pairs:
        if key in seen:
            raise ValueError("duplicate key " + repr(key))
        seen[key] = value
    return seen


def reject_constant(name):
    raise ValueError("non-standard JSON constant " + name)


def strict_json(data, what, error=Failure):
    try:
        return json.loads(data.decode("utf-8"), object_pairs_hook=no_duplicates, parse_constant=reject_constant)
    except (UnicodeDecodeError, ValueError) as exc:
        raise error("%s is not valid JSON: %s" % (what, exc))


def exact_keys(value, required, optional, what, error=Failure):
    if not isinstance(value, dict):
        raise error(what + " must be a JSON object")
    missing = sorted(set(required) - set(value))
    unknown = sorted(set(value) - set(required) - set(optional))
    if missing:
        raise error("%s lacks %s" % (what, ", ".join(missing)))
    if unknown:
        raise error("%s has unknown keys %s" % (what, ", ".join(unknown)))


def require_root_dir(path, what):
    st = os.lstat(path)
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != 0 or st.st_mode & 0o022:
        raise ConfigError("%s %s must be a root-owned directory nobody else can write" % (what, path))


def require_root_file(path, what, private):
    """A regular root-owned file in a root-owned directory; `private` also forbids reading."""
    try:
        st = os.lstat(path)
    except OSError:
        raise ConfigError("cannot find %s %s" % (what, path))
    if not stat.S_ISREG(st.st_mode) or st.st_uid != 0:
        raise ConfigError("%s %s must be a regular root-owned file" % (what, path))
    if st.st_mode & (0o077 if private else 0o022):
        raise ConfigError("%s %s must have mode %s" % (what, path, "0600" if private else "0644 or stricter"))
    require_root_dir(os.path.dirname(path) or "/", "the directory of " + what)


def read_capped(path, cap, what):
    with open(path, "rb") as handle:
        data = handle.read(cap + 1)
    if len(data) > cap:
        raise ConfigError(what + " is too large")
    return data


def absolute(value, what):
    if not isinstance(value, str) or not value.startswith("/") or os.path.normpath(value) != value or "\n" in value:
        raise ConfigError(what + " must be a normalized absolute path")
    return value


def whole(value, what, low, high):
    if type(value) is not int or not low <= value <= high:
        raise ConfigError("%s must be a whole number from %d to %d" % (what, low, high))
    return value


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_private(path, data, mode=0o600):
    """Writes `data` beside `path` and renames it into place, so readers see all or nothing."""
    temporary = path + ".beans-new"
    if os.path.lexists(temporary):
        os.unlink(temporary)
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    with os.fdopen(fd, "wb") as handle:
        handle.write(data)
        handle.flush()
        os.fsync(handle.fileno())
    os.chmod(temporary, mode)
    os.replace(temporary, path)
    directory = os.open(os.path.dirname(path) or "/", os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def sync_directory(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


# MARK: - The release


class HttpsOnlyRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise Retry("refusing a redirect away from HTTPS")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


OPENER = urllib.request.build_opener(HttpsOnlyRedirects)


class NoRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


# The relay's health is read directly: no environment proxy, no redirect.
HEALTH_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirects)


def fetch(url, cap, accept="application/octet-stream", sink=None):
    """GETs a public URL without credentials. Returns the body, or passes it chunk by chunk to
    `sink` and returns its length. A body longer than `cap` fails."""
    request = urllib.request.Request(url, headers={"Accept": accept, "User-Agent": USER_AGENT})
    try:
        with OPENER.open(request, timeout=60) as response:
            length = response.headers.get("Content-Length")
            if length is not None and length.isdigit() and int(length) > cap:
                raise Failure(url + " is larger than expected")
            total, chunks = 0, []
            while True:
                chunk = response.read(1 << 16)
                if not chunk:
                    break
                total += len(chunk)
                if total > cap:
                    raise Failure(url + " is larger than expected")
                if sink is None:
                    chunks.append(chunk)
                else:
                    sink(chunk)
            return b"".join(chunks) if sink is None else total
    except urllib.error.HTTPError as exc:
        raise Retry("%s answered HTTP %d" % (url, exc.code))
    except (urllib.error.URLError, http.client.HTTPException, OSError) as exc:
        raise Retry("%s: %s" % (url, exc))


def latest_release():
    """The tag of the latest stable release and the names of its assets."""
    release = strict_json(fetch(API_LATEST, MAX_API_BYTES, "application/vnd.github+json"), "the latest release", Retry)
    if not isinstance(release, dict):
        raise Retry("the latest release is not a JSON object")
    if release.get("draft") is not False or release.get("prerelease") is not False:
        raise Retry("the latest release is a draft or a prerelease")
    tag = release.get("tag_name")
    if not isinstance(tag, str) or not TAG.match(tag):
        raise Retry("the latest release tag %r is not beans-vX.Y.Z" % (tag,))
    names = {asset.get("name") for asset in release.get("assets") or [] if isinstance(asset, dict)}
    return tag, names


def release_url(tag, name):
    if not TAG.match(tag) or not ASSET_NAME.match(name):
        raise Failure("refusing to derive a download URL for %r %r" % (tag, name))
    return DOWNLOAD.format(tag=tag, name=urllib.parse.quote(name, safe=""))


def load_public_key(path):
    """The Beans public key as PEM, from its raw 32-byte base64 form."""
    require_root_file(path, "the Beans public key", private=False)
    try:
        raw = base64.b64decode(read_capped(path, 256, "the Beans public key").strip(), validate=True)
    except ValueError:
        raise ConfigError("the Beans public key is not base64")
    if len(raw) != 32:
        raise ConfigError("the Beans public key must be 32 raw Ed25519 bytes")
    der = base64.b64encode(ED25519_SPKI_PREFIX + raw).decode()
    return "-----BEGIN PUBLIC KEY-----\n" + der + "\n-----END PUBLIC KEY-----\n"


def verify_signature(pem, manifest, signature_text, work):
    """Checks the detached Ed25519 signature over the exact manifest bytes with openssl."""
    try:
        signature = base64.b64decode(signature_text.strip(), validate=True)
    except ValueError:
        raise Failure(SIGNATURE + " is not base64")
    if len(signature) != 64:
        raise Failure(SIGNATURE + " must hold 64 raw Ed25519 bytes")
    paths = {}
    for name, data in (("key.pem", pem.encode()), ("manifest", manifest), ("signature", signature)):
        paths[name] = os.path.join(work, name)
        write_private(paths[name], data)
    argv = ["openssl", "pkeyutl", "-verify", "-pubin", "-inkey", paths["key.pem"], "-rawin", "-in", paths["manifest"], "-sigfile", paths["signature"]]
    try:
        result = subprocess.run(argv, capture_output=True, env={"PATH": PATH_ENV}, timeout=60)
    except FileNotFoundError:
        raise ConfigError("openssl is not installed")
    except subprocess.TimeoutExpired:
        raise Failure("Beans signature verification timed out")
    except OSError as exc:
        raise ConfigError("cannot run openssl: %s" % exc)
    if result.returncode != 0:
        raise Failure(MANIFEST + " does not carry a valid Beans signature")


def version_tuple(text):
    return tuple(int(part) for part in text.split("."))


def parse_manifest(data, tag):
    """The signed manifest's artifacts by name, after every field is checked."""
    manifest = strict_json(data, MANIFEST)
    exact_keys(manifest, ("schema", "version", "revision", "protocol", "artifacts"), (), MANIFEST)
    if type(manifest["schema"]) is not int or manifest["schema"] != 1:
        raise Failure("unknown manifest schema %r" % (manifest["schema"],))
    version = manifest["version"]
    if not isinstance(version, str) or not SEMVER.match(version) or tag != "beans-v" + version:
        raise Failure("manifest version %r does not match release %s" % (version, tag))
    if not isinstance(manifest["revision"], str) or not HEX40.match(manifest["revision"]):
        raise Failure("manifest revision must be 40 lowercase hex characters")
    if type(manifest["protocol"]) is not int or manifest["protocol"] < MIN_PROTOCOL:
        raise Failure("manifest protocol %r is below %d" % (manifest["protocol"], MIN_PROTOCOL))
    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, list) or not artifacts:
        raise Failure("manifest artifacts must be a non-empty list")
    by_name = {}
    for artifact in artifacts:
        exact_keys(artifact, ("name", "sha256", "size", "component", "platform", "version"), (), "a manifest artifact")
        name = artifact["name"]
        if not isinstance(name, str) or not ASSET_NAME.match(name) or name in (MANIFEST, SIGNATURE):
            raise Failure("invalid artifact name %r" % (name,))
        if name in by_name:
            raise Failure("artifact %s is listed twice" % name)
        if not isinstance(artifact["sha256"], str) or not HEX64.match(artifact["sha256"]):
            raise Failure("artifact %s has an invalid sha256" % name)
        if type(artifact["size"]) is not int or artifact["size"] < 1:
            raise Failure("artifact %s has an invalid size" % name)
        for key in ("component", "platform"):
            if not isinstance(artifact[key], str) or not LABEL.match(artifact[key]):
                raise Failure("artifact %s has an invalid %s" % (name, key))
        if not isinstance(artifact["version"], str) or not SEMVER.match(artifact["version"]):
            raise Failure("artifact %s has an invalid version" % name)
        by_name[name] = artifact
    return by_name


def server_artifact(artifacts, platform):
    name = "beans-server-%s.tar.gz" % platform
    artifact = artifacts.get(name)
    if artifact is None:
        raise Failure("the release has no " + name)
    if artifact["component"] != "server" or artifact["platform"] != platform or artifact["size"] > MAX_ARCHIVE_BYTES:
        raise Failure("%s is not a server archive for %s" % (name, platform))
    return artifact


def download(tag, artifact, path, cap):
    """Streams an asset to `path`, failing unless its size and SHA-256 match the manifest."""
    if artifact["size"] > cap:
        raise Failure("%s is larger than allowed" % artifact["name"])
    digest = hashlib.sha256()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "wb") as handle:
        def sink(chunk):
            digest.update(chunk)
            handle.write(chunk)
        total = fetch(release_url(tag, artifact["name"]), artifact["size"], sink=sink)
    if total != artifact["size"] or digest.hexdigest() != artifact["sha256"]:
        raise Failure("%s does not match the signed manifest" % artifact["name"])


def check_elf(path, platform):
    with open(path, "rb") as handle:
        header = handle.read(20)
    if len(header) < 20 or header[:4] != b"\x7fELF" or header[4] != 2 or header[5] != 1 or int.from_bytes(header[18:20], "little") != ELF_MACHINE[platform]:
        raise Failure("%s is not a 64-bit executable for %s" % (os.path.basename(path), platform))


def extract(archive, platform, destination):
    """Unpacks the verified server archive by name: exactly lorca, lorca-relay, models/v1.json,
    and marketplace/v1.json as regular files. Links, devices, absolute or parent paths, and any
    other entry fail. Returns each file's staged path and SHA-256."""
    found = {}
    try:
        with tarfile.open(archive, "r:gz") as tar:
            for member in tar:
                name = member.name
                while name.startswith("./"):
                    name = name[2:]
                if member.isdir():
                    name = name.rstrip("/")
                    if name in ("", ".") or name in ARCHIVE_DIRS:
                        continue
                    raise Failure("archive directory %r is unexpected" % member.name)
                if name.startswith("/") or any(part in ("", ".", "..") for part in name.split("/")):
                    raise Failure("archive entry %r has an unsafe path" % member.name)
                if not member.isreg():
                    raise Failure("archive entry %r is not a regular file" % member.name)
                kind = ARCHIVE_FILES.get(name)
                if kind is None or name in found:
                    raise Failure("archive entry %r is unexpected" % member.name)
                cap = MAX_BINARY_BYTES if kind == "binary" else MAX_CATALOG_BYTES
                if member.size > cap:
                    raise Failure("archive entry %r is too large" % member.name)
                mode = 0o755 if kind == "binary" else 0o644
                target = os.path.join(destination, name.replace("/", "-"))
                source = tar.extractfile(member)
                digest, written = hashlib.sha256(), 0
                fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
                with os.fdopen(fd, "wb") as handle:
                    for chunk in iter(lambda: source.read(1 << 20), b""):
                        written += len(chunk)
                        if written > cap:
                            raise Failure("archive entry %r is too large" % member.name)
                        digest.update(chunk)
                        handle.write(chunk)
                    os.fchmod(handle.fileno(), mode)
                found[name] = {"path": target, "sha256": digest.hexdigest()}
    except (tarfile.TarError, EOFError, OSError) as exc:
        raise Failure("the server archive is not a readable tar.gz: %s" % exc)
    missing = sorted(set(ARCHIVE_FILES) - set(found))
    if missing:
        raise Failure("the server archive lacks " + ", ".join(missing))
    for name in ("lorca", "lorca-relay"):
        check_elf(found[name]["path"], platform)
    for name in CATALOGS:
        with open(found[name]["path"], "rb") as handle:
            strict_json(handle.read(), name)
    return found


# MARK: - Config


DEFAULTS = {
    "public_key": "/etc/beans/update-public-key.txt",
    "state_dir": "/var/lib/beans-updater",
    "lock_file": "/run/beans-server-updater.lock",
    "updater_path": "/usr/local/lib/beans/server-updater.py",
}


def load_config(path):
    """The admin's config: the relay first, then Runners in rollout order. Only the services,
    instances, and paths it names are touched."""
    require_root_file(path, "the config", private=True)
    config = strict_json(read_capped(path, MAX_CONFIG_BYTES, "the config"), "the config", ConfigError)
    exact_keys(config, ("relay",), ("runners", "public_key", "state_dir", "lock_file", "updater_path", "platform"), "the config", ConfigError)
    settings = {key: absolute(config.get(key, default), key) for key, default in DEFAULTS.items()}
    machine = os.uname().machine
    platform = config.get("platform", HOST_PLATFORM.get(machine))
    if platform not in ELF_MACHINE:
        raise ConfigError("platform %r is not linux-x86_64 or linux-aarch64 (this host is %s)" % (platform, machine))
    settings["platform"] = platform
    relay = parse_target(config["relay"], "relay", "relay")
    runners = config.get("runners", [])
    if not isinstance(runners, list):
        raise ConfigError("runners must be a list")
    targets = [relay] + [parse_target(runner, "runner", "runners[%d]" % index) for index, runner in enumerate(runners)]
    names = [target["name"] for target in targets]
    if len(set(names)) != len(names):
        raise ConfigError("every target needs its own name")
    settings["targets"] = targets
    return settings


def parse_target(value, role, what):
    common = ("name", "driver", "binary", "service", "health_url", "health_timeout")
    if role == "relay":
        optional = common + ("instance", "database", "catalog_dir", "backup_dir")
    else:
        optional = common + ("instance", "port", "token_file", "drain_timeout")
    if not isinstance(value, dict):
        raise ConfigError(what + " must be an object")
    exact_keys(value, ("name", "driver", "binary", "service"), optional, what, ConfigError)
    target = {"role": role, "name": value["name"], "driver": value["driver"]}
    if not isinstance(target["name"], str) or not TARGET_NAME.match(target["name"]):
        raise ConfigError(what + ".name must be lowercase letters, digits, and dashes")
    if target["driver"] == "incus":
        if not isinstance(value.get("instance"), str) or not INSTANCE.match(value["instance"]):
            raise ConfigError(what + ".instance must name the Incus instance")
        target["instance"] = value["instance"]
    elif target["driver"] == "systemd":
        if "instance" in value:
            raise ConfigError(what + ".instance applies only to the incus driver")
    else:
        raise ConfigError(what + ".driver must be systemd or incus")
    if not isinstance(value["service"], str) or not SERVICE.match(value["service"]):
        raise ConfigError(what + ".service must name a systemd .service unit")
    target["service"] = value["service"]
    target["binary"] = absolute(value["binary"], what + ".binary")
    expected = "lorca-relay" if role == "relay" else "lorca"
    if os.path.basename(target["binary"]) != expected:
        raise ConfigError("%s.binary must be a file named %s" % (what, expected))
    target["health_timeout"] = whole(value.get("health_timeout", 120), what + ".health_timeout", 5, 1800)
    if role == "relay":
        url = value.get("health_url", "http://127.0.0.1:8787/v1/health")
        split = urllib.parse.urlsplit(url) if isinstance(url, str) else None
        if split is None or split.scheme not in ("http", "https") or not split.hostname or split.path != "/v1/health" or split.query or split.username:
            raise ConfigError(what + ".health_url must be an http(s) URL ending in /v1/health")
        target["health_url"] = url
        database = value.get("database")
        if database == "external":
            if "backup_dir" in value:
                raise ConfigError(what + ".backup_dir does not apply when database is \"external\"")
            target["database"] = None
            target["backup_dir"] = None
        elif database is None:
            raise ConfigError(what + ".database is required: an absolute SQLite path, or \"external\" to confirm you back the relay database up yourself")
        else:
            target["database"] = absolute(database, what + ".database")
            target["backup_dir"] = absolute(value.get("backup_dir"), what + ".backup_dir")
            if target["driver"] == "incus":
                raise ConfigError(what + ".database backups need the systemd driver; use \"external\" for an Incus relay")
        target["catalog_dir"] = absolute(value["catalog_dir"], what + ".catalog_dir") if "catalog_dir" in value else None
    else:
        if "health_url" in value:
            raise ConfigError(what + ".health_url applies only to the relay")
        target["port"] = whole(value.get("port", 4862), what + ".port", 1, 65535)
        target["token_file"] = absolute(value.get("token_file"), what + ".token_file")
        target["drain_timeout"] = whole(value.get("drain_timeout", 1800), what + ".drain_timeout", 60, 86400)
    return target


# MARK: - Drivers


def run(argv, timeout=120, data=None, check=True, env=None):
    """Runs a fixed command with a minimal environment; its output never reaches the log."""
    environment = {"PATH": PATH_ENV, "LC_ALL": "C"}
    environment.update(env or {})
    try:
        result = subprocess.run(argv, input=data, capture_output=True, env=environment, timeout=timeout)
    except FileNotFoundError:
        raise ConfigError(argv[0] + " is not installed")
    except subprocess.TimeoutExpired:
        raise Failure("%s timed out" % " ".join(argv[:3]))
    if check and result.returncode != 0:
        raise Failure("%s exited %d" % (" ".join(argv[:3]), result.returncode))
    return result


def in_target(target, argv, timeout=120, data=None, check=True, env=None):
    """Runs `argv` on the target's host, or inside its Incus instance with the same minimal
    environment passed through `incus exec --env`."""
    if target["driver"] == "incus":
        environment = {"PATH": PATH_ENV, "LC_ALL": "C"}
        environment.update(env or {})
        flags = [flag for key, value in sorted(environment.items()) for flag in ("--env", key + "=" + value)]
        return run(["incus", "exec", target["instance"]] + flags + ["--"] + argv, timeout, data, check)
    return run(argv, timeout, data, check, env)


def systemctl(target, verb):
    in_target(target, ["systemctl", verb, target["service"]], timeout=SERVICE_TIMEOUT)


def main_pid(target):
    text = in_target(target, ["systemctl", "show", "--property=MainPID", "--value", target["service"]]).stdout.decode().strip()
    return int(text) if text.isdigit() else 0


def remote_sha256(target, path):
    if target["driver"] == "systemd":
        return sha256_file(path)
    output = in_target(target, ["sha256sum", "--", path]).stdout.decode().split()
    if not output or not HEX64.match(output[0]):
        raise Failure("cannot hash " + path)
    return output[0]


def remote_exists(target, path):
    if target["driver"] == "systemd":
        return os.path.lexists(path)
    return (in_target(target, ["test", "-e", path], check=False).returncode == 0
            or in_target(target, ["test", "-L", path], check=False).returncode == 0)


def remote_mode(target, path):
    """Owner, group, and permission bits of a regular file, without following its final link."""
    if target["driver"] == "systemd":
        st = os.lstat(path)
        if not stat.S_ISREG(st.st_mode):
            raise Failure(path + " is not a regular file")
        return st.st_uid, st.st_gid, stat.S_IMODE(st.st_mode)
    if in_target(target, ["test", "-L", path], check=False).returncode == 0:
        raise Failure(path + " is a symlink")
    output = in_target(target, ["stat", "-c", "%F %u %g %a", "--", path]).stdout.decode().strip()
    match = re.match(r"^regular (?:empty )?file (\d+) (\d+) ([0-7]+)$", output)
    if not match:
        raise Failure(path + " is not a regular file")
    return int(match.group(1)), int(match.group(2)), int(match.group(3), 8)


def require_target_path(target, path, executable=False, missing=False):
    """Checks every ancestor without following links before root reads, writes or executes."""
    directory = os.path.dirname(path)
    while True:
        if target["driver"] == "systemd":
            try:
                require_root_dir(directory, "the target directory")
            except OSError as exc:
                raise ConfigError("cannot inspect %s: %s" % (directory, exc))
        else:
            if in_target(target, ["test", "-L", directory], check=False).returncode == 0:
                raise ConfigError(directory + " is a symlink")
            result = in_target(target, ["stat", "-c", "%f %u", "--", directory], check=False)
            fields = result.stdout.decode().split()
            if (result.returncode != 0 or len(fields) != 2
                    or not re.fullmatch(r"[0-9a-fA-F]+", fields[0]) or not fields[1].isdigit()):
                raise ConfigError("cannot inspect target directory " + directory)
            mode, uid = int(fields[0], 16), int(fields[1])
            if not stat.S_ISDIR(mode) or uid != 0 or mode & 0o022:
                raise ConfigError(directory + " must be a root-owned directory nobody else can write, without symlinks")
        if directory == "/":
            break
        directory = os.path.dirname(directory)
    if missing and not remote_exists(target, path):
        return None
    try:
        owner = remote_mode(target, path)
    except (Failure, OSError) as exc:
        raise ConfigError("cannot inspect %s: %s" % (path, exc))
    uid, gid, mode = owner
    if uid != 0 or mode & 0o022 or (executable and (not mode & 0o111 or mode & 0o6222)):
        raise ConfigError(path + " must be a root-owned regular file " +
                          ("with executable bits, no write bits and no set-id bits" if executable else "nobody else can write"))
    return owner


def place(target, source, path, owner):
    """Puts `source` beside `path`, sets trusted ownership/mode, syncs, then renames it."""
    uid, gid, mode = owner
    staged = path + ".beans-new"
    if target["driver"] == "systemd":
        if os.path.lexists(staged):
            os.unlink(staged)
        fd = os.open(staged, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, "wb") as handle, open(source, "rb") as data:
            shutil.copyfileobj(data, handle, 1 << 20)
            handle.flush()
            os.fchown(handle.fileno(), uid, gid)
            os.fchmod(handle.fileno(), mode)
            os.fsync(handle.fileno())
        os.replace(staged, path)
        directory = os.open(os.path.dirname(path), os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        return
    in_target(target, ["rm", "-f", "--", staged])
    run(["incus", "file", "push", "--uid", str(uid), "--gid", str(gid), "--mode", "%04o" % mode, source, "%s%s" % (target["instance"], staged)], timeout=600)
    in_target(target, ["sync", "--", staged])
    in_target(target, ["mv", "-f", "-T", "--", staged, path])
    in_target(target, ["sync", "--", os.path.dirname(path)])


def copy_out(target, path, destination):
    """Copies a target's file into the updater's private state, for rollback."""
    if os.path.lexists(destination):
        raise ConfigError("refusing to overwrite retained rollback file " + destination)
    if target["driver"] == "systemd":
        shutil.copyfile(path, destination, follow_symlinks=False)
    else:
        run(["incus", "file", "pull", "%s%s" % (target["instance"], path), destination], timeout=600)
    os.chmod(destination, 0o600)
    with open(destination, "rb") as handle:
        os.fsync(handle.fileno())
    sync_directory(os.path.dirname(destination))


# MARK: - Health and Runner control


def relay_health(target):
    """The relay's `/v1/health` answer, read from this host at the configured URL (for an Incus
    relay, an address this host reaches it at)."""
    request = urllib.request.Request(target["health_url"], headers={"User-Agent": USER_AGENT})
    try:
        with HEALTH_OPENER.open(request, timeout=10) as response:
            data = response.read(MAX_HEALTH_BYTES + 1)
    except (urllib.error.URLError, http.client.HTTPException, OSError):
        return None
    if len(data) > MAX_HEALTH_BYTES:
        return None
    try:
        return strict_json(data, "relay health")
    except Failure:
        return None


def runner_control(target, command, check=True):
    """`lorca update …` against the Runner's service, as root inside its host or instance. The
    token file is the operator's, named by path in the environment; its contents never pass
    through here."""
    require_target_path(target, target["binary"], executable=True)
    argv = [target["binary"], "--port", str(target["port"]), "update"] + command
    result = in_target(target, argv, timeout=60, check=False, env={"LORCA_UPDATE_TOKEN_FILE": target["token_file"]})
    if result.returncode != 0:
        if check:
            raise Failure("lorca update %s on %s exited %d" % (command[0], target["name"], result.returncode))
        return None
    try:
        return strict_json(result.stdout.strip() or b"{}", "lorca update " + command[0])
    except Failure:
        if check:
            raise
        return None


def bound_runner_control(target, command, pid):
    """Accepts control only from the configured unit's unchanged, running MainPID."""
    if pid <= 0 or main_pid(target) != pid:
        raise ConfigError("%s's service is stopped or its MainPID changed; operator review is required" % target["name"])
    started = time.monotonic()
    try:
        status = runner_control(target, command)
    except Failure as exc:
        raise ConfigError("%s's update control failed: %s; check drain bootstrap, port and token configuration" % (target["name"], exc))
    if (not isinstance(status, dict) or type(status.get("pid")) is not int or status["pid"] != pid
            or status.get("serving") is False or main_pid(target) != pid):
        raise ConfigError("%s's update endpoint does not belong to its configured service MainPID" % target["name"])
    remaining = status.get("expires_in")
    status["_lease_expires_at"] = started + remaining - 1 if type(remaining) is int and remaining > 0 else started
    # Reserve the stop window only for handoff, not while waiting for busy work to finish.
    status["_lease_deadline"] = status["_lease_expires_at"] - SERVICE_TIMEOUT
    return status


def cancel_drained(target, pid):
    """Never cancel a lease at an endpoint belonging to some other service."""
    try:
        status = bound_runner_control(target, ["status"], pid)
        if status.get("prepared") is True:
            if main_pid(target) != pid:
                raise ConfigError("the service changed before cancellation")
            result = runner_control(target, ["cancel"])
            if (not isinstance(result, dict) or type(result.get("released")) is not bool
                    or main_pid(target) != pid):
                raise ConfigError("cancellation did not answer from the unchanged service")
            status = bound_runner_control(target, ["status"], pid)
            if status.get("prepared") is not False:
                raise ConfigError("cancellation did not release the update lease")
    except (Failure, ConfigError) as exc:
        raise ConfigError("%s's lease could not be cancelled safely (%s); it expires on its own, and needs operator review" % (target["name"], exc))


def wait_for(predicate, timeout, what):
    deadline = time.monotonic() + timeout
    while True:
        if predicate():
            return
        if time.monotonic() >= deadline:
            raise Failure(what)
        time.sleep(POLL_SECONDS)


def answers_as(target, version, required_protocol, old_pid=0):
    """The service's process is not `old_pid` and answers as `version`: the relay's health meets
    the signed protocol requirement, or the Runner's status comes from that process with no lease."""
    pid = main_pid(target)
    if pid == 0 or pid == old_pid:
        return False
    if target["role"] == "relay":
        health = relay_health(target)
        return (isinstance(health, dict) and health.get("ok") is True and health.get("service") == "lorca-relay"
                and type(health.get("protocol")) is int and health["protocol"] >= required_protocol
                and health.get("version") == version)
    status = runner_control(target, ["status"], check=False)
    return (isinstance(status, dict) and status.get("version") == version
            and type(status.get("pid")) is int and status["pid"] == pid
            and status.get("serving") is not False and status.get("prepared") is False and main_pid(target) == pid)


def runs_installed(target, expected_hash):
    """The service's process runs the binary file now installed, and that file is the release's.
    A process started from a file since replaced shows a deleted path and fails this."""
    require_target_path(target, target["binary"], executable=True)
    pid = main_pid(target)
    if pid == 0:
        return False
    exe = in_target(target, ["readlink", "-e", "/proc/%d/exe" % pid], check=False).stdout.decode().strip()
    return exe == target["binary"] and remote_sha256(target, target["binary"]) == expected_hash


def verify_started(target, old_pid, expected_hash, expected_version, required_protocol):
    """A restarted process, from the new file, running the new code and required protocol."""
    wait_for(lambda: answers_as(target, expected_version, required_protocol, old_pid), target["health_timeout"],
             "%s did not come back healthy as %s" % (target["name"], expected_version))
    if not runs_installed(target, expected_hash):
        raise Failure("%s's process does not run the installed %s" % (target["name"], target["binary"]))


def drain_runner(target):
    """Only a genuinely busy, correctly configured Runner is a temporary failure."""
    pid = main_pid(target)
    status = bound_runner_control(target, ["status"], pid)
    if status.get("control") is not True:
        raise ConfigError("%s needs drain-capable bootstrap and service LORCA_UPDATE_TOKEN_FILE configuration" % target["name"])
    try:
        return wait_drained(target, pid)
    except (Retry, ConfigError):
        cancel_drained(target, pid)
        raise


def wait_drained(target, pid):
    ttl = 600
    status = bound_runner_control(target, ["prepare", "--ttl", str(ttl)], pid)
    lease = status.get("lease_id")
    if not isinstance(lease, str) or not lease:
        raise ConfigError("%s did not return an update lease id" % target["name"])
    deadline = time.monotonic() + target["drain_timeout"]
    renewed = time.monotonic()
    while True:
        if status.get("prepared") is not True or status["_lease_expires_at"] <= time.monotonic():
            raise ConfigError("%s lost its update lease" % target["name"])
        if status.get("ready") is True:
            return pid, lease
        if time.monotonic() >= deadline:
            raise Retry("%s stayed busy; its lease is cancelled and the next pass tries again" % target["name"])
        time.sleep(POLL_SECONDS)
        if time.monotonic() - renewed > ttl / 3:
            status = bound_runner_control(target, ["prepare", "--ttl", str(ttl)], pid)
            if status.get("lease_id") != lease:
                raise ConfigError("%s's update lease changed while renewing" % target["name"])
            renewed = time.monotonic()
        else:
            status = bound_runner_control(target, ["status"], pid)


def confirm_drained(target, pid, lease):
    status = bound_runner_control(target, ["prepare", "--ttl", "600"], pid)
    if (status.get("prepared") is not True or status.get("ready") is not True
            or status.get("lease_id") != lease or status["_lease_deadline"] <= time.monotonic()):
        raise ConfigError("%s is no longer ready under the same service-bound lease" % target["name"])


# MARK: - Installing


BACKUP_CHILD = """import os, sqlite3, sys
source, temp = sys.argv[1], sys.argv[2]
os.close(os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600))
try:
    src = sqlite3.connect(source, timeout=60)
    dst = sqlite3.connect(temp)
    src.backup(dst)
    ok = dst.execute("PRAGMA integrity_check").fetchone() == ("ok",)
    dst.close()
    src.close()
except BaseException:
    os.unlink(temp)
    raise
if not ok:
    os.unlink(temp)
    sys.exit(3)
"""


def backup_database(target, version):
    """Copies the relay's SQLite database through SQLite's online backup, never as a live file.
    SQLite runs as the database file's owner, so any -wal/-shm it touches stay that user's; the
    result is copied into the root-only backup directory. A failure here is before any change:
    a retry. Only the newest backups are kept."""
    database = target["database"]
    try:
        owner = os.stat(database)
    except OSError as exc:
        raise ConfigError("the relay database %s cannot be read: %s" % (database, exc))
    if not stat.S_ISREG(owner.st_mode):
        raise ConfigError("the relay database %s is not a regular file" % database)
    directory = target["backup_dir"]
    os.makedirs(directory, mode=0o700, exist_ok=True)
    require_root_dir(directory, "the backup directory")
    os.chmod(directory, 0o700)
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    path = os.path.join(directory, "relay-%s-before-%s.sqlite3" % (stamp, version))
    temp = os.path.join(os.path.dirname(database), ".beans-backup-%s.sqlite3" % stamp)
    try:
        try:
            result = subprocess.run([sys.executable, "-I", "-c", BACKUP_CHILD, database, temp], capture_output=True,
                                    env={"PATH": PATH_ENV, "LC_ALL": "C"}, timeout=1800,
                                    user=owner.st_uid, group=owner.st_gid, extra_groups=[])
        except (subprocess.SubprocessError, OSError) as exc:
            raise Retry("the relay database backup could not run: %s" % exc)
        if result.returncode != 0:
            raise Retry("the relay database backup exited %d; nothing was changed" % result.returncode)
        fd = os.open(temp, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd, "rb") as source:
            st = os.fstat(source.fileno())
            if not stat.S_ISREG(st.st_mode) or st.st_uid != owner.st_uid:
                raise Retry("the relay database backup file is not the database owner's regular file")
            out = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(out, "wb") as handle:
                shutil.copyfileobj(source, handle, 1 << 20)
                handle.flush()
                os.fsync(handle.fileno())
    finally:
        try:
            os.unlink(temp)
        except OSError:
            pass
    try:
        check = sqlite3.connect("file:%s?mode=ro&immutable=1" % urllib.parse.quote(path), uri=True)
        try:
            good = check.execute("PRAGMA integrity_check").fetchone() == ("ok",)
        finally:
            check.close()
    except sqlite3.Error as exc:
        good = False
    if not good:
        os.unlink(path)
        raise Retry("the relay database backup failed its integrity check; nothing was changed")
    sync_directory(directory)
    sync_directory(os.path.dirname(directory))
    backups = sorted(name for name in os.listdir(directory) if name.startswith("relay-") and name.endswith(".sqlite3"))
    for name in backups[:-BACKUPS_KEPT]:
        os.unlink(os.path.join(directory, name))
    sync_directory(directory)
    log("backed up the relay database to " + path)


def files_for(target, staged):
    """(installed path, staged file) pairs a target receives: its binary, and the relay's
    public catalogs when it serves them."""
    files = [(target["binary"], staged["lorca-relay" if target["role"] == "relay" else "lorca"])]
    if target["role"] == "relay" and target["catalog_dir"]:
        files += [(os.path.join(target["catalog_dir"], name), staged[name]) for name in CATALOGS]
    return files


def save_journal(keep, journal):
    write_private(os.path.join(keep, "journal.json"), (json.dumps(journal, indent=2, sort_keys=True) + "\n").encode())


def recovery_journal(target, files, state_dir, version):
    """Retains immutable original bytes; the durable journal authorizes subsequent swaps."""
    parent = os.path.join(state_dir, "previous", target["name"])
    for directory in (os.path.join(state_dir, "previous"), parent):
        os.makedirs(directory, mode=0o700, exist_ok=True)
        require_root_dir(directory, "the recovery directory")
        sync_directory(os.path.dirname(directory))
    keep = os.path.join(parent, version)
    binding = {key: target.get(key) for key in ("driver", "instance", "service", "binary")}
    for name in os.listdir(parent):
        if name.isdigit():
            raise ConfigError("legacy rollback files in %s lack a durable journal; preserve them and recover manually" % parent)
        directory = os.path.join(parent, name)
        require_root_dir(directory, "the retained recovery directory")
        path = os.path.join(directory, "journal.json")
        if not os.path.exists(path):
            raise ConfigError("incomplete original backup in %s; preserve it for operator recovery" % directory)
        require_root_file(path, "the recovery journal", private=True)
        other = strict_json(read_capped(path, MAX_CONFIG_BYTES, "the recovery journal"), "the recovery journal", ConfigError)
        if not isinstance(other, dict):
            raise ConfigError("invalid recovery journal in " + directory)
        if name != version and other.get("phase") == "swapping":
            raise ConfigError("unfinished rollback journal in %s must be recovered before a different release" % directory)
    if os.path.isdir(keep):
        path = os.path.join(keep, "journal.json")
        journal = strict_json(read_capped(path, MAX_CONFIG_BYTES, "the recovery journal"), "the recovery journal", ConfigError)
        if (journal.get("version") != version or journal.get("target") != binding
                or journal.get("phase") not in ("prepared", "swapping", "installed", "rolled_back")
                or not isinstance(journal.get("files"), list) or len(journal["files"]) != len(files)):
            raise ConfigError("the retained recovery journal does not match this target and release: " + keep)
        for index, (entry, (installed, item)) in enumerate(zip(journal["files"], files)):
            if (not isinstance(entry, dict) or entry.get("path") != installed or entry.get("new_sha256") != item["sha256"]
                    or entry.get("copy") not in (None, str(index))):
                raise ConfigError("the retained file inventory does not match this release: " + keep)
            owner = entry.get("owner")
            if (not isinstance(owner, list) or len(owner) != 3 or any(type(value) is not int for value in owner)
                    or owner[0] != 0 or owner[1] < 0 or not 0 <= owner[2] <= 0o7777 or owner[2] & 0o022
                    or (index == 0 and (entry["copy"] is None or not owner[2] & 0o111 or owner[2] & 0o6222))):
                raise ConfigError("unsafe original file metadata in " + keep)
            if entry["copy"] is not None:
                copy = os.path.join(keep, entry["copy"])
                require_root_file(copy, "the retained original", private=True)
                if sha256_file(copy) != entry.get("sha256"):
                    raise ConfigError("the retained original hash differs: " + copy)
        return keep, journal
    os.mkdir(keep, 0o700)
    sync_directory(parent)
    entries = []
    for index, (path, item) in enumerate(files):
        owner = require_target_path(target, path, executable=index == 0, missing=index != 0)
        entry = {"path": path, "copy": None, "owner": list(owner or (0, 0, 0o644)), "new_sha256": item["sha256"]}
        if owner is not None:
            copy = os.path.join(keep, str(index))
            copy_out(target, path, copy)
            entry.update({"copy": str(index), "sha256": sha256_file(copy)})
        entries.append(entry)
    journal = {"version": version, "target": binding, "phase": "prepared", "files": entries}
    save_journal(keep, journal)
    return keep, journal


def service_stopped(target):
    active = in_target(target, ["systemctl", "show", "--property=ActiveState", "--value", target["service"]]).stdout.decode().strip()
    return active in ("inactive", "failed") and main_pid(target) == 0


def install(target, staged, artifact, state_dir, required_protocol, release_version):
    """Resumes mixed swaps using the same originals, including restart-only recovery."""
    files = files_for(target, staged)
    for index, (path, item) in enumerate(files):
        require_target_path(target, path, executable=index == 0, missing=index != 0)
    binary_hash = files[0][1]["sha256"]
    current = all(remote_exists(target, path) and remote_sha256(target, path) == item["sha256"] for path, item in files)
    healthy = current and runs_installed(target, binary_hash) and answers_as(target, artifact["version"], required_protocol)
    retained = os.path.join(state_dir, "previous", target["name"], release_version, "journal.json")
    if current and not healthy and not os.path.exists(retained):
        raise ConfigError("%s has new files without a retained original journal; review restart/recovery manually" % target["name"])
    keep, journal = recovery_journal(target, files, state_dir, release_version)
    if healthy:
        journal["phase"] = "installed"
        save_journal(keep, journal)
        log("%s already runs %s" % (target["name"], artifact["version"]))
        return False
    previous = [(entry["path"], os.path.join(keep, entry["copy"]) if entry["copy"] is not None else None,
                 tuple(entry["owner"])) for entry in journal["files"]]
    old_pid = main_pid(target)
    lease = None
    if target["role"] == "runner":
        if old_pid == 0 and journal["phase"] in ("swapping", "installed") and service_stopped(target):
            # A durable, previously authorized stop, not a serving:false answer at another port.
            log("%s resumes a stopped, journaled installation" % target["name"])
        else:
            old_pid, lease = drain_runner(target)
    elif not current and target["database"]:
        backup_database(target, artifact["version"])
    elif not current:
        log("%s's database backup is external, as the config states" % target["name"])
    if lease is not None:
        try:
            confirm_drained(target, old_pid, lease)
        except ConfigError:
            cancel_drained(target, old_pid)
            raise
    elif target["role"] == "runner" and not service_stopped(target):
        raise ConfigError("%s restarted during stopped-service recovery; operator review is required" % target["name"])
    try:
        systemctl(target, "stop")
        if not service_stopped(target):
            raise Failure("%s did not stop; no files may be swapped" % target["name"])
        # Only a verified stop authorizes stopped-service recovery on the next pass.
        journal["phase"] = "swapping"
        save_journal(keep, journal)
        if not current:
            for index, ((path, item), (_, _, owner)) in enumerate(zip(files, previous)):
                # Never carry forward an executable's ownership or writable/special mode.
                place(target, item["path"], path, (0, 0, 0o555) if index == 0 else owner)
        systemctl(target, "start")
        verify_started(target, old_pid, binary_hash, artifact["version"], required_protocol)
        journal["phase"] = "installed"
        save_journal(keep, journal)
    except Exception as exc:
        log("%s failed (%s); restoring retained originals from %s" % (target["name"], exc, keep))
        if rollback(target, previous):
            journal["phase"] = "rolled_back"
            save_journal(keep, journal)
        error = ConfigError if isinstance(exc, ConfigError) else Failure
        raise error("%s did not take %s: %s" % (target["name"], artifact["version"], exc))
    log("%s now runs %s" % (target["name"], artifact["version"]))
    return True


def rollback(target, previous):
    """Restores retained binary/catalog bytes only; never an incompatible database."""
    try:
        systemctl(target, "stop")
        if not service_stopped(target):
            raise Failure("the service did not stop for rollback")
        for index, (path, copy, owner) in enumerate(previous):
            require_target_path(target, path, executable=index == 0, missing=index != 0)
            if copy is not None:
                place(target, copy, path, (0, 0, 0o555) if index == 0 else owner)
            else:
                in_target(target, ["rm", "-f", "--", path])
                if target["driver"] == "systemd":
                    sync_directory(os.path.dirname(path))
                else:
                    in_target(target, ["sync", "--", os.path.dirname(path)])
        systemctl(target, "start")
        return True
    except Exception as exc:
        log("rolling %s back failed: %s; retained originals need an operator" % (target["name"], exc))
        return False


def prepare_updater(tag, artifacts, assets, work):
    """Validates and stages the required signed updater before any target service changes."""
    artifact = artifacts.get(UPDATER_ASSET)
    if artifact is None:
        raise Failure("the release has no " + UPDATER_ASSET)
    if (artifact["component"] != "updater" or artifact["platform"] != "linux"
            or artifact["version"] != tag[len("beans-v"):] or artifact["size"] > MAX_UPDATER_BYTES):
        raise Failure(UPDATER_ASSET + " is not the server updater for this release")
    if UPDATER_ASSET not in assets:
        raise Failure("the release has not published " + UPDATER_ASSET)
    staged = os.path.join(work, UPDATER_ASSET)
    download(tag, artifact, staged, MAX_UPDATER_BYTES)
    with open(staged, "rb") as handle:
        source = handle.read()
    if not source.startswith(b"#!/usr/bin/env python3\n"):
        raise Failure(UPDATER_ASSET + " is not a Python script")
    try:
        compile(source, UPDATER_ASSET, "exec")
    except (SyntaxError, ValueError) as exc:
        raise Failure("%s does not compile: %s" % (UPDATER_ASSET, exc))
    return artifact, source


def replace_updater(tag, artifact, source, settings):
    """Installs the preflighted signed updater only after every target succeeds."""
    path = settings["updater_path"]
    require_target_path({"driver": "systemd"}, path, executable=True, missing=True)
    if os.path.isfile(path) and sha256_file(path) == artifact["sha256"]:
        return
    write_private(path, source, 0o555)
    log("installed the updater from " + tag)


# MARK: - One pass


def load_state(state_dir):
    path = os.path.join(state_dir, "state.json")
    if not os.path.exists(path):
        return {}
    require_root_file(path, "the updater state", private=True)
    state = strict_json(read_capped(path, MAX_CONFIG_BYTES, "the updater state"), "the updater state", ConfigError)
    return state if isinstance(state, dict) else {}


def save_state(state_dir, state):
    write_private(os.path.join(state_dir, "state.json"), (json.dumps(state, indent=2, sort_keys=True) + "\n").encode())


def all_running(targets, state):
    """Every target runs its recorded binary and answers as the installed release with its
    signed protocol. Missing hashes or protocol require a full signed-manifest pass."""
    hashes = state.get("hashes")
    protocol = state.get("protocol")
    if not isinstance(hashes, dict) or type(protocol) is not int or protocol < MIN_PROTOCOL:
        return False
    try:
        return all(isinstance(hashes.get(t["name"]), str) and runs_installed(t, hashes[t["name"]])
                   and answers_as(t, state.get("server_version"), protocol) for t in targets)
    except Exception:
        return False


def one_pass(settings):
    """Installs the latest release. Each target that verifies is recorded at once, so a pass cut
    short resumes with the next target in order, never moving a Runner before the relay."""
    state_dir = settings["state_dir"]
    pem = load_public_key(settings["public_key"])
    state = load_state(state_dir)
    if state.get("operator_hold"):
        raise ConfigError("manual operator hold: %s; repair the configuration/recovery and clear operator_hold in %s" %
                          (state["operator_hold"], os.path.join(state_dir, "state.json")))
    tag, assets = latest_release()
    version = tag[len("beans-v"):]
    if state.get("failed") == version:
        log("%s failed before; waiting for a newer release or an operator (edit %s to clear)" % (version, os.path.join(state_dir, "state.json")))
        return
    installed = state.get("version")
    completed_versions = [installed, state.get("version_floor")]
    if isinstance(state.get("targets"), list) and state["targets"]:
        completed_versions.append(state.get("rolling"))
    version_floor = max((value for value in completed_versions if isinstance(value, str) and SEMVER.match(value)),
                        key=version_tuple, default=None)
    if version_floor is not None and version_tuple(version) < version_tuple(version_floor):
        raise Failure("refusing to downgrade from %s to %s" % (version_floor, version))
    names = [target["name"] for target in settings["targets"]]
    done = state.get("targets") if state.get("rolling") == version and isinstance(state.get("targets"), list) else []
    if installed == version and state.get("rolling") is None and state.get("targets") == names and all_running(settings["targets"], state):
        return
    # The manifest is published last: a release without it is not ready yet.
    if MANIFEST not in assets or SIGNATURE not in assets:
        log("%s is not ready yet" % tag)
        return
    work = tempfile.mkdtemp(prefix="work-", dir=state_dir)
    try:
        manifest = fetch(release_url(tag, MANIFEST), MAX_MANIFEST_BYTES)
        signature = fetch(release_url(tag, SIGNATURE), MAX_SIGNATURE_BYTES).decode("ascii", "replace")
        verify_signature(pem, manifest, signature, work)
        artifacts = parse_manifest(manifest, tag)
        release = json.loads(manifest)
        required_protocol = release["protocol"]
        artifact = server_artifact(artifacts, settings["platform"])
        archive = os.path.join(work, artifact["name"])
        download(tag, artifact, archive, MAX_ARCHIVE_BYTES)
        staged_dir = os.path.join(work, "staged")
        os.mkdir(staged_dir, 0o700)
        staged = extract(archive, settings["platform"], staged_dir)
        updater_artifact, updater_source = prepare_updater(tag, artifacts, assets, work)
        require_target_path({"driver": "systemd"}, settings["updater_path"], executable=True, missing=True)
        state.update({"rolling": version, "targets": [name for name in names if name in done], "protocol": required_protocol})
        # Resetting progress for a newer attempt must not forget an earlier partial rollout.
        if version_floor is not None:
            state["version_floor"] = version_floor
        if not isinstance(state.get("hashes"), dict):
            state["hashes"] = {}
        save_state(state_dir, state)
        # Relay first; persist intent before install so an interrupted partial rollout is visible.
        for target in settings["targets"]:
            state["pending"] = target["name"]
            state["version_floor"] = version
            save_state(state_dir, state)
            install(target, staged, artifact, state_dir, required_protocol, version)
            state["hashes"][target["name"]] = staged["lorca-relay" if target["role"] == "relay" else "lorca"]["sha256"]
            if target["name"] not in state["targets"]:
                state["targets"].append(target["name"])
            state.pop("pending", None)
            save_state(state_dir, state)
        replace_updater(tag, updater_artifact, updater_source, settings)
        state.pop("failed", None)
        state.pop("failed_reason", None)
        state.pop("rolling", None)
        state.update({"version": version, "revision": release["revision"], "server_version": artifact["version"], "installed_at": int(time.time())})
        save_state(state_dir, state)
        log("%s installed" % tag)
    except Failure as exc:
        # Includes signature, manifest, archive/hash/extraction and updater preflight failures.
        state["failed"] = version
        state["failed_reason"] = str(exc)
        save_state(state_dir, state)
        raise
    except ConfigError as exc:
        # Operator faults are not release failures and must not recur after a partial rollout.
        state["operator_hold"] = str(exc)
        save_state(state_dir, state)
        raise
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description="Install the latest signed Beans server release.")
    parser.add_argument("--config", required=True, help="the root-owned JSON config")
    parser.add_argument("--once", action="store_true", help="run one pass and exit (the default; the timer runs it every 15 minutes)")
    arguments = parser.parse_args()
    os.umask(0o077)
    try:
        if os.geteuid() != 0:
            raise ConfigError("run the updater as root")
        settings = load_config(absolute(arguments.config, "--config"))
        os.makedirs(settings["state_dir"], mode=0o700, exist_ok=True)
        require_root_dir(settings["state_dir"], "the state directory")
        os.chmod(settings["state_dir"], 0o700)
        lock = os.open(settings["lock_file"], os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            log("another pass is running")
            return 0
        one_pass(settings)
        return 0
    except ConfigError as exc:
        log("configuration: %s" % exc)
        return 78
    except Retry as exc:
        log("will try again: %s" % exc)
        return EX_TEMPFAIL
    except Failure as exc:
        log("stopped: %s" % exc)
        return 1


if __name__ == "__main__":
    sys.exit(main())
