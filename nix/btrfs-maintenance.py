"""Bounded Btrfs maintenance. Only the explicit clean wrapper runs Nix GC."""

import argparse
from dataclasses import dataclass
from fractions import Fraction
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import sys


U64 = (1 << 64) - 1
SEARCH = 0xC0709411  # BTRFS_IOC_TREE_SEARCH_V2, fixed header size 112
GET_FEATURES = 0x80189439
KEY = struct.Struct("=7Q4I4Q")
HEADER = struct.Struct("=3Q2I")


@dataclass(frozen=True)
class Group:
    start: int
    length: int
    used: int
    flags: int

    @property
    def data(self):
        # Never relocate mixed data/metadata or remapped groups.
        return self.flags & 7 == 1 and not self.flags & ((1 << 11) | (1 << 12))

    @property
    def candidate(self):
        return self.data and 0 < self.used < self.length * 90 // 100


def search(fd, tree, objectid, kind, offset=0, maximum=U64):
    """Search a fixed objectid/type interval, never the entire extent tree."""
    while offset <= maximum:
        capacity = 65536
        while True:
            buf = bytearray(112 + capacity)
            KEY.pack_into(buf, 0, tree, objectid, objectid, offset, maximum,
                          0, U64, kind, kind, 128, 0, 0, 0, 0, 0)
            struct.pack_into("=Q", buf, 104, capacity)
            try:
                fcntl.ioctl(fd, SEARCH, buf, True)
                break
            except OSError as error:
                needed = struct.unpack_from("=Q", buf, 104)[0]
                if error.errno != 75 or not capacity < needed <= 16 * 1024 * 1024:
                    raise
                capacity = needed
        count = struct.unpack_from("=I", buf, 64)[0]
        if not count:
            return
        pos = 112
        last = offset
        for _ in range(count):
            if pos + HEADER.size > len(buf):
                raise ValueError("Truncated Btrfs search header")
            _, obj, last, item_type, length = HEADER.unpack_from(buf, pos)
            pos += HEADER.size
            if (obj != objectid or item_type != kind or last < offset
                    or last > maximum or pos + length > len(buf)):
                raise ValueError("Invalid Btrfs search item")
            yield last, bytes(buf[pos:pos + length])
            pos += length
        if last == U64:
            return
        offset = last + 1


def block_groups(mount):
    fd = os.open(mount, os.O_RDONLY | os.O_DIRECTORY)
    try:
        features = bytearray(24)
        fcntl.ioctl(fd, GET_FEATURES, features, True)
        _, compat_ro, incompat = struct.unpack("=3Q", features)
        if incompat & (1 << 17):
            raise ValueError("Remap-tree maintenance is not supported")
        tree = 11 if compat_ro & (1 << 3) else 2
        groups = []
        # The chunk tree is small even when the extent tree is very large.
        for start, chunk in search(fd, 3, 256, 228):
            if len(chunk) < 48:
                raise ValueError("Truncated Btrfs chunk item")
            length = struct.unpack_from("<Q", chunk)[0]
            chunk_flags = struct.unpack_from("<Q", chunk, 24)[0]
            if not 0 < length <= U64 - start:
                raise ValueError("Invalid Btrfs chunk range")
            items = list(search(fd, tree, start, 192, length, length))
            if len(items) != 1:
                raise ValueError(f"Missing/ambiguous Btrfs block group for chunk {start}; retry inspection")
            bg_length, item = items[0]
            if bg_length != length or len(item) < 24:
                raise ValueError("Invalid Btrfs block group item")
            used, _, flags = struct.unpack_from("<3Q", item)
            if used > length or flags != chunk_flags:
                raise ValueError("Invalid Btrfs block group usage/flags")
            groups.append(Group(start, length, used, flags))
        if not groups:
            raise ValueError("No Btrfs block groups found; cannot verify maintenance")
        return groups
    finally:
        os.close(fd)


def topology(groups):
    # Ignore metadata transaction churn. Data changes invalidate a completed
    # sweep; relocation-induced changes are recorded only at sweep completion.
    items = sorted((g.start, g.length, g.used, g.flags) for g in groups if g.data)
    return hashlib.sha256(json.dumps(items).encode()).hexdigest()


