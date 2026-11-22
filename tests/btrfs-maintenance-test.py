"""Policy, error propagation and ioctl ABI tests; never operate on host Btrfs."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("maintenance", sys.argv.pop(1))
m = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = m
spec.loader.exec_module(m)
G = m.Group


class Fake:
    mount = "/test mount"

    def __init__(self, count=30, free=10000, effect="none"):
        self.items = [G(i * 1000, 1000, 100 + i, 1) for i in range(1, count + 1)]
        self.free, self.target, self.effect = free, 8000, effect
        self.calls = []

    def sync(self):
        pass

    def groups(self):
        return self.items.copy()

    def space(self):
        return self.free, self.target

    def balance(self, filters):
        self.calls.append(filters)
        if not filters.startswith("-dvrange="):
            return
        if self.effect == "error":
            raise subprocess.CalledProcessError(1, "balance")
        start = int(filters.split("=")[1].split("..")[0])
        group = next(g for g in self.items if g.start == start)
        if self.effect in ("reclaim", "relocate", "consume"):
            self.items.remove(group)
            if self.effect == "reclaim":
                self.free += 1000
            else:
                # The source is removed, but a new destination group consumes
                # the same allocation. This MUST NOT count as reclamation.
                self.items.append(G(max([g.start for g in self.items] + [start]) + 1000,
                                    group.length, group.used, 1))
                if self.effect == "consume":
                    self.free -= 100

    def attempts(self):
        return [x for x in self.calls if x.startswith("-dvrange=")]


class Policy(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / "state.json"
        self.output = io.StringIO()
        self.redirect = contextlib.redirect_stdout(self.output)
        self.redirect.__enter__()
        self.addCleanup(self.redirect.__exit__, None, None, None)

    def run_pass(self, backend, mode="clean"):
        m.maintain(backend, mode, self.path)

    def test_bounded_resume_then_exhausted_no_repeated_relocations(self):
        fake = Fake()
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 24)
        self.assertEqual(len(m.load_state(self.path)["pending"]), 6)
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 30)
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 30)
        self.assertIn("unchanged data layout", self.output.getvalue())
        self.assertIn("delta=0", self.output.getvalue())
        self.assertIn("no allocation progress", self.output.getvalue())

    def test_relocating_source_without_allocation_gain_terminates(self):
        fake = Fake(3, effect="relocate")
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 3)
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 3)
        self.assertIn("positive allocation deltas=0", self.output.getvalue())

    def test_clean_keeps_reclaiming_above_reserve_and_rechecks_dense_layout(self):
        fake = Fake(30, effect="reclaim")
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 24)
        self.assertEqual(fake.free, 34000)
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 30)
        self.assertEqual(fake.free, 40000)
        self.assertIn("no data groups below 90%", self.output.getvalue())

    def test_switch_one_group_and_resume_without_gc(self):
        fake = Fake(3)
        self.run_pass(fake, "switch")
        self.assertEqual(len(fake.attempts()), 1)
        self.run_pass(fake, "switch")
        self.assertEqual(len(fake.attempts()), 2)
        self.assertNotEqual(fake.attempts()[0], fake.attempts()[1])

    def test_switch_low_reserve_uses_bounded_recovery_until_target(self):
        fake = Fake(30, free=1000, effect="reclaim")
        self.run_pass(fake, "switch")
        self.assertEqual(len(fake.attempts()), 7)
        self.assertEqual(fake.free, 8000)

    def test_low_reserve_not_success_even_when_balance_exits_zero(self):
        fake = Fake(30, free=1000)
        with self.assertRaisesRegex(ValueError, "reserve remains low"):
            self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 24)
        self.assertEqual(len(m.load_state(self.path)["pending"]), 6)

    def test_new_deletions_invalidate_completed_or_partial_sweep(self):
        fake = Fake(3)
        self.run_pass(fake)
        fake.items[0] = G(1000, 1000, 10, 1)
        self.run_pass(fake, "switch")
        self.assertEqual(len(fake.attempts()), 4)
        self.assertIn("1000..2000", fake.attempts()[-1])
        # A change during a partial sweep must not starve the remaining tail.
        fake.items[0] = G(1000, 1000, 1, 1)
        self.run_pass(fake, "switch")
        self.assertIn("2000..3000", fake.attempts()[-1])
        self.run_pass(fake, "switch")
        self.assertIn("3000..4000", fake.attempts()[-1])
        # Once the tail is visited, changed earlier candidates get a new sweep.
        self.run_pass(fake, "switch")
        self.assertIn("1000..2000", fake.attempts()[-1])

    def test_writes_between_bounded_runs_do_not_reset_the_queue(self):
        fake = Fake(30)
        self.run_pass(fake)
        fake.items[0] = G(1000, 1000, 102, 1)
        self.run_pass(fake)
        self.assertIn("25000..26000", fake.attempts()[24])

    def test_restored_workspace_allows_recheck_without_changed_data_layout(self):
        fake = Fake(2)
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 2)
        fake.free += 1000
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 4)

    def test_failure_does_not_consume_queue(self):
        fake = Fake(3, effect="error")
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_pass(fake)
        self.assertEqual(m.load_state(self.path)["pending"], [1000, 2000, 3000])
        fake.effect = "none"
        self.run_pass(fake, "switch")
        self.assertEqual(m.load_state(self.path)["pending"], [2000, 3000])

    def test_failed_balance_reports_actual_final_allocation(self):
        fake = Fake(1)
        def failing_balance(filters):
            if filters.startswith("-dvrange="):
                fake.free += 100
                raise subprocess.CalledProcessError(1, "balance")
        with patch.object(fake, "balance", side_effect=failing_balance):
            with self.assertRaises(subprocess.CalledProcessError):
                self.run_pass(fake)
        self.assertIn("after=10100, delta=100", self.output.getvalue())

    def test_unavailable_final_sample_is_not_reported_as_zero_delta(self):
        fake = Fake(1, effect="error")
        with patch.object(fake, "space", side_effect=[(10000, 8000), (10000, 8000), OSError("lost")]):
            with self.assertRaises(subprocess.CalledProcessError):
                self.run_pass(fake)
        self.assertIn("after=unknown, delta=unknown", self.output.getvalue())

    def test_decreasing_workspace_stops_immediately(self):
        fake = Fake(30, effect="consume")
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 1)
        self.assertIn("workspace decreased", self.output.getvalue())
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 2)
        self.assertNotEqual(fake.attempts()[0], fake.attempts()[1])

    def test_single_counterproductive_group_is_not_repeated(self):
        fake = Fake(1, effect="consume")
        self.run_pass(fake)
        self.run_pass(fake)
        self.assertEqual(len(fake.attempts()), 1)

    def test_dense_mixed_and_metadata_are_not_relocated(self):
        fake = Fake(0)
        fake.items = [G(1, 1000, 900, 1), G(1001, 1000, 10, 5), G(2001, 1000, 10, 4)]
        self.run_pass(fake)
        self.assertEqual(fake.attempts(), [])
        self.assertEqual(fake.calls, ["-dusage=0", "-musage=0"])

    def test_unreadable_inventory_is_failure(self):
        fake = Fake(1)
        with patch.object(fake, "groups", side_effect=OSError("ioctl denied")):
            with self.assertRaises(OSError):
                self.run_pass(fake)


class Inspection(unittest.TestCase):
    def test_bad_state_fails_instead_of_claiming_completion(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "state.json"
            for invalid in ("{", "[]", '{"version": 100}'):
                path.write_text(invalid)
                with self.assertRaises(ValueError):
                    m.load_state(path)

    def test_metadata_pressure_counts_all_committed_bytes_and_small_fs_cap(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "uuid/allocation/metadata"
            root.mkdir(parents=True)
            values = dict(total_bytes=1000000000, bytes_used=700000000,
                          bytes_reserved=100000000, bytes_pinned=100000000,
                          bytes_may_use=100000000, bytes_readonly=0)
            for key, value in values.items():
                (root / key).write_text(str(value))
            args = SimpleNamespace(sysfs=directory, reserve=8 * 1024**3)
            backend = m.Backend(args, "/space mount", "uuid")
            with patch.object(backend, "command", return_value="Device size: 100000000000\nDevice unallocated: 123\n"):
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    free, target = backend.space()
                self.assertEqual((free, target), (123, 16 * 1024**3))
                self.assertIn("pressure=True", output.getvalue())
            with patch.object(backend, "command", return_value="Device size: 4000\nDevice unallocated: 123\n"):
                self.assertEqual(backend.space(), (123, 1000))
            (root / "bytes_pinned").unlink()
            with patch.object(backend, "command", return_value="Device size: 4000\nDevice unallocated: 123\n"):
                with self.assertRaises(FileNotFoundError):
                    backend.space()
            (root / "bytes_pinned").write_text("100000000")
            (root / "total_bytes").unlink()
            with patch.object(backend, "command", return_value="Device size: 4000\nDevice unallocated: 123\n"):
                self.assertEqual(backend.space(), (123, 1000))
            with patch.object(backend, "command", return_value="unknown"):
                with self.assertRaisesRegex(ValueError, "Cannot parse"):
                    backend.space()

    def test_mount_dedupe_readonly_and_space_paths(self):
        entries = [dict(target=target, fstype=fs, uuid=uuid, options=opts) for target, fs, uuid, opts in [
            ("/", "btrfs", "root", "rw"), ("/alias", "btrfs", "root", "rw"),
            ("/space mount", "btrfs", "home", "rw"), ("/ro", "btrfs", "ro", "ro"),
            ("/boot", "vfat", "boot", "rw")]]
        with patch.object(m.subprocess, "run", return_value=SimpleNamespace(stdout=json.dumps({"filesystems": entries}))):
            self.assertEqual(list(m.mounts("findmnt")), [("/", "root"), ("/space mount", "home")])

    def test_ioctl_fixed_key_pagination_and_both_block_group_trees(self):
        for bgt in (False, True):
            calls = []
            def ioctl(fd, operation, buf, mutate):
                if operation == m.GET_FEATURES:
                    struct.pack_into("=3Q", buf, 0, 0, 8 if bgt else 0, 0)
                    return
                self.assertEqual(operation, 0xc0709411)
                fields = m.KEY.unpack_from(buf)
                tree, obj, maxobj, offset, maximum, _, _, kind = fields[:8]
                self.assertEqual(obj, maxobj)
                calls.append((tree, obj, kind, offset, maximum))
                records = []
                if tree == 3 and offset == 0:
                    self.assertEqual((obj, kind), (256, 228))
                    chunk = bytearray(48)
                    struct.pack_into("<Q", chunk, 0, 1000)
                    struct.pack_into("<Q", chunk, 24, 1)
                    records = [(1000, chunk)]
                elif tree == (11 if bgt else 2) and offset == 1000:
                    self.assertEqual((obj, kind, maximum), (1000, 192, 1000))
                    records = [(1000, struct.pack("<3Q", 123, 256, 1))]
                struct.pack_into("=I", buf, 64, len(records))
                pos = 112
                for item_offset, payload in records:
                    m.HEADER.pack_into(buf, pos, 1, obj, item_offset, kind, len(payload))
                    pos += 32
                    buf[pos:pos + len(payload)] = payload
                    pos += len(payload)
            with patch.object(m.os, "open", return_value=9), patch.object(m.os, "close"), patch.object(m.fcntl, "ioctl", side_effect=ioctl):
                self.assertEqual(m.block_groups("/fake"), [G(1000, 1000, 123, 1)])
                self.assertIn((3, 256, 228, 1001, m.U64), calls)

    def test_missing_block_group_is_failure_even_with_other_valid_groups(self):
        chunk = bytearray(48)
        struct.pack_into("<Q", chunk, 0, 1000)
        struct.pack_into("<Q", chunk, 24, 1)
        def fake_search(fd, tree, obj, kind, *args):
            if tree == 3:
                return iter([(1000, chunk), (2000, chunk)])
            return iter([(1000, struct.pack("<3Q", 100, 256, 1))] if obj == 1000 else [])
        with patch.object(m.os, "open", return_value=9), patch.object(m.os, "close"), \
             patch.object(m.fcntl, "ioctl"), patch.object(m, "search", side_effect=fake_search):
            with self.assertRaisesRegex(ValueError, "Missing/ambiguous"):
                m.block_groups("/fake")


class Main(unittest.TestCase):
    def invoke(self, mode, directory):
        arguments = ["maintenance", "--btrfs", "fake", "--findmnt", "fake",
                     "--reserve", "8", "--sysfs", directory,
                     "--state-dir", directory, mode]
        with patch.object(sys, "argv", arguments):
            return m.main()

    def test_other_filesystems_attempted_after_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            failed, healthy = Fake(1, effect="error"), Fake(0)
            with patch.object(m, "mounts", return_value=[("/a", "a"), ("/b", "b")]), \
                 patch.object(m, "Backend", side_effect=[failed, healthy]), \
                 contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(self.invoke("clean", directory), 1)
            self.assertEqual(healthy.calls, ["-dusage=0", "-musage=0"])

    def test_report_is_readonly_and_does_not_fail_for_low_reserve(self):
        with tempfile.TemporaryDirectory() as directory:
            fake = Fake(1, free=1)
            with patch.object(m, "mounts", return_value=[("/a", "a")]), \
                 patch.object(m, "Backend", return_value=fake):
                self.assertEqual(self.invoke("report", directory), 0)
            self.assertEqual(fake.calls, [])

    def test_no_btrfs_is_success_and_mount_enumeration_failure_is_fatal(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(m, "mounts", return_value=[]):
                self.assertEqual(self.invoke("switch", directory), 0)
            with patch.object(m, "mounts", side_effect=subprocess.CalledProcessError(1, "findmnt")):
                with self.assertRaises(subprocess.CalledProcessError):
                    self.invoke("switch", directory)


if __name__ == "__main__":
    unittest.main()
