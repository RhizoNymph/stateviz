# Cascade

A design-time visualizer for systems of communicating state machines. One YAML
definition file generates every view: causal flow, structure, trace and
coupling matrix. Static checks catch cascade cycles, unhandled events, dead
commands and unreachable states.

Start a new design from scratch with `cascade new my-system.yaml`: it
creates a blank definition and opens the app in Build mode. Open an existing
one with `cascade open <file>`.

See [docs/spec.md](docs/spec.md) for the product spec and
[docs/OVERVIEW.md](docs/OVERVIEW.md) for the codebase map.
