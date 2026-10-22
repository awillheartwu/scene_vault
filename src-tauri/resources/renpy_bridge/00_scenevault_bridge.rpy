# Scene Vault Ren'Py bridge (installed by Scene Vault).
#
# The bridge records the engine's screenshots (default key: S). An opt-in
# policy also captures after suppressing standard UI in a temporary
# context, then restores the original interaction without advancing it.
# It records the current game context in a JSON file inside the app cache
# named by `scenevault_bridge.json`. Scene Vault ingests that file and deletes
# it right away; nothing is written next to the screenshots.
#
# Safety rules followed by this file:
#   * Python 2 / Python 3 compatible, ASCII-only source.
#   * Bridge failures are logged and contained; engine control flow such as
#     quitting and restarting still propagates normally.
#   * No gameplay variables are assigned; bridge state lives on renpy.config.

init -1000 python:
    # SCENEVAULT-BRIDGE-LOGIC-BEGIN
    import io as _sv_io
    import json as _sv_json
    import os as _sv_os
    import time as _sv_time
    try:
        import builtins as _sv_builtins
    except ImportError:
        import __builtin__ as _sv_builtins

    SCENEVAULT_BRIDGE_VERSION = "1.1.12"
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
        replacer = getattr(_sv_os, "replace", None)
        if replacer is not None:
            replacer(source, target)
        else:
            _sv_os.rename(source, target)

    class _SceneVaultBridge(_sv_builtins.object):
        def __init__(self):
            self.config = None
            self.dialogue = None
            self.installed = False
            self.capture_in_progress = False
            self.pending_snapshot = None
            self.policy_error_logged = False
            self.screen_action_at = 0.0

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
            try:
                callbacks = getattr(renpy.config, "interact_callbacks", None)
                guard = self._maintain_wrapping
                if callbacks is not None and guard not in callbacks:
                    callbacks.append(guard)
            except Exception:
                self.log_exception("register_interact_guard")
            try:
                overlay_screens = getattr(renpy.config, "overlay_screens", None)
                if overlay_screens is not None and "_scenevault_screenshot_key" not in overlay_screens:
                    overlay_screens.append("_scenevault_screenshot_key")
            except Exception:
                self.log_exception("register_screenshot_screen")
            try:
                # Wrap even while disabled: inherited settings can change in
                # the app without restarting or rewriting the game files.
                wrapped = self._wrap_screenshot_actions()
                self.log("bridge %s ready: autoHideUi=%s, screenshot keymaps=%s" %
                         (SCENEVAULT_BRIDGE_VERSION, self._auto_hide_enabled(), wrapped))
                if not wrapped:
                    self.log("warning: no screenshot keymap found; automatic UI hiding is unavailable")
            except Exception:
                self.log_exception("wrap_screenshot_actions")

        def _load_config(self):
            try:
                gamedir = renpy.config.gamedir
                path = _sv_os.path.join(gamedir, "scenevault_bridge.json")
                if not _sv_os.path.isfile(path):
                    return None
                handle = _sv_io.open(path, "r", encoding="utf-8")
                try:
                    return _sv_json.loads(handle.read())
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
            if not isinstance(callbacks, _sv_builtins.list):
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

        def _capture_context(self):
            payload = {
                "schemaVersion": SCENEVAULT_BRIDGE_SCHEMA,
                "bridgeVersion": SCENEVAULT_BRIDGE_VERSION,
                "createdAt": _sv_time.strftime("%Y-%m-%dT%H:%M:%SZ", _sv_time.gmtime()),
            }
            errors = []
            engine_version = self._engine_version()
            if engine_version:
                payload["engineVersion"] = engine_version
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
            return payload

        def on_screenshot(self, screenshot_path):
            started = _sv_time.time()
            if not self.config.get("metadataDirectory"):
                return
            snapshot = self.pending_snapshot
            if snapshot and _sv_os.path.abspath(screenshot_path) == snapshot[0]:
                payload = _sv_builtins.dict(snapshot[1])
            else:
                payload = self._capture_context()
                payload["captureMode"] = ("manual-ui-hidden" if
                    getattr(renpy.store, "_windows_hidden", False) else "native")
                payload["uiSuppressionApplied"] = False
                if self._auto_hide_enabled() and payload["captureMode"] == "native":
                    self.log("screenshot bypassed automatic UI hiding: " + _sv_os.path.basename(screenshot_path))
            payload["screenshot"] = {"file": _sv_os.path.basename(screenshot_path)}
            payload["timing"] = {"callbackMs": int((_sv_time.time() - started) * 1000)}
            self.write_sidecar(screenshot_path, payload)

        # -- optional standard-UI screenshot action ----------------------

        def _wrap_screenshot_actions(self):
            # Keymaps hold the original callable. Reassigning _screenshot
            # alone would miss those references; retain the game's keymap.
            wrapped = 0
            for underlay in getattr(renpy.config, "underlay", []):
                mapping = getattr(underlay, "keymap", None)
                # .rpy globals replace dict/list with rollback types. Engine
                # Keymap.keymap is a native Python dictionary, so explicitly
                # use builtins (which also accepts revertable subclasses).
                if not isinstance(mapping, _sv_builtins.dict) or "screenshot" not in mapping:
                    continue
                previous = mapping["screenshot"]
                if getattr(previous, "_scenevault_action", False):
                    wrapped += 1
                    continue
                bridge = self

                def screenshot_action(_previous=previous):
                    return bridge._screenshot_action(_previous)

                screenshot_action._scenevault_action = True
                mapping["screenshot"] = screenshot_action
                wrapped += 1
            return wrapped

        def _maintain_wrapping(self):
            # Games may rebuild the underlay keymap or reset the callback
            # after init. Reapply the wrappers before each interaction so the
            # screenshot key cannot silently bypass auto-hide.
            try:
                self._wrap_screenshot_callback()
                self._wrap_screenshot_actions()
            except Exception:
                self.log_exception("maintain wrapping")

        def _screenshot_from_screen(self):
            # This key is shown on the overlay layer, so it also works when a
            # modal phone/menu screen or a standard renpy.pause owns the
            # interaction and the underlay keymap cannot see S.
            if self.capture_in_progress:
                return
            previous = getattr(renpy.store, "_screenshot", None)
            if previous is None:
                self.log("screen screenshot action: native action missing")
                return
            self.screen_action_at = _sv_time.time()
            return self._screenshot_action(previous, from_screen=True)

        def _auto_hide_enabled(self):
            # Rust owns policy persistence. Python reads only this small local
            # cache file and never opens the application's SQLite database.
            path = self.config.get("screenshotPolicyFile")
            if path:
                try:
                    handle = _sv_io.open(path, "r", encoding="utf-8")
                    try:
                        policy = _sv_json.loads(handle.read())
                    finally:
                        handle.close()
                    if policy.get("schemaVersion") != 1:
                        raise ValueError("unsupported screenshot policy schema")
                    installed = policy.get("installations", {})
                    value = installed.get(self.config.get("integrationId"))
                    if not isinstance(value, _sv_builtins.bool):
                        value = policy.get("projects", {}).get(self.config.get("projectId"))
                    if not isinstance(value, _sv_builtins.bool):
                        raise ValueError("screenshot policy has no matching project")
                    self.policy_error_logged = False
                    return value
                except Exception:
                    if not self.policy_error_logged:
                        self.log_exception("read screenshot policy; using installed setting")
                        self.policy_error_logged = True
            return self.config.get("autoHideUi") is True

        def _auto_hide_unavailable(self, previous):
            context = renpy.context()
            if getattr(context, "_menu", False) or getattr(renpy.store, "_main_menu", False):
                return "menu"
            # renpy.context() already returns Context.info in Ren'Py 7/8.
            interaction = getattr(context, "_current_interact_type", None)
            if interaction not in ("say", "nvl", "menu", "pause"):
                return "non-story interaction"
            interface = getattr(getattr(renpy, "game", None), "interface", None)
            if interface is None:
                return "missing transition API"
            if getattr(interface, "trans_pause", False):
                return "transition pause"
            # Entering a new context clears the engine's transition state.
            # Finished standard transitions are safe; active or unknown ones
            # must stay untouched, even if the previous interact type was say.
            for layer, transition in getattr(interface, "ongoing_transition", {}).items():
                if not transition:
                    continue
                start = getattr(interface, "transition_time", {}).get(layer)
                delay = getattr(transition, "delay", None)
                frame = getattr(interface, "frame_time", None)
                if start is None or delay is None or frame is None or frame - start < delay:
                    return "active transition"
            if getattr(renpy.config, "skipping", None):
                return "skipping"
            if previous is not getattr(renpy.store, "_screenshot", None):
                return "custom screenshot action"
            # Ren'Py 8's default is Hide("notify", immediately=True). It is
            # already hidden by clearing the temporary context's screen layer.
            hide = getattr(renpy.store, "Hide", None)
            for action in getattr(renpy.config, "pre_screenshot_actions", []):
                if (type(action) is not hide or getattr(action, "screen", None) != "notify"
                        or getattr(action, "immediately", False) is not True
                        or getattr(action, "transition", None) is not None
                        or getattr(action, "_layer", None) is not None):
                    return "custom pre-screenshot actions"
            if renpy.has_label("hide_windows"):
                return "custom hide_windows"
            layers = getattr(renpy.config, "context_clear_layers", [])
            interface_layer = getattr(renpy.config, "interface_layer", "screens")
            if interface_layer not in layers or _sv_builtins.any(layer not in ("screens", "top", "bottom", "x52Overlay") for layer in layers):
                return "custom context layers"
            for name, allowed in (
                ("layers", ("master", "transient", "screens", "overlay", "x52Overlay")),
                ("top_layers", ("top",)), ("bottom_layers", ("bottom",)),
            ):
                if _sv_builtins.any(layer not in allowed for layer in getattr(renpy.config, name, [])):
                    return "custom display layers"
            if "x52Overlay" in getattr(renpy.config, "layers", []) and not _sv_builtins.callable(getattr(renpy, "scene", None)):
                return "missing overlay API"
            if getattr(renpy.config, "always_shown_screens", []):
                return "always-shown screens"
            ui = getattr(renpy, "ui", None)
            if not _sv_builtins.all(_sv_builtins.callable(getattr(ui, name, None)) for name in ("timer", "keymap", "interact")):
                return "missing UI API"
            if not _sv_builtins.callable(getattr(renpy, "invoke_in_new_context", None)) or not _sv_builtins.callable(getattr(renpy, "screenshot", None)):
                return "missing capture API"
            return None

        def _screenshot_filename(self):
            destination = renpy.config.renpy_base
            if getattr(renpy, "macapp", False):
                destination = _sv_os.path.expanduser(u"~/Desktop")
            pattern = getattr(renpy.store, "_screenshot_pattern", None) or renpy.config.screenshot_pattern
            number = 1
            while True:
                filename = _sv_os.path.abspath(_sv_os.path.join(destination, pattern % number))
                if not _sv_os.path.exists(filename):
                    self._ensure_directory(_sv_os.path.dirname(filename))
                    return filename
                number += 1

        def _capture_hidden_frame(self, filename):
            ui = renpy.ui
            # URM adds its controls above the standard screen layer. Clear
            # only this known UI layer inside the copied capture context;
            # returning from the context restores the game's original layer.
            if "x52Overlay" in getattr(renpy.config, "layers", []):
                renpy.scene(layer="x52Overlay")
            # Match the engine's standard H interaction: keep spoken dialogue
            # playing through this temporary interaction using its public API.
            sustain = getattr(renpy.store, "voice_sustain", None)
            if callable(sustain):
                sustain()
            # A timer is processed by the hidden interaction after its draw,
            # not a blocking sleep or a queued save after UI restoration.
            def save():
                self.save_attempted = True
                return "saved" if renpy.screenshot(filename) else "failed"

            def cancel():
                return "cancelled"

            ui.keymap(game_menu=cancel, hide_windows=cancel)
            ui.timer(0.05, action=save)
            return ui.interact(
                suppress_overlay=True, suppress_window=True,
                suppress_underlay=True, show_mouse=False,
            )

        def _engine_control_flow(self, error):
            game = getattr(renpy, "game", None)
            names = ("QuitException", "JumpException", "JumpOutException", "CallException",
                     "RestartContext", "RestartTopContext", "FullRestartException",
                     "UtterRestartException", "EndReplay")
            classes = tuple(getattr(game, name) for name in names if hasattr(game, name))
            return isinstance(error, classes)

        def _notify(self, message):
            try:
                renpy.notify(message)
            except Exception:
                self.log_exception("notify")

        def _screenshot_action(self, previous, from_screen=False):
            if self.capture_in_progress:
                return
            if not from_screen and _sv_time.time() - getattr(self, "screen_action_at", 0.0) < 0.5:
                return
            # H then S remains the game's own screenshot path. This is also
            # the explicit manual retry after an unsupported auto-hide.
            auto_hide = self._auto_hide_enabled()
            hidden = getattr(renpy.store, "_windows_hidden", False)
            if not auto_hide or hidden:
                return renpy.run(previous)
            self.capture_in_progress = True
            self.save_attempted = False
            try:
                reason = self._auto_hide_unavailable(previous)
                if reason:
                    self.log("auto-hide unsupported: " + reason)
                    self._notify("Auto-hide unavailable (%s). No screenshot taken. Press H, then S." % reason)
                    return
                filename = self._screenshot_filename()
                payload = self._capture_context()
                payload["contextSnapshotAt"] = payload["createdAt"]
                payload["captureMode"] = "standard-ui-hidden"
                payload["uiSuppressionApplied"] = True
                self.pending_snapshot = (filename, payload)
                result = renpy.invoke_in_new_context(self._capture_hidden_frame, filename)
                # invoke_in_new_context restores the original display even
                # on exceptions. Notify the game only after that restoration.
                if result == "saved":
                    self.log("auto-hide screenshot saved: " + _sv_os.path.basename(filename))
                    self._wrap_screenshot_callback()
                    renpy.config.screenshot_callback(filename)
                elif result == "cancelled":
                    self._notify("Screenshot cancelled. UI restored.")
                else:
                    self.log("auto-hide screenshot save failed: " + _sv_os.path.basename(filename))
                    self._notify("Screenshot save failed. Check the screenshot folder and retry. UI restored.")
            except Exception as error:
                if self._engine_control_flow(error):
                    raise
                self.log_exception("auto-hide screenshot")
                if self.save_attempted:
                    self._notify("Screenshot result uncertain. Check the screenshot folder before retrying. UI restored.")
                else:
                    self._notify("Auto-hide failed. No screenshot taken. Press H, then S.")
            finally:
                self.pending_snapshot = None
                self.capture_in_progress = False
                try:
                    renpy.restart_interaction()
                except Exception:
                    self.log_exception("restore interaction")

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

        def _engine_version(self):
            """Exact version of the running engine, when the build exposes it.

            Both spellings of ``renpy.version`` exist in the wild: a plain
            string on some builds and a callable on others, so accept either
            and strip the "Ren'Py " prefix.
            """
            for attribute in ("version_only", "version"):
                value = getattr(renpy, attribute, None)
                if value is None:
                    continue
                try:
                    text = value() if callable(value) else value
                except Exception:
                    continue
                text = _sv_text(text)
                if not text:
                    continue
                text = text.strip()
                if text.lower().startswith("ren'py "):
                    text = text[7:].strip()
                if text:
                    return text
            return None

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
                if not _sv_os.path.isdir(path):
                    _sv_os.makedirs(path)
            except Exception:
                pass

        # -- output ------------------------------------------------------

        def sidecar_path(self, screenshot_path):
            metadata_directory = self.config.get("metadataDirectory")
            if not metadata_directory:
                return None
            stem = _sv_os.path.splitext(_sv_os.path.basename(screenshot_path))[0]
            return _sv_os.path.join(metadata_directory, stem + ".scenevault.json")

        def write_sidecar(self, screenshot_path, payload):
            target = self.sidecar_path(screenshot_path)
            if target is None:
                return
            self._ensure_directory(_sv_os.path.dirname(target))
            temporary = target + ".tmp"
            text = _sv_json.dumps(payload, ensure_ascii=False, indent=2)
            handle = _sv_io.open(temporary, "w", encoding="utf-8")
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
                if _sv_os.path.isfile(path) and _sv_os.path.getsize(path) > 524288:
                    _sv_os.remove(path)
                handle = _sv_io.open(path, "a", encoding="utf-8")
                try:
                    handle.write(
                        u"[%s] %s\n" % (_sv_time.strftime("%Y-%m-%d %H:%M:%S"), message)
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

    def _scenevault_screen_screenshot():
        bridge = getattr(renpy.config, "scenevault_bridge", None)
        if bridge is not None:
            return bridge._screenshot_from_screen()
    # SCENEVAULT-BRIDGE-LOGIC-END


init 999 python:
    try:
        _scenevault_bridge_install()
    except Exception:
        pass


screen _scenevault_screenshot_key():
    key "screenshot" action Function(_scenevault_screen_screenshot)
