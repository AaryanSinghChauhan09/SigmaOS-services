# SigmaOS-services

Component boundary for system services covering networking, audio, Bluetooth,
power, authentication, storage, and backup. The integration repository is
SigmaOS, as recorded in `COMPONENT.toml`.

## Current state

`COMPONENT.toml` marks this repository as a scaffold. The checkout currently
contains a component manifest, license, README, and metadata validation
workflow; service implementations and stable interfaces are not present here.

## Intended boundary

Keep service contracts explicit and least-privilege. Each service should state
its owner, inputs, outputs, failure modes, permissions, and lifecycle. Separate
device mechanisms from policy and user interfaces so implementations can be
replaced without making services kernel dependencies. The current list of
service areas is a planned scope, not evidence that those services exist.

## Readiness evidence

Before reporting a service as implemented, add its contract and source, focused
checks for normal and failure paths, and an integration example. The current
metadata workflow validates component declarations only and does not validate
service behavior.
