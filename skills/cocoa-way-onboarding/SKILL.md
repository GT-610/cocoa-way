---
name: cocoa-way-onboarding
description: Guide a new Cocoa-Way user through choosing an image source, creating an Apple Container application profile, or saving a classic SSH/local Waypipe connection. Use for first-run setup and configuration questions.
---

# Cocoa-Way Onboarding

Keep setup local, reviewable, and reversible.

1. Call `cocoa_way_onboarding` with the user's goal.
2. For Apple Container, call `cocoa_way_image_sources`, then `cocoa_way_application_template` with the chosen image and command.
3. Explain that a base OCI image is not automatically GUI-ready. It needs Waypipe and the requested GUI command; clipboard and audio need the Cocoa-Way image helpers.
4. Ask the user to review the generated fields in Applications > New Application, run Check, and launch explicitly.
5. Use `desktop` for nested compositors such as niri or Hyprland. Use `rootless` for ordinary xdg-shell apps such as Foot or Firefox.
6. For SSH or an existing local Waypipe socket, call `cocoa_way_connection_template`. Save the reviewed fields through Connections > Connect to Machine or run the generated `run_waypipe.sh` command.
7. Create or select a managed Display before connecting when the default display is occupied.
8. If setup fails, switch to the `cocoa-way-diagnose` workflow and collect evidence before changing configuration.

## Safety

- Never choose an untrusted registry or tag on the user's behalf.
- Never store passwords, registry credentials, or private keys in Cocoa-Way configuration.
- MCP setup tools are read-only. Do not bypass explicit Check, Launch, Stop, or deletion controls.
- Do not force software rendering to hide a guest-image or transport problem.
