"""Logic tests for the bundled Ren'Py bridge.

The bridge ships as a `.rpy` file, so these tests extract the pure-Python
block between the SCENEVAULT-BRIDGE-LOGIC markers and execute it against a
stub `renpy` module. That keeps the speaker resolution, the scene tags and
the metadata writer covered without launching a game.
"""

import json
import re
import textwrap
import types
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

REPO_ROOT = Path(__file__).resolve().parents[2]
BRIDGE_PATH = (
    REPO_ROOT / "src-tauri" / "resources" / "renpy_bridge" / "00_scenevault_bridge.rpy"
)


def load_bridge_logic():
    source = BRIDGE_PATH.read_text(encoding="utf-8")
    begin = source.index("# SCENEVAULT-BRIDGE-LOGIC-BEGIN")
    end = source.index("# SCENEVAULT-BRIDGE-LOGIC-END")
    block = source[begin:end].splitlines()[1:]
    return textwrap.dedent("\n".join(block))


class _Node:
    """Stand-in for a Ren'Py displayable/property bag."""

    def __init__(self, **attributes):
        self.__dict__.update(attributes)


def build_stub(tags=None, attributes=None, store=None):
    state = {
        "tags": tags or [],
        "attributes": attributes or {},
    }
    renpy = types.SimpleNamespace()
    renpy.config = types.SimpleNamespace(
        gamedir="",
        screenshot_callback=None,
        all_character_callbacks=[],
        screen_width=1920,
        screen_height=1080,
    )
    renpy.store = types.SimpleNamespace(
        _last_say_who="ash", _last_say_what="Are you serious?"
    )
    for key, value in (store or {}).items():
        setattr(renpy.store, key, value)

    def _py_eval(expression):
        if hasattr(renpy.store, expression):
            return getattr(renpy.store, expression)
        raise ValueError(expression)

    renpy.python = types.SimpleNamespace(py_eval=_py_eval)
    renpy.substitute = lambda text: text
    renpy.get_showing_tags = lambda layer, sort=False: list(state["tags"])
    renpy.get_attributes = lambda tag: tuple(state["attributes"].get(tag, ()))
    renpy.get_physical_size = lambda: (3840, 2160)
    return renpy


def write_config(game_directory, metadata_directory, log_file):
    Path(game_directory, "scenevault_bridge.json").write_text(
        json.dumps(
            {
                "schemaVersion": 1,
                "bridgeVersion": "1.1.1",
                "metadataDirectory": str(metadata_directory),
                "logFile": str(log_file),
            }
        ),
        encoding="utf-8",
    )


class BridgeLogicTests(unittest.TestCase):
    def load(self, **kwargs):
        renpy = build_stub(**kwargs)
        namespace = {"renpy": renpy}
        exec(compile(load_bridge_logic(), str(BRIDGE_PATH), "exec"), namespace)
        bridge = namespace["_SceneVaultBridge"]()
        return namespace, renpy, bridge

    def test_block_is_python2_friendly(self):
        code = load_bridge_logic()
        compile(code, str(BRIDGE_PATH), "exec")
        for pattern in (r"(?<![A-Za-z0-9_])f['\"]", r":=", r"async def", r"nonlocal "):
            self.assertIsNone(re.search(pattern, code), pattern)
        self.assertNotIn("sourceAssets", code)
        self.assertNotIn("stage_asset", code)

    def test_speaker_resolves_script_identifiers_and_tags(self):
        cases = [
            ("ash", {"ash": _Node(name="Ashley")}, "Ashley", "ash"),
            ("Ashley", {}, "Ashley", None),
            ("{color=#fff}Ashley{/color}", {}, "Ashley", None),
            ("player_name", {"player_name": "Alex"}, "Alex", "player_name"),
            ('"Ashley"', {}, "Ashley", None),
            (
                "character.ash",
                {"character": _Node(ash=_Node(name="Ashley"))},
                "Ashley",
                "character.ash",
            ),
            (
                "p",
                {"p": _Node(name="player_name", dynamic=True), "player_name": "Alex"},
                "Alex",
                "p",
            ),
        ]
        for who, store, expected_name, expected_variable in cases:
            _namespace, renpy, bridge = self.load(store=store)
            renpy.store._last_say_who = who
            speaker, source, variable = bridge.current_speaker()
            self.assertEqual(speaker, expected_name, who)
            self.assertEqual(source, "_last_say_who")
            self.assertEqual(variable, expected_variable, who)

        _namespace, renpy, bridge = self.load()
        renpy.store._last_say_who = None
        self.assertEqual(bridge.current_speaker(), (None, None, None))

    def test_screenshot_hook_writes_metadata_into_the_cache_directory(self):
        namespace, renpy, _bridge = self.load(
            tags=["bedroom", "ashley"],
            attributes={"ashley": ("casual", "happy")},
            store={"ash": _Node(name="Ashley")},
        )
        calls = []
        renpy.config.screenshot_callback = lambda filename: calls.append(filename)

        with TemporaryDirectory() as directory:
            game_directory = Path(directory, "game")
            game_directory.mkdir()
            metadata_directory = Path(directory, "metadata")
            renpy.config.gamedir = str(game_directory)
            write_config(game_directory, metadata_directory, Path(directory, "bridge.log"))

            namespace["_scenevault_bridge_install"]()
            screenshot = str(game_directory / "screenshot0001.png")
            Path(screenshot).write_bytes(b"png")
            renpy.config.screenshot_callback(screenshot)

            self.assertEqual(calls, [screenshot], "previous callback must still run")
            metadata_file = metadata_directory / "screenshot0001.scenevault.json"
            document = json.loads(metadata_file.read_text(encoding="utf-8"))
            self.assertEqual(document["speaker"], "Ashley")
            self.assertEqual(document["speakerVariable"], "ash")
            self.assertEqual(document["dialogue"], "Are you serious?")
            self.assertEqual(document["virtualResolution"], [1920, 1080])
            self.assertEqual(document["physicalSize"], [3840, 2160])
            self.assertIn("ashley casual happy", document["sceneTags"])
            self.assertGreaterEqual(document["timing"]["callbackMs"], 0)
            self.assertNotIn("sourceAssets", document)
            self.assertFalse(
                (game_directory / "screenshot0001.scenevault.json").exists(),
                "nothing may be written next to the screenshots",
            )
            leftovers = [
                name.name for name in metadata_directory.iterdir() if name.name.endswith(".tmp")
            ]
            self.assertEqual(leftovers, [])

    def test_missing_metadata_directory_is_a_noop(self):
        namespace, renpy, _bridge = self.load()
        calls = []
        renpy.config.screenshot_callback = lambda filename: calls.append(filename)
        with TemporaryDirectory() as directory:
            game_directory = Path(directory, "game")
            game_directory.mkdir()
            renpy.config.gamedir = str(game_directory)
            # Older config without metadataDirectory: the bridge stays inert.
            Path(game_directory, "scenevault_bridge.json").write_text(
                json.dumps({"schemaVersion": 1, "bridgeVersion": "1.0.0"}),
                encoding="utf-8",
            )
            namespace["_scenevault_bridge_install"]()
            screenshot = str(game_directory / "screenshot0002.png")
            Path(screenshot).write_bytes(b"png")
            renpy.config.screenshot_callback(screenshot)
            self.assertEqual(calls, [screenshot])
            self.assertEqual(
                [p.name for p in game_directory.iterdir() if p.suffix == ".json"],
                ["scenevault_bridge.json"],
            )


if __name__ == "__main__":
    unittest.main()