class Backend:
    def __init__(self, args, mount, uuid):
        self.args, self.mount, self.uuid = args, mount, uuid

    def command(self, *arguments, capture=False):
        command = [self.args.btrfs, *arguments, self.mount]
        if not capture:
            print("$ btrfs " + " ".join(command[1:]), flush=True)
        return subprocess.run(command, check=True, text=True,
                              stdout=subprocess.PIPE if capture else None,
                              env={**os.environ, "LC_ALL": "C"}).stdout

    def sync(self):
        self.command("filesystem", "sync", capture=True)

    def groups(self):
        return block_groups(self.mount)

    def space(self):
        output = self.command("filesystem", "usage", "--raw", capture=True)
        size = re.search(r"^\s*Device size:\s+(\d+)\s*$", output, re.M)
        free = re.search(r"^\s*Device unallocated:\s+(\d+)\s*$", output, re.M)
        if not size or not free or not 0 <= int(free[1]) <= int(size[1]) or int(size[1]) == 0:
            raise ValueError("Cannot parse Btrfs allocation")
        counters = {}
        for name in ("total_bytes", "bytes_used", "bytes_reserved", "bytes_pinned",
                     "bytes_may_use", "bytes_readonly"):
            value = (Path(self.args.sysfs) / self.uuid / "allocation/metadata" / name).read_text().strip()
            if not value.isascii() or not value.isdecimal():
                raise ValueError(f"Invalid metadata counter {name}")
            counters[name] = int(value)
        total, used = counters["total_bytes"], counters["bytes_used"]
        if not 0 <= used <= total or total == 0:
            raise ValueError("Invalid Btrfs metadata capacity")
        headroom = total - sum(v for k, v in counters.items() if k != "total_bytes")
        pressure = used * 100 >= total * 85 or headroom < min(total // 8, 268435456)
        target = max(1, min(int(size[1]) // 4, self.args.reserve * (2 if pressure else 1)))
        print(f"Btrfs {self.mount}: {int(free[1])} bytes unallocated; reserve target {target} bytes; "
              f"metadata used={used}/{total}, headroom={headroom}, pressure={pressure}.", flush=True)
        return int(free[1]), target

    def balance(self, filters):
        try:
            self.command("balance", "start", filters)
        except subprocess.CalledProcessError:
            print(f"Btrfs {self.mount}: balance failed; file-level free space does not guarantee "
                  "allocation workspace. Inspect btrfs filesystem usage --raw and journalctl -k -b. "
                  "Restore workspace before retrying ENOSPC; increasing limits may not help.", file=sys.stderr)
            raise


def load_state(path):
    try:
        state = json.loads(path.read_text())
        if (not isinstance(state, dict) or state.get("version") != 1 or not isinstance(state.get("pending"), list)
                or any(type(x) is not int or not 0 <= x <= U64 for x in state["pending"])
                or type(state.get("sweep_delta")) is not int
                or type(state.get("dirty")) is not bool
                or not isinstance(state.get("finished"), str)
                or not isinstance(state.get("expected"), str)
                or type(state.get("expected_free")) is not int):
            raise ValueError("Invalid maintenance state")
        return state
    except FileNotFoundError:
        return {"version": 1, "pending": [], "sweep_delta": 0, "finished": "",
                "expected": "", "expected_free": -1, "dirty": False}


def save_state(path, state):
    temporary = path.with_suffix(".tmp")
    with temporary.open("w") as stream:
        json.dump(state, stream)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def maintain(backend, mode, path, clean_limit=24):
    state = load_state(path)
    backend.sync()
    before, target = backend.space()
    after = before
    attempts = reclaimed = 0
    status = "failed"
    try:
        # Empty chunks need no relocation workspace; no used metadata balance.
        backend.balance("-dusage=0")
        backend.balance("-musage=0")
        backend.sync()
        after, target = backend.space()
        groups = backend.groups()
        if state["expected"] != topology(groups) or state["expected_free"] != after:
            # Writes/GC between invocations can make previously unproductive
            # groups reclaimable. Do not let the old completion hide them.
            if state["pending"]:
                # GC and ordinary writes must not starve the unvisited tail
                # by resetting a bounded sweep to its first groups each time.
                state["dirty"] = True
            else:
                state.update(sweep_delta=0, finished="", dirty=False)
        state["expected"] = topology(groups)
        state["expected_free"] = after
        budget = clean_limit if mode == "clean" else (24 if after < target else 1)
        while True:
            signature = topology(groups)
            if not state["pending"]:
                if state["finished"] == signature:
                    status = "nothing further to reclaim under the safe policy (unchanged data layout)"
                    break
                state["pending"] = [g.start for g in sorted(
                    (g for g in groups if g.candidate),
                    key=lambda g: (Fraction(g.used, g.length), g.start))]
                state["sweep_delta"] = 0
                state["dirty"] = False
                state["finished"] = ""
                if not state["pending"]:
                    state["finished"] = signature
                    status = "nothing to reclaim under the safe policy (no data groups below 90% usage)"
                    break
            if attempts >= budget or (mode == "switch" and attempts > 0 and after >= target):
                status = f"bounded pass complete; {len(state['pending'])} queued groups remain; run clean to continue"
                break
            start = state["pending"][0]
            group = next((g for g in groups if g.start == start and g.candidate), None)
            if group is None:
                state["pending"].pop(0)
            else:
                # Persist the unconsumed queue first: a crash/error retries the
                # same range rather than claiming it was successfully checked.
                save_state(path, state)
                previous = after
                attempts += 1
                backend.balance(f"-dvrange={start}..{start + group.length},usage=90,limit=1")
                backend.sync()
                after, target = backend.space()
                groups = backend.groups()
                state["expected"] = topology(groups)
                state["expected_free"] = after
                state["sweep_delta"] += after - previous
                remaining = next((g for g in groups if g.start == start), None)
                if after > previous:
                    reclaimed += after - previous
                    print(f"Btrfs {backend.mount}: allocation progress +{after - previous} bytes.", flush=True)
                else:
                    print(f"Btrfs {backend.mount}: no allocation progress for group {start}; "
                          f"source bytes {group.used} -> {remaining.used if remaining else 0}; "
                          f"unallocated delta {after - previous}.", flush=True)
                state["pending"].pop(0)
                if after < previous:
                    # A new destination chunk or concurrent writes consumed
                    # physical workspace. Do not churn through more groups.
                    if not state["pending"] and state["sweep_delta"] <= 0 and not state["dirty"]:
                        state["finished"] = topology(groups)
                    status = ("stopped: allocation workspace decreased; "
                              f"{len(state['pending'])} queued groups remain; run clean to continue")
                    break
            if not state["pending"]:
                if state["sweep_delta"] > 0 or state["dirty"]:
                    # Revisit the new, denser layout only after a sweep that
                    # actually freed allocation or external changes made old
                    # observations stale, never just because bytes relocated.
                    state["finished"] = ""
                    state["dirty"] = False
                else:
                    state["finished"] = topology(groups)
            save_state(path, state)
        save_state(path, state)
        if after < target:
            status = f"allocation reserve remains low ({after} < {target}); {status}"
            raise ValueError(status)
    except (OSError, ValueError, subprocess.CalledProcessError):
        # A failing balance may still have partially changed allocation. Never
        # report the last successful sample as the final state on that path.
        try:
            backend.sync()
            after, _ = backend.space()
        except (OSError, ValueError, subprocess.CalledProcessError):
            after = None
        raise
    finally:
        delta = after - before if after is not None else "unknown"
        print(f"Btrfs {backend.mount}: before={before}, after={after if after is not None else 'unknown'}, delta={delta} "
              f"unallocated bytes; data attempts={attempts}, positive allocation deltas={reclaimed}; {status}.", flush=True)


def mounts(findmnt):
    output = subprocess.run([findmnt, "--json", "--list", "-o", "TARGET,FSTYPE,UUID,OPTIONS"],
                            check=True, text=True, stdout=subprocess.PIPE).stdout
    seen = set()
    for entry in json.loads(output)["filesystems"]:
        if entry["fstype"] != "btrfs" or "ro" in entry["options"].split(","):
            continue
        uuid = entry["uuid"]
        if not uuid or not re.fullmatch(r"[a-zA-Z0-9-]+", uuid):
            raise ValueError("Missing/invalid Btrfs UUID")
        if uuid not in seen:
            seen.add(uuid)
            yield entry["target"], uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--btrfs", required=True)
    parser.add_argument("--findmnt", required=True)
    parser.add_argument("--reserve", type=int, required=True)
    parser.add_argument("--sysfs", required=True)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--clean-limit", type=int, choices=range(1, 25), default=24)
    parser.add_argument("mode", choices=["switch", "clean", "report"])
    args = parser.parse_args()
    print(f"Btrfs maintenance ({args.mode})", flush=True)
    args.state_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    failed = False
    # Direct invocations are serialized too, independently of yonix-system.
    with (args.state_dir / "lock").open("w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        for mount, uuid in list(mounts(args.findmnt)):
            try:
                backend = Backend(args, mount, uuid)
                if args.mode == "report":
                    backend.space()
                else:
                    maintain(backend, args.mode, args.state_dir / f"{uuid}.json", args.clean_limit)
            except (OSError, ValueError, subprocess.CalledProcessError) as error:
                print(f"WARNING: Btrfs maintenance failed for {mount}: {error}", file=sys.stderr)
                failed = True
    return int(failed)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f"Btrfs maintenance failed: {error}", file=sys.stderr)
        sys.exit(1)
