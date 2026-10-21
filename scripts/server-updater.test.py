import runpy
import unittest
from pathlib import Path
from unittest.mock import patch


UPDATER = runpy.run_path(
    str(Path(__file__).with_name("server-updater.py")), run_name="server_updater_tests"
)
ANSWERS_AS = UPDATER["answers_as"]


class RelayFormatTests(unittest.TestCase):
    def accepts(self, manifest_protocol=5, **fields):
        health = {
            "ok": True,
            "service": "beans-relay",
            "version": "0.1.0",
            "format": "beans-v2",
            "protocol": 5,
            "min_protocol": 5,
            "min_roster_protocol": 5,
        }
        for key, value in fields.items():
            if value is None:
                health.pop(key, None)
            else:
                health[key] = value
        with patch.dict(
            ANSWERS_AS.__globals__,
            {"main_pid": lambda target: 42, "relay_health": lambda target: health},
        ):
            return ANSWERS_AS({"role": "relay"}, "0.1.0", manifest_protocol, old_pid=11)

    def test_rejects_missing_or_wrong_format(self):
        for value in (None, "", "beans-v1", 2, True):
            with self.subTest(value=value):
                self.assertFalse(self.accepts(format=value))

    def test_rejects_missing_malformed_or_unsafe_floors(self):
        for key in ("min_protocol", "min_roster_protocol"):
            for value in (None, 3, 4, 6, "5", 5.0, True):
                with self.subTest(key=key, value=value):
                    self.assertFalse(self.accepts(**{key: value}))

    def test_accepts_matching_or_newer_relay_with_supported_floors(self):
        self.assertTrue(self.accepts())
        self.assertTrue(self.accepts(protocol=6))
        self.assertTrue(self.accepts(6, protocol=6, min_protocol=6, min_roster_protocol=6))

    def test_effective_floors_are_ordered_within_the_signed_release(self):
        for signed, advertised, account_floor, roster_floor, accepted in (
            (6, 6, 6, 5, False),
            (6, 7, 6, 6, True),
            (6, 6, 5, 6, True),
            (5, 6, 6, 6, False),
        ):
            with self.subTest(
                signed=signed, advertised=advertised,
                account_floor=account_floor, roster_floor=roster_floor,
            ):
                self.assertEqual(
                    self.accepts(
                        signed, protocol=advertised,
                        min_protocol=account_floor, min_roster_protocol=roster_floor,
                    ),
                    accepted,
                )

    def test_rejects_older_release_or_unsupported_relay_protocol(self):
        for manifest_protocol, health_protocol in ((3, 5), (4, 5), (5, 4), (6, 5)):
            with self.subTest(manifest_protocol=manifest_protocol, health_protocol=health_protocol):
                self.assertFalse(self.accepts(manifest_protocol, protocol=health_protocol))
        for value in (None, "5", 5.0, True):
            with self.subTest(value=value):
                self.assertFalse(self.accepts(protocol=value))


class ReleaseProtocolTests(unittest.TestCase):
    def test_manifest_rejects_old_or_malformed_protocol_before_inventory(self):
        import json

        for protocol in (3, 4, "5", 5.0, True):
            manifest = {
                "schema": 1, "version": "1.0.0", "revision": "a" * 40,
                "protocol": protocol, "artifacts": [],
            }
            with self.subTest(protocol=protocol), self.assertRaisesRegex(UPDATER["Failure"], "protocol"):
                UPDATER["parse_manifest"](json.dumps(manifest).encode(), "beans-v1.0.0")


class RunnerHomeTests(unittest.TestCase):
    def target(self, **changes):
        value = {"name": "runner", "driver": "systemd", "binary": "/usr/local/bin/beans", "service": "beans.service", "home": "/var/lib/beans/.beans-v2", "token_file": "/etc/beans/update-token"}
        value.update(changes)
        return value

    def test_home_is_required_absolute_and_not_inferred_from_update_token(self):
        for home in (None, "", "relative/home", 42):
            with self.subTest(home=home), self.assertRaises(UPDATER["ConfigError"]):
                UPDATER["parse_target"](self.target(home=home), "runner", "runner")
        parsed = UPDATER["parse_target"](self.target(), "runner", "runner")
        self.assertEqual(parsed["home"], "/var/lib/beans/.beans-v2")
        self.assertEqual(parsed["token_file"], "/etc/beans/update-token")

    def test_every_driver_and_control_verb_selects_home_without_changing_update_token(self):
        import subprocess
        control = UPDATER["runner_control"]
        for driver in ("systemd", "incus"):
            for verb in ("status", "prepare", "cancel"):
                value = self.target(driver=driver)
                if driver == "incus": value["instance"] = "scratch"
                target = UPDATER["parse_target"](value, "runner", "runner")
                calls = []
                def invoke(target, argv, **kwargs):
                    calls.append((argv, kwargs))
                    return subprocess.CompletedProcess(argv, 0, stdout=b"{}", stderr=b"")
                with patch.dict(control.__globals__, {"require_target_path": lambda *args, **kwargs: None, "in_target": invoke}):
                    control(target, [verb])
                argv, kwargs = calls[0]
                self.assertEqual(argv, [target["binary"], "--home", target["home"], "--port", "4874", "update", verb])
                self.assertEqual(kwargs["env"], {"BEANS_UPDATE_TOKEN_FILE": target["token_file"]})


if __name__ == "__main__":
    unittest.main()
