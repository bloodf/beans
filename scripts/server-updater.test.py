import runpy
import unittest
from pathlib import Path
from unittest.mock import patch


UPDATER = runpy.run_path(
    str(Path(__file__).with_name("server-updater.py")), run_name="server_updater_tests"
)
ANSWERS_AS = UPDATER["answers_as"]


class RelayRosterFloorTests(unittest.TestCase):
    def accepts(self, manifest_protocol, health_protocol, fields):
        health = {
            "ok": True,
            "service": "lorca-relay",
            "version": "0.1.0",
            "protocol": health_protocol,
            "min_protocol": 3,
        }
        health.update(fields)
        with patch.dict(
            ANSWERS_AS.__globals__,
            {"main_pid": lambda target: 42, "relay_health": lambda target: health},
        ):
            return ANSWERS_AS({"role": "relay"}, "0.1.0", manifest_protocol, old_pid=11)

    def test_protocol4_rejects_missing_unsafe_or_malformed_roster_floor(self):
        for fields in (
            {},
            {"min_roster_protocol": None},
            {"min_roster_protocol": 3},
            {"min_roster_protocol": "4"},
            {"min_roster_protocol": 4.0},
            {"min_roster_protocol": True},
        ):
            with self.subTest(fields=fields):
                self.assertFalse(self.accepts(4, 4, fields))

    def test_protocol4_accepts_floor4_from_matching_or_newer_relay(self):
        for health_protocol in (4, 5):
            with self.subTest(health_protocol=health_protocol):
                self.assertTrue(self.accepts(4, health_protocol, {"min_roster_protocol": 4}))

    def test_roster_floor_and_relay_must_support_the_signed_protocol(self):
        for manifest_protocol, health_protocol, roster_floor in (
            (4, 5, 5),
            (4, 4, 5),
            (5, 4, 4),
        ):
            with self.subTest(
                manifest_protocol=manifest_protocol,
                health_protocol=health_protocol,
                roster_floor=roster_floor,
            ):
                self.assertFalse(
                    self.accepts(manifest_protocol, health_protocol, {"min_roster_protocol": roster_floor})
                )

    def test_newer_manifest_keeps_roster_floor_within_supported_range(self):
        for roster_floor, accepted in ((3, False), (4, True), (5, True), (6, False)):
            with self.subTest(roster_floor=roster_floor):
                self.assertEqual(self.accepts(5, 6, {"min_roster_protocol": roster_floor}), accepted)

    def test_protocol3_releases_keep_existing_health_compatibility(self):
        for fields in ({}, {"min_roster_protocol": 3}):
            with self.subTest(fields=fields):
                self.assertTrue(self.accepts(3, 3, fields))
        self.assertFalse(self.accepts(3, 2, {}))


if __name__ == "__main__":
    unittest.main()
