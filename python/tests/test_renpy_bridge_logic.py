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


def write_config(game_directory, metadata_directory, log_file, auto_hide_ui=False):
    Path(game_directory, "scenevault_bridge.json").write_text(
        json.dumps(
            {
                "schemaVersion": 1,
                "bridgeVersion": "1.1.1",
                "metadataDirectory": str(metadata_directory),
                "logFile": str(log_file),
                "autoHideUi": auto_hide_ui,
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


class AutoHideScreenshotTests(unittest.TestCase):
    """Exercise action ordering and failures; actual rendering still needs a game."""

    def setUp(self):
        temporary = TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.game = self.root / "game"
        self.game.mkdir()
        self.metadata = self.root / "metadata"
        self.renpy = renpy = build_stub(store={"ash": _Node(name="Ashley")})
        namespace = {"renpy": renpy}
        exec(compile(load_bridge_logic(), str(BRIDGE_PATH), "exec"), namespace)
        self.namespace = namespace
        self.bridge = namespace["_SceneVaultBridge"]()
        self.events = []
        self.notices = []
        self.hidden = False
        self.context = _Node(_menu=False, _current_interact_type="say")
        renpy.context = lambda: self.context
        renpy.has_label = lambda _name: False
        renpy.config.gamedir = str(self.game)
        renpy.config.renpy_base = str(self.root)
        renpy.config.screenshot_pattern = "screenshot%04d.png"
        renpy.config.context_clear_layers = ["screens"]
        renpy.game = _Node(interface=_Node(
            trans_pause=False, ongoing_transition={}, transition_time={}, frame_time=2.0,
        ))

        class Hide:
            def __init__(self, screen, immediately=False, transition=None, _layer=None):
                self.screen = screen
                self.immediately = immediately
                self.transition = transition
                self._layer = _layer

        renpy.store.Hide = Hide
        renpy.store.voice_sustain = lambda: self.events.append("sustain")

        def native():
            self.events.append("native")

        renpy.store._screenshot = native
        self.keymap = {"screenshot": native}
        renpy.config.underlay = [_Node(keymap=self.keymap)]
        renpy.run = lambda action: action()
        renpy.notify = self.notices.append
        renpy.restart_interaction = lambda: self.events.append("restart")

        def callback(_filename):
            self.assertFalse(self.hidden, "callback must run after restoring the original context")
            self.events.append("callback")
            # A game's callback must not change the frozen screenshot metadata.
            renpy.store._last_say_what = "Changed in callback"

        renpy.config.screenshot_callback = callback

        def enter(function, *args):
            self.hidden = True
            self.events.append("enter")
            old_who = renpy.store._last_say_who
            renpy.store._last_say_who = None
            try:
                return function(*args)
            finally:
                renpy.store._last_say_who = old_who
                self.hidden = False
                self.events.append("restore")

        renpy.invoke_in_new_context = enter
        self.timer_action = None
        self.cancel_actions = {}

        def timer(delay, action):
            self.assertGreater(delay, 0)
            self.timer_action = action

        def keymap(**actions):
            self.cancel_actions = actions

        def interact(**kwargs):
            self.assertTrue(self.hidden)
            for name in ("suppress_overlay", "suppress_window", "suppress_underlay"):
                self.assertTrue(kwargs[name])
            self.assertFalse(kwargs["show_mouse"])
            self.events.append("draw")
            return self.timer_action()

        renpy.ui = _Node(timer=timer, keymap=keymap, interact=interact)

        def screenshot(filename):
            self.assertTrue(self.hidden)
            self.events.append("save")
            # Repeated S during an in-flight capture must not save another image.
            self.keymap["screenshot"]()
            Path(filename).write_bytes(b"stub image")
            return True

        renpy.screenshot = screenshot

    def install(self, enabled=True):
        write_config(self.game, self.metadata, self.root / "bridge.log", enabled)
        self.bridge.install()
        return self.keymap["screenshot"]

    def test_default_off_and_manual_h_keep_the_original_action(self):
        action = self.install(False)
        self.assertTrue(getattr(action, "_scenevault_action", False))
        action()
        self.assertEqual(self.events, ["native"])
        self.bridge.config["autoHideUi"] = True
        self.bridge._wrap_screenshot_actions()
        self.renpy.store._windows_hidden = True
        self.keymap["screenshot"]()
        self.assertEqual(self.events, ["native", "native"])

    def test_native_engine_containers_work_with_revertable_store_types(self):
        # Ren'Py replaces the store names, but Keymap is implemented in a
        # Python module and returns a native dict; config callbacks are native too.
        self.namespace["dict"] = type("RevertableDict", (dict,), {})
        self.namespace["list"] = type("RevertableList", (list,), {})
        action = self.install()
        self.assertTrue(getattr(action, "_scenevault_action", False))
        self.assertEqual(len(self.renpy.config.all_character_callbacks), 1)
        action()
        self.assertEqual(self.events.count("save"), 1)
        document = json.loads(next(self.metadata.glob("*.scenevault.json")).read_text())
        self.assertTrue(document["uiSuppressionApplied"])

    def test_live_policy_changes_apply_without_reinstalling_keymap(self):
        action = self.install(False)
        policy_path = self.root / "screenshot-policy.json"
        self.bridge.config.update({
            "screenshotPolicyFile": str(policy_path),
            "integrationId": "installation", "projectId": "project",
        })
        policy = {"schemaVersion": 1, "projects": {"project": False},
                  "installations": {"installation": True}}
        policy_path.write_text(json.dumps(policy), encoding="utf-8")
        action()
        self.assertEqual(self.events.count("save"), 1)
        policy["installations"]["installation"] = False
        policy_path.write_text(json.dumps(policy), encoding="utf-8")
        self.events.clear()
        action()
        self.assertEqual(self.events, ["native"])
        self.assertIs(action, self.keymap["screenshot"])

    def test_invalid_live_policy_uses_fallback_and_logs_once(self):
        action = self.install(False)
        policy_path = self.root / "screenshot-policy.json"
        policy_path.write_text("{", encoding="utf-8")
        self.bridge.config["screenshotPolicyFile"] = str(policy_path)
        action()
        action()
        self.assertEqual(self.events, ["native", "native"])
        log = (self.root / "bridge.log").read_text(encoding="utf-8")
        self.assertEqual(log.count("error in read screenshot policy; using installed setting"), 1)

    def test_standard_7_and_8_save_once_after_draw_with_frozen_metadata(self):
        action = self.install()
        cases = [(7, []), (8, [self.renpy.store.Hide("notify", immediately=True)])]
        for engine, pre_actions in cases:
            self.renpy.config.pre_screenshot_actions = pre_actions
            self.renpy.config.context_clear_layers = (
                ["screens"] if engine == 7 else ["screens", "top", "bottom"]
            )
            self.renpy.store._last_say_what = "Are you serious?"
            self.events.clear()
            action()
            self.assertEqual(
                self.events, ["enter", "sustain", "draw", "save", "restore", "callback", "restart"]
            )
            documents = sorted(self.metadata.glob("*.scenevault.json"))
            payload = json.loads(documents[-1].read_text(encoding="utf-8"))
            self.assertEqual(payload["speaker"], "Ashley")
            self.assertEqual(payload["dialogue"], "Are you serious?")
            self.assertEqual(payload["captureMode"], "standard-ui-hidden")
            self.assertTrue(payload["uiSuppressionApplied"])
            self.assertEqual(payload["contextSnapshotAt"], payload["createdAt"])
            self.assertIsNone(self.bridge.pending_snapshot)
            self.assertFalse(self.bridge.capture_in_progress)
        self.assertEqual(len(list(self.root.glob("screenshot*.png"))), 2)

    def test_urm_overlay_is_cleared_only_inside_capture_context(self):
        self.renpy.config.layers = ["master", "transient", "screens", "overlay", "x52Overlay"]
        original_layers = {"master": ["story image"], "x52Overlay": ["URM controls"]}
        active_layers = original_layers
        original_enter = self.renpy.invoke_in_new_context
        original_screenshot = self.renpy.screenshot

        def enter(function, *args):
            nonlocal active_layers
            active_layers = {name: list(contents) for name, contents in original_layers.items()}
            try:
                return original_enter(function, *args)
            finally:
                active_layers = original_layers

        def scene(layer):
            self.assertTrue(self.hidden, "clear only inside the temporary context")
            self.assertEqual(layer, "x52Overlay")
            active_layers[layer] = []
            self.events.append("clear-overlay")

        def screenshot(filename):
            self.assertEqual(active_layers["x52Overlay"], [])
            self.assertEqual(active_layers["master"], ["story image"])
            return original_screenshot(filename)

        self.renpy.scene = scene
        self.renpy.invoke_in_new_context = enter
        self.renpy.screenshot = screenshot
        action = self.install()
        for clear_layers in (["screens"], ["screens", "top", "bottom", "x52Overlay"]):
            with self.subTest(clear_layers=clear_layers):
                self.renpy.config.context_clear_layers = clear_layers
                self.events.clear()
                action()
                self.assertEqual(self.events, [
                    "enter", "clear-overlay", "sustain", "draw", "save", "restore", "callback", "restart",
                ])
                self.assertIs(active_layers, original_layers)
                self.assertEqual(original_layers["x52Overlay"], ["URM controls"])
                self.assertFalse(self.hidden)
                self.assertFalse(self.bridge.capture_in_progress)
        self.assertEqual(len(list(self.root.glob("screenshot*.png"))), 2)
        self.assertEqual(self.notices, [])

    def test_urm_overlay_without_clear_api_does_not_capture(self):
        self.renpy.config.layers = ["master", "transient", "screens", "overlay", "x52Overlay"]
        self.install()()
        self.assertEqual(self.events, ["restart"])
        self.assertIn("missing overlay API", self.notices[-1])
        self.assertTrue(self.notices[-1].isascii())
        self.assertEqual(list(self.root.glob("screenshot*.png")), [])

    def test_unsafe_interactions_and_custom_actions_do_not_save_or_fallback(self):
        action = self.install()
        reasons = ("menu", "skipping", "custom", "custom_layer", "transition_pause", "active_transition")
        for reason in reasons:
            self.context._menu = reason == "menu"
            self.renpy.config.skipping = reason == "skipping"
            self.renpy.config.pre_screenshot_actions = [object()] if reason == "custom" else []
            self.renpy.config.layers = (
                ["master", "hud", "screens"] if reason == "custom_layer"
                else ["master", "transient", "screens", "overlay"]
            )
            interface = self.renpy.game.interface
            interface.trans_pause = reason == "transition_pause"
            interface.ongoing_transition = (
                {None: _Node(delay=3.0)} if reason == "active_transition" else {}
            )
            interface.transition_time = {None: 0.0}
            self.events.clear()
            action()
            self.assertEqual(self.events, ["restart"], reason)
            self.assertFalse(self.hidden)
            self.assertFalse(self.bridge.capture_in_progress)
            self.assertTrue(self.notices[-1].isascii())
            self.assertIn("No screenshot taken", self.notices[-1])
        self.assertEqual(len(self.notices), 6)
        self.assertEqual(list(self.root.glob("screenshot*.png")), [])

    def test_finished_standard_transition_does_not_block_screenshot(self):
        action = self.install()
        self.renpy.game.interface.ongoing_transition = {None: _Node(delay=0.5)}
        self.renpy.game.interface.transition_time = {None: 0.0}
        action()
        self.assertEqual(self.events.count("save"), 1)

    def test_callback_failure_still_writes_metadata_without_resaving(self):
        def fail(_filename):
            self.events.append("callback")
            raise RuntimeError("game callback failed")

        self.renpy.config.screenshot_callback = fail
        self.install()()
        self.assertEqual(self.events.count("save"), 1)
        self.assertEqual(len(list(self.metadata.glob("*.scenevault.json"))), 1)
        self.assertFalse(self.hidden)

    def test_cancel_restores_without_saving(self):
        def cancel(**_kwargs):
            return self.cancel_actions["game_menu"]()

        self.renpy.ui.interact = cancel
        self.install()()
        self.assertEqual(self.events, ["enter", "sustain", "restore", "restart"])
        self.assertEqual(list(self.root.glob("screenshot*.png")), [])
        self.assertFalse(self.bridge.capture_in_progress)
        self.assertEqual(self.notices, ["Screenshot cancelled. UI restored."])

    def test_uncertain_save_is_not_retried(self):
        def fail(filename):
            self.events.append("save")
            Path(filename).write_bytes(b"possibly saved")
            raise OSError("result unknown")

        self.renpy.screenshot = fail
        self.install()()
        self.assertEqual(self.events, ["enter", "sustain", "draw", "save", "restore", "restart"])
        self.assertFalse(self.hidden)
        self.assertIsNone(self.bridge.pending_snapshot)
        self.assertEqual(len(list(self.root.glob("screenshot*.png"))), 1)
        self.assertIn("result uncertain", self.notices[-1])
        self.assertTrue(self.notices[-1].isascii())

    def test_engine_quit_propagates_after_restoring(self):
        class QuitException(Exception):
            pass

        self.renpy.game.QuitException = QuitException

        def quit_capture(_filename):
            raise QuitException()

        self.renpy.screenshot = quit_capture
        with self.assertRaises(QuitException):
            self.install()()
        self.assertFalse(self.hidden)
        self.assertIsNone(self.bridge.pending_snapshot)
        self.assertFalse(self.bridge.capture_in_progress)


if __name__ == "__main__":
    unittest.main()

