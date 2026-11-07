"""Finite real-process regression for the mounted skills driver's POSIX teardown."""
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("skills_driver", Path(__file__).with_name("check-desktop-skills-consumer.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)

CHILD = """import json,os,pathlib,signal,sys
if sys.argv[2]=='ignore': signal.signal(signal.SIGTERM,signal.SIG_IGN)
target=pathlib.Path(sys.argv[1]);temp=target.with_suffix('.tmp')
temp.write_text(json.dumps({'pid':os.getpid(),'ppid':os.getppid(),'pgid':os.getpgrp(),'session':os.getsid(0),'cmdline':pathlib.Path('/proc/self/cmdline').read_bytes().decode().split('\\0')}));temp.replace(target)
while True: signal.pause()
"""
LEADER = """import json,os,pathlib,subprocess,sys,time,signal
pathlib.Path(sys.argv[2]+'.leader').write_text(json.dumps({'pid':os.getpid(),'ppid':os.getppid(),'pgid':os.getpgrp(),'session':os.getsid(0),'cmdline':pathlib.Path('/proc/self/cmdline').read_bytes().decode().split('\\0')}))
subprocess.Popen([sys.executable,'-I','-c',sys.argv[1],sys.argv[2],sys.argv[3]])
deadline=time.monotonic()+3
while not pathlib.Path(sys.argv[2]).exists():
 if time.monotonic()>deadline: raise RuntimeError('child did not report ready')
 time.sleep(.01)
if sys.argv[4]=='exit': sys.exit(7)
while True: signal.pause()
"""


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux /proc attribution and test-only subreaper")
class Teardown(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.libc = ctypes.CDLL(None, use_errno=True)
        cls.previous = ctypes.c_int()
        assert cls.libc.prctl(37, ctypes.byref(cls.previous), 0, 0, 0) == 0
        assert cls.libc.prctl(36, 1, 0, 0, 0) == 0

    @classmethod
    def tearDownClass(cls):
        assert cls.libc.prctl(36, cls.previous.value, 0, 0, 0) == 0

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.processes = []
        self.canary = subprocess.Popen([sys.executable, "-I", "-c", "import signal;signal.pause()"], start_new_session=True)
        self.processes.append(self.canary)

    def tearDown(self):
        for process in self.processes:
            try:
                driver.stop_owned_session(process, timeout=2)
            except Exception:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait(timeout=2)
                raise
        self.temp.cleanup()

    def tree(self, early=False, ignore=False):
        ready = Path(self.temp.name) / ("child-"+str(len(self.processes))+".json")
        process = subprocess.Popen([sys.executable, "-I", "-c", LEADER, CHILD, str(ready), "ignore" if ignore else "term", "exit" if early else "live"], start_new_session=True)
        self.processes.append(process)
        deadline = time.monotonic()+4
        while not ready.exists():
            if time.monotonic() >= deadline:
                self.fail("owned child did not become ready")
            time.sleep(.01)
        child = json.loads(ready.read_text())
        self.assertEqual(child["pgid"], process.pid)
        self.assertEqual(child["session"], process.pid)
        self.assertNotEqual(child["pgid"], os.getpgrp())
        self.assertNotEqual(child["pgid"], self.canary.pid)
        leader = json.loads(Path(str(ready)+".leader").read_text())
        self.assertEqual(leader["pgid"], process.pid)
        self.assertEqual(leader["session"], process.pid)
        print(json.dumps({"leaderAtReadiness":leader,"childAtReadiness":child,"canaryPID":self.canary.pid,"canaryPGID":os.getpgid(self.canary.pid),"ownerPGID":os.getpgrp()}), flush=True)
        if early:
            self.assertEqual(process.wait(timeout=3), 7)
        return process, child

    def assert_gone_and_canary_alive(self, process, child):
        with self.assertRaises(ProcessLookupError):
            os.killpg(process.pid, 0)
        self.assertFalse(Path("/proc/"+str(child["pid"])).exists())
        self.assertIsNone(self.canary.poll())
        os.kill(self.canary.pid, 0)

    def test_real_descendant_tree(self):
        process, child = self.tree()
        driver.stop_owned_session(process, timeout=2)
        self.assert_gone_and_canary_alive(process, child)

    def test_exited_leader_and_term_ignoring_descendant(self):
        process, child = self.tree(early=True, ignore=True)
        started = time.monotonic()
        driver.stop_owned_session(process, timeout=.6)
        self.assertGreaterEqual(time.monotonic()-started, .3)
        self.assertEqual(process.returncode, 7)
        self.assert_gone_and_canary_alive(process, child)

    def test_original_error_survives_successful_teardown(self):
        process, child = self.tree()
        with self.assertRaisesRegex(RuntimeError, "original consumer failure"):
            try:
                raise RuntimeError("original consumer failure")
            finally:
                driver.stop_owned_session(process, timeout=2)
        self.assert_gone_and_canary_alive(process, child)

    def test_unsettled_timeout_is_an_error_not_success(self):
        process, child = self.tree(ignore=True)
        killpg = os.killpg
        def observation_never_settles(group, sig):
            if sig == 0:
                return None
            return killpg(group, sig)
        with patch.object(driver.os, "killpg", observation_never_settles):
            with self.assertRaisesRegex(RuntimeError, "did not settle"):
                driver.stop_owned_session(process, timeout=.3)
        self.assert_gone_and_canary_alive(process, child)


if __name__ == "__main__":
    unittest.main(verbosity=2)
