# Scene Vault Ren'Py bridge (installed by Scene Vault).
#
# The bridge never captures the screen and never changes game logic. When the
# player takes the engine's own screenshot (default key: S), it records the
# current game context into a small JSON file inside the app cache directory
# named by `scenevault_bridge.json`. Scene Vault ingests that file and deletes
# it right away; nothing is written next to the screenshots.
#
# Safety rules followed by this file:
#   * Python 2 / Python 3 compatible, ASCII-only source.
#   * Every entry point is wrapped; failures are logged and swallowed so the
#     game can never be blocked or crashed by the bridge.
#   * No store variables are written (state lives on renpy.config).

init -1000 python:
    # SCENEVAULT-BRIDGE-LOGIC-BEGIN
    import io
    import json
    import os
    import time

    SCENEVAULT_BRIDGE_VERSION = "1.1.3"
    SCENEVAULT_BRIDGE_SCHEMA = 1

    def _sv_text(value):
        """Coerce engine values to text without crashing on tag objects."""
        if value is None:
            return None
        if isinstance(value, str):
            return value
        if type(value).__name__ == "unicode":
            return value
        try:
            return u"%s" % (value,)
        except Exception:
            return None

    def _sv_replace(source, target):
        replacer = getattr(os, "replace", None)
        if replacer is not None:
            replacer(source, target)
        else:
            os.rename(source, target)

    class _SceneVaultBridge(object):
        def __init__(self):
            self.config = None
            self.dialogue = None
            self.installed = False

        # -- setup -------------------------------------------------------

        def install(self):
            if self.installed:
                return
            self.installed = True
            self.config = self._load_config()
            if self.config is None:
                return
            try:
                self._wrap_screenshot_callback()
            except Exception:
                self.log_exception("wrap_screenshot_callback")
            try:
                self._register_character_callback()
            except Exception:
                self.log_exception("register_character_callback")

        def _load_config(self):
            try:
                gamedir = renpy.config.gamedir
                path = os.path.join(gamedir, "scenevault_bridge.json")
                if not os.path.isfile(path):
                    return None
                handle = io.open(path, "r", encoding="utf-8")
                try:
                    return json.loads(handle.read())
                finally:
                    handle.close()
            except Exception:
                return None

        def _wrap_screenshot_callback(self):
            previous = getattr(renpy.config, "screenshot_callback", None)
            if getattr(previous, "_scenevault_wrapped", False):
                return

            bridge = self

            def _scenevault_screenshot_callback(filename):
                if previous is not None:
                    try:
                        previous(filename)
                    except Exception:
                        bridge.log_exception("previous screenshot_callback")
                try:
                    bridge.on_screenshot(filename)
                except Exception:
                    bridge.log_exception("on_screenshot")

            _scenevault_screenshot_callback._scenevault_wrapped = True
            renpy.config.screenshot_callback = _scenevault_screenshot_callback

        def _register_character_callback(self):
            callbacks = getattr(renpy.config, "all_character_callbacks", None)
            if not isinstance(callbacks, list):
                return
            bridge = self

            def _scenevault_character_callback(event, **kwargs):
                try:
                    if event != "begin":
                        return
                    text = _sv_text(kwargs.get("what"))
                    if text:
                        bridge.dialogue = text
                except Exception:
                    bridge.log_exception("character_callback")

            callbacks.append(_scenevault_character_callback)

        # -- capture-time context ----------------------------------------

        def on_screenshot(self, screenshot_path):
            started = time.time()
            if not self.config.get("metadataDirectory"):
                return
            payload = {
                "schemaVersion": SCENEVAULT_BRIDGE_SCHEMA,
                "bridgeVersion": SCENEVAULT_BRIDGE_VERSION,
                "createdAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "screenshot": {"file": os.path.basename(screenshot_path)},
            }
            errors = []
            speaker, speaker_source, speaker_variable = self.current_speaker()
            if speaker:
                payload["speaker"] = speaker
            if speaker_source:
                payload["speakerSource"] = speaker_source
            if speaker_variable:
                payload["speakerVariable"] = speaker_variable
            dialogue = self.current_dialogue()
            if dialogue:
                payload["dialogue"] = dialogue
            virtual = self._engine_size()
            if virtual:
                payload["virtualResolution"] = list(virtual)
            physical = self._physical_size()
            if physical:
                payload["physicalSize"] = list(physical)
            payload["sceneTags"] = self.scene_tags()
            payload["errors"] = errors
            payload["timing"] = {"callbackMs": int((time.time() - started) * 1000)}
            self.write_sidecar(screenshot_path, payload)

        def current_speaker(self):
            """Display name of the current speaker.

            `_last_say_who` stores whatever the say statement used: a script
            identifier ("ash"), a dotted namespace ("character.ash"), a quoted
            display string, a Character object, or None for the narrator.
            Identifiers are resolved through the store so common
            `define ash = Character("Ashley")` games report the real name; a
            resolved store value that is not a Character (for example a
            dynamic name variable) is the display text itself. Dynamic
            Characters have their name expression evaluated like the engine
            does at say time. The raw identifier is kept separately for
            debugging.
            """
            try:
                who = getattr(renpy.store, "_last_say_who", None)
            except Exception:
                return None, None, None
            if who is None:
                return None, None, None
            variable = None
            candidate = who
            text = None
            if not hasattr(candidate, "name"):
                raw = _sv_text(candidate)
                if raw is None:
                    return None, None, None
                raw = raw.strip()
                quoted = (
                    len(raw) >= 2
                    and raw[0] == raw[-1]
                    and raw[0] in ("'", '"')
                )
                if quoted:
                    # A quoted speaker is already the literal display name.
                    text = raw[1:-1].strip()
                else:
                    resolved = self._resolve_store_identifier(raw)
                    if resolved is not None and hasattr(resolved, "name"):
                        variable = raw
                        candidate = resolved
                    elif resolved is not None:
                        # Dynamic names read a plain store value at say time;
                        # that value is what the player saw.
                        variable = raw
                        text = _sv_text(resolved)
            name = getattr(candidate, "name", None)
            if text is None and name is not None:
                if callable(name):
                    try:
                        name = name()
                    except Exception:
                        name = None
                if getattr(candidate, "dynamic", False):
                    # dynamic Characters evaluate their name expression
                    # through the engine on every say; mirror that instead
                    # of showing the raw expression string.
                    evaluated = self._evaluate_dynamic_name(name)
                    if evaluated is not None:
                        name = evaluated
                text = _sv_text(name)
            if text is None:
                text = _sv_text(candidate)
            if text is None:
                return None, None, variable
            text = self._strip_text_tags(self._interpolate(text)).strip()
            if not text:
                return None, None, variable
            return text, "_last_say_who", variable

        def _resolve_store_identifier(self, name):
            """Resolve store identifiers, including dotted namespaces such as
            `character.ash`; expressions never resolve."""
            if not name or "{" in name or "}" in name:
                return None
            candidate = renpy.store
            for segment in name.split("."):
                if not segment:
                    return None
                candidate = getattr(candidate, segment, None)
                if candidate is None:
                    return None
            return candidate

        def _evaluate_dynamic_name(self, expression):
            """Evaluates a dynamic Character name expression like the engine."""
            evaluator = getattr(getattr(renpy, "python", None), "py_eval", None)
            if evaluator is None:
                return None
            try:
                return evaluator(expression)
            except Exception:
                return None

        def _interpolate(self, text):
            substitute = getattr(renpy, "substitute", None)
            if substitute is None:
                return text
            try:
                return substitute(text)
            except Exception:
                return text

        def _strip_text_tags(self, text):
            out = []
            in_tag = False
            for ch in text:
                if ch == "{":
                    in_tag = True
                elif ch == "}":
                    in_tag = False
                elif not in_tag:
                    out.append(ch)
            return "".join(out)

        def current_dialogue(self):
            for candidate in (self.dialogue, self._last_say_what()):
                text = _sv_text(candidate)
                if text:
                    text = text.strip()
                    if text:
                        return text
            return None

        def _last_say_what(self):
            try:
                return getattr(renpy.store, "_last_say_what", None)
            except Exception:
                return None

        def _engine_size(self):
            try:
                width = int(renpy.config.screen_width)
                height = int(renpy.config.screen_height)
            except Exception:
                return None
            if width <= 0 or height <= 0:
                return None
            return (width, height)

        def _physical_size(self):
            getter = getattr(renpy, "get_physical_size", None)
            if getter is None:
                return None
            try:
                size = getter()
                width = int(size[0])
                height = int(size[1])
            except Exception:
                return None
            if width <= 0 or height <= 0:
                return None
            return (width, height)

        def scene_tags(self):
            tags = None
            try:
                tags = renpy.get_showing_tags("master", sort=True)
            except Exception:
                try:
                    tags = renpy.get_showing_tags("master")
                except Exception:
                    tags = None
            if not tags:
                return []
            rv = []
            for tag in tags:
                name = _sv_text(tag)
                if not name:
                    continue
                attributes = None
                try:
                    attributes = renpy.get_attributes(tag)
                except Exception:
                    attributes = None
                if attributes:
                    parts = [name]
                    for attribute in attributes:
                        text = _sv_text(attribute)
                        if text:
                            parts.append(text)
                    rv.append(" ".join(parts))
                else:
                    rv.append(name)
            return rv

        def _ensure_directory(self, path):
            try:
                if not os.path.isdir(path):
                    os.makedirs(path)
            except Exception:
                pass

        # -- output ------------------------------------------------------

        def sidecar_path(self, screenshot_path):
            metadata_directory = self.config.get("metadataDirectory")
            if not metadata_directory:
                return None
            stem = os.path.splitext(os.path.basename(screenshot_path))[0]
            return os.path.join(metadata_directory, stem + ".scenevault.json")

        def write_sidecar(self, screenshot_path, payload):
            target = self.sidecar_path(screenshot_path)
            if target is None:
                return
            self._ensure_directory(os.path.dirname(target))
            temporary = target + ".tmp"
            text = json.dumps(payload, ensure_ascii=False, indent=2)
            handle = io.open(temporary, "w", encoding="utf-8")
            try:
                handle.write(text)
            finally:
                handle.close()
            _sv_replace(temporary, target)

        def log_exception(self, where):
            import traceback
            try:
                self.log("error in %s: %s" % (where, traceback.format_exc()))
            except Exception:
                pass

        def log(self, message):
            path = None
            if self.config:
                path = self.config.get("logFile")
            if not path:
                return
            try:
                if os.path.isfile(path) and os.path.getsize(path) > 524288:
                    os.remove(path)
                handle = io.open(path, "a", encoding="utf-8")
                try:
                    handle.write(
                        u"[%s] %s\n" % (time.strftime("%Y-%m-%d %H:%M:%S"), message)
                    )
                finally:
                    handle.close()
            except Exception:
                pass

    def _scenevault_bridge_install():
        bridge = getattr(renpy.config, "scenevault_bridge", None)
        if bridge is None:
            bridge = _SceneVaultBridge()
            renpy.config.scenevault_bridge = bridge
        bridge.install()
    # SCENEVAULT-BRIDGE-LOGIC-END


init 999 python:
    try:
        _scenevault_bridge_install()
    except Exception:
        pass
