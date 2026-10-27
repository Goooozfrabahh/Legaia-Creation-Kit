# LCK development and testing

- Use repository, shell, compiler, debugger and Git tools for development.
- Never invoke local desktop computer-control tools, move the user's physical
  cursor, or take control of the visible Windows screen.
- Interactive application testing must use an established isolated graphical
  sandbox. If unavailable, report GUI testing as not verified; do not fall back
  to the physical desktop or launch a local GUI to simulate a sandbox.
- Never launch or control DuckStation on the physical Windows computer, through
  any GUI, command, script, test runner or subprocess. Sandbox gameplay testing
  is permitted only if genuine isolation is established.
- Preserve the untouched retail disc. Test outputs must use new paths and may
  never replace that source image or earlier successful artifacts.
- Distinguish sandbox GUI verification, automated verification, native gameplay
  verification and not verified. Headless/simulated input tests are automated
  verification, not sandbox GUI or gameplay evidence.
- Preserve recoverable checkpoints and working editor/native-export safeguards.
