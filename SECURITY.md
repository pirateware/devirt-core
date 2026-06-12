# Security Policy

## Reporting a vulnerability

Please report security issues **privately**, not in public issues or PRs.

- Preferred: GitHub's private vulnerability reporting — the **Security** tab →
  **Report a vulnerability** on this repository.
- Alternatively, email **vasie1337@gmail.com**.

Please include a minimal reproducing input and what you observed. We'll
acknowledge the report and keep you updated on a fix and disclosure timeline.

## Supported versions

This is a pre-1.0 project under active development. Only the latest release and
`main` receive security fixes.

## Threat model

devirt-core deobfuscates **untrusted, often adversarial** JavaScript: its whole
job is to ingest hostile input. The relevant boundary is the *host running
devirt-core*, not the analyzed program. In scope:

- **Sandbox escape.** To lift string decoders, a [Boa](https://github.com/boa-dev/boa)
  sandbox (`crates/core/src/sandbox.rs`) evaluates attacker-controlled JavaScript.
  Any way for that evaluated code to reach the host (filesystem, network, process,
  arbitrary code execution) is a vulnerability.
- **Host crash escaping containment.** The pipeline runs on a worker thread with a
  large explicit stack and catches panics, falling back to a clean reformat (see
  the README's *Robustness* section). An input that crashes or aborts the *host*
  process despite these guards is in scope.
- **Unbounded resource use** that escapes the existing depth/time guards (e.g. a
  hang the sandbox interrupt doesn't break, or memory blow-up) such that a single
  input can take down the host.

Out of scope:

- The deobfuscator producing imperfect or incomplete output on some input. That's a
  correctness bug — open a normal issue with a repro.
- Behavior of the malicious sample you are analyzing. Always run analysis of real
  malware in an isolated environment; devirt-core analyzes such input but does not
  promise to neutralize it.
