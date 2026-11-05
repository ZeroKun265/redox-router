# RedoxRouter Roadmap

Legend: [x] done, [ ] todo. Add new tasks as we go.

## Done
- [x] Cargo project skeleton (eframe/egui window)
- [x] Roadmap
- [x] Decision: use the `pipewire` crate, not CLI tools
- [x] PipeWire thread + channel to UI; live node list (UNTESTED, never compiled yet)

## Foundation
- [x] Verified by ZeroKun265 on the real system (2026-11-04): steps 1-6 compile and run
- [x] Ports (registry events, direction, parent node) (UNTESTED on real PipeWire)
- [x] Links (registry events) (UNTESTED on real PipeWire)
- [x] Graph model in `graph.rs` with unit tests (4 pass)
- [ ] Verify ports/links list matches `pw-link -l` on the real system
- [x] Canvas: nodes as boxes with ports, pan (drag background), zoom (scroll), drag nodes, links as curves (UNTESTED on screen; `layout.rs` math has 7 passing tests)
- [ ] Verify canvas on the real system (nodes without ports are not drawn yet; auto-placement happens when a node first has ports, "Reset layout" re-stacks)
- [x] Link ownership model: links we create ("owned") are non-lingering, so PipeWire removes them when RedoxRouter exits or crashes (UNTESTED on real PipeWire)
- [x] Drag port to port to create an owned link; click a link + Delete removes it (owned only) (UNTESTED)
- [x] External links hidden by default; "Show external links" toggle; per-link "Show as visible only" via the already-connected prompt (UNTESTED)
- [x] "Make Redox connection": from the already-connected prompt, or via the top-bar button when an external link is selected. Destroys the original (`registry.destroy_global`) and recreates it as ours (UNTESTED; may fail if PipeWire permissions deny destroying another client's link, error shows in the top bar)
- [ ] Right-click menu on links (currently: click to select, then Delete or the top-bar button)
- [ ] Verify link creation really disappears on quit, and that port-to-port drag works on screen
- [ ] Fit-to-view / zoom-to-fit button
- [x] Save node positions, pan and zoom to `~/.config/RedoxRouter/layout.json` (keyed by node.name, kept for absent nodes, saved at most once per second; UNTESTED, `layout.rs` claim logic has a passing test)
- [ ] Verify layout survives restart and an unplugged/replugged device
- [ ] Place new (unsaved) nodes so they avoid overlapping saved ones
- [ ] Remember window size and position
- [x] Resizable nodes (drag bottom-right grip, double-click grip = automatic width), wrapped text (max 2 lines, "…" when cut), widths saved by name (UNTESTED)
- [x] Nickname tab above each node (pencil opens editor; Enter/click away saves, Esc cancels), saved by name (UNTESTED)

## Features requested
- [x] Hide nodes manually (right-click node -> Hide node; "Hidden (n)" menu to show; saved by node name; UNTESTED)
- [x] Hide nodes by filter rules (Rules window: field + text/wildcard pattern + Hide; saved in layout.json; matching engine has unit tests; UI UNTESTED)
- [x] Color rules, filter-based (header color via the Rules window, UNTESTED)
- [x] Color a single node manually: right-click -> Color swatches / Reset color. Writes an exact-name rule into the first group "Manual colors" (shown in the Rules window; engine tested, menu UNTESTED)
- [x] Volume sliders on audio nodes (Props via `set_param`; cubic curve 0-150%, double-click = 100%, click the % to mute; UNTESTED, math + state merge tested)
- [x] Sound card volume: devices use their route volume (what the desktop applet changes); nodes without one use node volume (UNTESTED; state logic tested)
- [ ] Volume: per-channel sliders, show both route (hardware) and node (software) volume if they differ
- [ ] Virtual microphone sinks
- [ ] Virtual speaker sinks
- [ ] Save/load setups, resilient to missing nodes
- [ ] Auto-connect nodes when they appear
- [ ] Text comments
- [ ] Translucent colored comment blocks
- [ ] Cables as splines
- [ ] Cables as user-drawn horizontal/vertical segments

## Ideas / later
- Tray icon (StatusNotifier); close window = hide, Quit = drop owned links. Some desktops need an extension for tray icons.
- Per-connection "keep after quit" flag (set object.linger on that link)
- Prompt on connect is per port pair, not per node pair
- [x] Tooltip on a node's title block with the technical name, class and id (UNTESTED)
- [x] Nickname rule field (UNTESTED UI; engine tested)
- [x] Rule groups (own on/off, Up/Down order) and a shared rule library (`rule_library.json`): "To library", "Copy here", "Use here", "Delete from library"; setups reference library groups with their own on/off (engine tested; UI UNTESTED)
- [ ] Collapse/expand rule groups
- [ ] Rule library: import/export a group as a file, rename a library group
- [ ] When saved setups exist: each setup stores its own group list (library references + local groups); layout.json is the only "setup" for now
- Multiple setups as tabs (each: own canvas, rules, links), with a "make active" button. Plan: switch in-process (no app restart): destroy current owned links, create the setup's links (stored by node/port names), auto-connect as nodes appear.
