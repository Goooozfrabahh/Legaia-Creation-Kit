# LCK development and testing

- Use repository, shell, compiler, debugger and Git tools for development.
- Never invoke local desktop computer-control tools, move the user's physical
  cursor, or take control of the visible Windows screen.
- The user personally launches LCK and performs manual GUI and DuckStation
  gameplay verification. Prepare precise scenarios and analyze their supplied
  screenshots, logs and results. Never launch host LCK or DuckStation.
- QEMU and VM provisioning is paused indefinitely. Do not resume downloads,
  install VirtualBox, bypass Windows requirements, or create a sandbox without
  new explicit user authorization. Continue development without waiting for it.
- Never launch or control DuckStation on the physical Windows computer, through
  any GUI, command, script, test runner or subprocess.
- Preserve the untouched retail disc. Test outputs must use new paths and may
  never replace that source image or earlier successful artifacts.
- Distinguish automated verification, user-reported manual GUI verification,
  user-reported native gameplay verification and not verified. Headless/simulated
  input tests are automated verification. Preserve historical sandbox evidence
  separately if relevant; none has been established for M1.1.
- Preserve recoverable checkpoints and working editor/native-export safeguards.
