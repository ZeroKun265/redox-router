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
- [x] Volume slider only on application streams. Devices (sinks/sources) show a read-only bar, using the sound card route volume when available so it matches the system applet; they are controlled by the system volume control (decision 2026-11-05, avoids duplicate control)
- [ ] Volume: per-channel sliders, show both route (hardware) and node (software) volume if they differ
- [ ] Virtual microphone sinks
- [x] Virtual speaker sinks: "Create sink…" dialog (`support.null-audio-sink` via `adapter` factory, `object.linger: false`, `redox.owned: true`) and "Destroy sink" menu/context action; saved in `sinks.json` so Redox sinks are automatically recreated on startup and cleaned up when RedoxRouter exits
- [x] Sink visibility modes (`Show all`, `Hide Redox only`, `Hide all`) + `Action::Show` in the rules engine to selectively unhide sinks via filter rules
- [x] Audio bleed / auto-connection prevention (`Allow all`, `Block for managed nodes`, `Block for all new apps`) to automatically sever unwanted external/WirePlumber default links
- [x] Composite node persistence key (`"{node.name}|{media.class}"`) so nodes sharing a technical name (e.g. Discord input vs output `WEBRTC VoiceEngine`) have independent positions, widths, nicknames, visibility, and manual colors
- [x] Saved links: links you create/take over are saved by node+port name in layout.json and auto-connected whenever both ends exist; "Saved links" window (pause/remove). Missing nodes are skipped. Duplicate node names use the oldest node. Engine tested, UI UNTESTED
- [x] Setups as tabs (`setups.json`; old layout.json migrates into "Main"). Each setup = layout, rules, saved links. Click a tab to view it; "Make active" removes our links the new setup doesn't use and auto-connects its links (● = live). Right-click tab: Rename/Duplicate/Delete; "+" = new setup copying layout+rules without links. Inactive setups show and edit their own saved links (virtual, nothing live changes). List logic tested, UI UNTESTED
- [ ] Setups: per-setup window/pan, export/import a setup
- [x] Text comments: right-click empty canvas -> Add comment; drag, double-click to edit, drag right edge for width, right-click for color/delete (saved per setup; UNTESTED)
- [x] Translucent colored blocks with a title bar (drag the bar, resize with the corner grip, right-click for edit/color/delete; drawn behind links and nodes; UNTESTED)
- [ ] Cables as splines
- [x] Zigzag cables (horizontal/vertical segments, rounded corners) via the top-bar style menu, saved per setup (math tested, UI UNTESTED)
- [x] Cable waypoints: double-click one of your links to add a point, drag to move, double-click a point to remove; saved per link; works for curved and zigzag (UNTESTED)
- [x] Box select (Shift+drag, Ctrl adds) with bottom-right All/Nodes/Links switch; selected nodes move together, Delete removes selected links; external links are not box-selectable (UNTESTED)

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
- [x] Rule match modes: contains/wildcards, exact match, is empty (blank/whitespace) on any field (engine tested, UI UNTESTED)
- [x] "Reset all settings…" button (confirmation; clears setups, links, rules, library, layout; UNTESTED)
- [x] UI theme refresh: darker rounded panels and widgets, teal accent, dot-grid canvas, node shadows/gloss, glowing links, ringed ports (apply_theme in main.rs; UNTESTED visually)
- [x] Animated flowing dots on links, glow on external links too; nickname tab drawn behind its node (fused look)
- [x] Markdown in comments (headings, bullets, bold, italic, code, strike; custom subset parser, tested; rendering UNTESTED). Bold is shown as brighter text with wider letters since egui has no bold font
- [x] Double-click a block title = select the nodes fully inside; dragging the title then moves them too. Rename a block via right-click > Edit text
