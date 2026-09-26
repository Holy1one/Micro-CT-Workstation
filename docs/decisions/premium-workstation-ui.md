# Premium Workstation UI

## Direction

The interface keeps its fixed industrial-console structure and makes its hierarchy quieter and clearer: one compact title row, three aligned control columns, and a shared baseline for the event log and operation summary. The 1920 × 1080 design canvas remains the sizing source. The workspace uses a consistent 12 px inset and 12 px panel gaps so the top, bottom, and side edges read as one frame.

Information architecture, hierarchy, readability, and spatial depth lead the styling decisions. Dense device readings, inputs, the 3D view, operation summary, and logs remain solid or softly tinted content surfaces. Translucency is reserved for functional chrome: the custom title bar, the floating action island, menu popovers, and scene status controls. The island uses the page's panel, neutral, text, line, and accent tokens: a pale frosted surface in the light theme and a blue-grey panel surface in the dark theme. A fine edge highlight and restrained shadow separate it from the scene without introducing a second neutral family.

One cool neutral family carries the chassis and panels. Deep teal-blue is the primary accent; red, amber, and green retain their existing safety and telemetry meanings. The existing local dock SVGs remain the control icon family. Window actions use simple CSS geometry, not a second downloaded icon set.

## Depth and interaction scales

- Four depth levels: application frame, content panel, inset field, and transient chrome. Fine dividers carry most of the separation; panels do not each receive a floating shadow.
- Radius tokens are compact for panels and controls (6–12 px). The island keeps its capsule silhouette; its four action keys use a 12 px radius.
- Shadows are limited to a short panel edge, a small floating-popover shadow, and the island's clearer lift. No ambient glows are added to ordinary cards.
- Hover and focus feedback use 120–180 ms transitions. Control surfaces use 180–260 ms. The island opens over 340 ms with a center-origin horizontal expansion; its approach ripple repeats only while approaching and signals an available interaction.
- Reduced motion opens the island immediately, removes the ripple, and keeps keyboard and pointer operation available. Escape collapses it.
- Disabled controls retain readable labels. Status words and measured values do not rely on color alone.

## Island and window chrome

The compact island is anchored 18 px from the top edge of the 3D scene. The compact state shows the central red status ring, a short state word, and a chevron; the expanded state stays at the same top anchor. The 5-position row has Home and Start/Pause/Resume to the left, a reserved center cell under the red ring, and Restore and Stop to the right. Its 558 px expanded width fits between the optical-axis heading and the right-side scene status chips. The center cell and equal grid tracks keep both sides symmetric while the island grows from the center and leave the camera's upper equipment margin visible. The page-token glass treatment changes with the light/dark theme, while the red ring retains its fault/status meaning. Actions still use `ws.dock`; invalid local setup drafts continue to block Home, Play, and Restore while Stop only follows the engine's Stop availability and stale-state gate.

The drawn title bar occupies a 39 px row. It carries the compact logo, `Micro-CT Workstation` name, muted `v0.7.0` version, four existing menus, and minimize/maximize/close controls. Brand and version use the same 15 px type size; a 4 px gap keeps the version attached to the name without a divider. The menu labels use 14 px type and 36 px hit areas, and the window controls fill the taller row. Tauri decorations are disabled. Tauri's drag region and window API provide movement and window actions; double-clicking unused title-bar space toggles maximize. `requires_maximized_window`, `schedule_dynamic_min_size`, `SetWindowSubclass`, `SC_MOVE` / `SC_SIZE` / `SC_RESTORE`, and `set_maximizable(!maximize_only)` remain the monitor and DPI policy. A display that is maximize-only cannot restore into an unreadable window.

## Height alignment and honest unknowns

At the 1920 × 1080 design viewport, the Operation Status panel and bottom log grid row are both 264 px high. The 39 px title row uses 13 px more vertical space than the former 26 px row; the flexible upper content row absorbs that reduction, while the 264 px log/Operation Status alignment and 30 px status bar remain fixed. At the 1920 × 1080 viewport, the full design canvas still meets all four client-area edges.

An unknown USB Auto Shut Down readback remains unknown. The X-ray safety block names the unresolved operator choice and explains that deadman arming cannot yet be verified. This is presentation only and does not manufacture confirmation.

## Anti-pattern review

- Glass is not applied to every card or data surface.
- Gradients, background decoration, and ambient glows are absent.
- The neutral hierarchy remains distinct: panel title bars, dense content, inset wells, and transient chrome each have a clear role.
- Text contrast and disabled-state labels remain readable in light and dark themes.
- Local dock SVGs are reused; the drawn window glyphs stay within the same monochrome chrome language.
- Motion follows pointer approach, hover, expansion, focus, or committed state. Reduced-motion users get the same controls immediately without ripple or unfolding.
- The 39 px menu row remains in the grid; the canvas continues to fill all four client edges under uniform scaling.
